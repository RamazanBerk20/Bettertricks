use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::BufRead;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use parking_lot::{Mutex, RwLock};
use regex::Regex;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::lock::PrefixLocks;
use crate::{
    BettertricksError, PrefixArchitecture, PrefixSource, PrefixStatus, Result, Store, WinePrefix,
};

const PREFIX_NAMESPACE: Uuid = Uuid::from_u128(0xe28d612d_05a9_467a_97ad_9d46c8303467);

/// The stable id of the prefix at `path` (shared by discovery, the CLI and the lock table).
pub fn prefix_id_for_path(path: &Path) -> Uuid {
    Uuid::new_v5(
        &PREFIX_NAMESPACE,
        normalize_path(path).to_string_lossy().as_bytes(),
    )
}

#[async_trait]
pub trait PrefixProvider: Send + Sync {
    fn source(&self) -> PrefixSource;
    async fn discover(&self) -> Result<Vec<PrefixCandidate>>;
    /// Files and directories whose modification times decide whether a previous scan is
    /// still valid. Empty means the provider is rescanned every time.
    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct PrefixCandidate {
    pub path: PathBuf,
    pub name: Option<String>,
    pub source: PrefixSource,
    pub runtime: Option<PathBuf>,
    pub runtime_label: Option<String>,
}

type Fingerprint = Vec<(PathBuf, Option<SystemTime>)>;

/// What reading a prefix directory tells us; cached until its key files change.
#[derive(Debug, Clone)]
struct Inspection {
    exists: bool,
    architecture: PrefixArchitecture,
    installed_verbs: Vec<String>,
    last_modified: Option<DateTime<Utc>>,
}

pub struct PrefixDiscovery {
    store: Arc<Store>,
    locks: PrefixLocks,
    providers: Vec<Box<dyn PrefixProvider>>,
    scans: Mutex<HashMap<usize, (Fingerprint, Vec<PrefixCandidate>)>>,
    inspections: Mutex<HashMap<PathBuf, (Fingerprint, Inspection)>>,
    cached_prefixes: RwLock<HashMap<Uuid, (PrefixCandidate, WinePrefix)>>,
}

impl PrefixDiscovery {
    pub fn new(store: Arc<Store>, locks: PrefixLocks) -> Self {
        Self {
            store,
            locks,
            providers: vec![
                Box::new(StandardWineProvider),
                Box::new(SteamProvider),
                Box::new(LutrisProvider),
                Box::new(BottlesProvider),
                Box::new(HeroicProvider),
            ],
            scans: Mutex::new(HashMap::new()),
            inspections: Mutex::new(HashMap::new()),
            cached_prefixes: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_provider(mut self, provider: Box<dyn PrefixProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    async fn provider_candidates(&self, index: usize) -> Result<Vec<PrefixCandidate>> {
        let provider = &self.providers[index];
        let watched = provider.watch_paths();
        let fingerprint = fingerprint(&watched);
        if !watched.is_empty()
            && let Some((cached, candidates)) = self.scans.lock().get(&index)
            && *cached == fingerprint
        {
            return Ok(candidates.clone());
        }
        let candidates = provider.discover().await?;
        if !watched.is_empty() {
            self.scans
                .lock()
                .insert(index, (fingerprint, candidates.clone()));
        }
        Ok(candidates)
    }

    pub async fn discover(&self) -> Result<Vec<WinePrefix>> {
        let mut candidates = Vec::new();
        for index in 0..self.providers.len() {
            match self.provider_candidates(index).await {
                Ok(mut discovered) => candidates.append(&mut discovered),
                Err(error) => tracing::warn!(
                    source = ?self.providers[index].source(),
                    %error,
                    "prefix provider failed"
                ),
            }
        }
        for (path, name, source, runtime) in self.store.registered_prefixes()? {
            candidates.push(PrefixCandidate {
                path,
                name: Some(name),
                source,
                runtime,
                runtime_label: Some("Custom runtime".into()),
            });
        }
        let custom_wine = self.store.settings()?.custom_wine_binary;

        // A launcher keeps owning its prefix (managed safeguards, detected runtime) even when
        // the user also registers the same directory manually.
        let mut launcher: HashMap<PathBuf, PrefixCandidate> = HashMap::new();
        let mut unique: HashMap<PathBuf, PrefixCandidate> = HashMap::new();
        for candidate in candidates {
            let normalized = normalize_path(&candidate.path);
            if candidate.source.is_managed() {
                launcher.insert(normalized.clone(), candidate.clone());
            }
            if let Some(existing) = unique.get(&normalized)
                && provider_priority(existing.source) >= provider_priority(candidate.source)
            {
                continue;
            }
            unique.insert(normalized, candidate);
        }

        let mut prefixes = Vec::new();
        let mut cache = HashMap::new();
        for (normalized, mut candidate) in unique {
            let owner = launcher.get(&normalized);
            if candidate.runtime.is_none()
                && let Some(owner) = owner
            {
                candidate.runtime = owner.runtime.clone();
                candidate.runtime_label = owner.runtime_label.clone();
            }
            if candidate.runtime.is_none() && owner.is_none() {
                candidate.runtime = custom_wine.clone();
            }
            let mut prefix = self.inspect_candidate(&candidate);
            prefix.managed = owner.is_some();
            cache.insert(prefix.id, (candidate, prefix.clone()));
            prefixes.push(prefix);
        }
        prefixes.sort_by(|left, right| {
            source_priority(left.source)
                .cmp(&source_priority(right.source))
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                })
        });
        *self.cached_prefixes.write() = cache;
        Ok(prefixes)
    }

    /// Returns the latest discovered snapshot when available. Catalog filtering calls this on
    /// every query and does not need to rescan every launcher directory to rediscover a prefix
    /// that bootstrap already inspected. A cold cache still performs a full discovery.
    pub async fn by_id_cached(&self, id: Uuid) -> Result<WinePrefix> {
        let cached = self
            .cached_prefixes
            .read()
            .get(&id)
            .map(|(_, prefix)| prefix.clone());
        if let Some(mut prefix) = cached {
            prefix.status = self.status(&prefix.path, prefix.id, prefix.exists);
            return Ok(prefix);
        }
        self.by_id(id).await
    }

    /// Fresh state of one prefix: re-inspects only that directory (cheap when unchanged)
    /// instead of rescanning every launcher; falls back to a full discovery when unknown.
    pub async fn by_id(&self, id: Uuid) -> Result<WinePrefix> {
        let cached = self.cached_prefixes.read().get(&id).cloned();
        if let Some((candidate, previous)) = cached {
            let mut prefix = self.inspect_candidate(&candidate);
            prefix.managed = previous.managed;
            self.cached_prefixes
                .write()
                .insert(id, (candidate, prefix.clone()));
            return Ok(prefix);
        }
        self.discover()
            .await?
            .into_iter()
            .find(|prefix| prefix.id == id)
            .ok_or_else(|| BettertricksError::PrefixNotFound(id.to_string()))
    }

    /// Whether `path` is a prefix this app knows (used to restrict what the UI may open).
    pub fn is_known_prefix_path(&self, path: &Path) -> bool {
        let path = normalize_path(path);
        self.cached_prefixes
            .read()
            .values()
            .any(|(_, prefix)| prefix.path == path)
    }

    pub fn register_manual(&self, path: &Path, name: &str, runtime: Option<&Path>) -> Result<()> {
        let path = validate_existing_prefix_path(path)?;
        self.store
            .register_prefix(&path, name, PrefixSource::Manual, runtime)
    }

    pub fn unregister_manual(&self, path: &Path) -> Result<()> {
        self.store.unregister_prefix(path)
    }

    fn status(&self, path: &Path, id: Uuid, exists: bool) -> PrefixStatus {
        if self.locks.is_held(id) {
            PrefixStatus::Busy
        } else if exists {
            PrefixStatus::Ready
        } else if path.exists() {
            PrefixStatus::Uninitialized
        } else {
            PrefixStatus::Missing
        }
    }

    fn inspect_candidate(&self, candidate: &PrefixCandidate) -> WinePrefix {
        let path = normalize_path(&candidate.path);
        let key = fingerprint(&[
            path.clone(),
            path.join("drive_c"),
            path.join("system.reg"),
            path.join("winetricks.log"),
            path.join("drive_c/windows/syswow64"),
        ]);
        let cached = self
            .inspections
            .lock()
            .get(&path)
            .filter(|(stamp, _)| *stamp == key)
            .map(|(_, inspection)| inspection.clone());
        let inspection = cached.unwrap_or_else(|| {
            let inspection = inspect_path(&path);
            self.inspections
                .lock()
                .insert(path.clone(), (key, inspection.clone()));
            inspection
        });
        let name = candidate.name.clone().unwrap_or_else(|| {
            path.file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("Wine prefix")
                .to_string()
        });
        let id = prefix_id_for_path(&path);
        WinePrefix {
            id,
            name,
            status: self.status(&path, id, inspection.exists),
            path,
            source: candidate.source,
            architecture: inspection.architecture,
            runtime: candidate.runtime.clone(),
            runtime_label: candidate.runtime_label.clone(),
            managed: candidate.source.is_managed(),
            exists: inspection.exists,
            installed_verbs: inspection.installed_verbs,
            size_bytes: None,
            last_modified: inspection.last_modified,
        }
    }
}

fn fingerprint(paths: &[PathBuf]) -> Fingerprint {
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok(),
            )
        })
        .collect()
}

fn inspect_path(path: &Path) -> Inspection {
    let exists = path.join("drive_c").is_dir() && path.join("system.reg").is_file();
    Inspection {
        exists,
        architecture: if exists {
            detect_architecture(path)
        } else {
            PrefixArchitecture::Unknown
        },
        installed_verbs: read_installed_verbs(path),
        last_modified: std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .map(system_time_to_utc),
    }
}

struct StandardWineProvider;

#[async_trait]
impl PrefixProvider for StandardWineProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::DefaultWine
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let mut prefixes = Vec::new();
        if let Some(path) = std::env::var_os("WINEPREFIX").map(PathBuf::from) {
            prefixes.push(PrefixCandidate {
                path,
                name: Some("Environment prefix".into()),
                source: PrefixSource::DefaultWine,
                runtime: None,
                runtime_label: Some("System Wine".into()),
            });
        }
        if let Some(home) = home_directory() {
            prefixes.push(PrefixCandidate {
                path: home.join(".wine"),
                name: Some("Default prefix".into()),
                source: PrefixSource::DefaultWine,
                runtime: None,
                runtime_label: Some("System Wine".into()),
            });
            if let Ok(entries) = std::fs::read_dir(wineprefixes_root(&home)) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        prefixes.push(PrefixCandidate {
                            name: entry.file_name().to_str().map(str::to_owned),
                            path,
                            source: PrefixSource::WinePrefixes,
                            runtime: None,
                            runtime_label: Some("System Wine".into()),
                        });
                    }
                }
            }
        }
        Ok(prefixes)
    }
}

fn wineprefixes_root(home: &Path) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("wineprefixes")
}

struct SteamProvider;

impl SteamProvider {
    fn roots() -> Vec<PathBuf> {
        let Some(home) = home_directory() else {
            return Vec::new();
        };
        vec![
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        ]
    }

    fn libraries() -> HashSet<PathBuf> {
        let library_path = Regex::new(r#""path"\s+"([^"]+)""#).expect("valid expression");
        let mut libraries = HashSet::new();
        for root in Self::roots() {
            if !root.exists() {
                continue;
            }
            libraries.insert(root.clone());
            let vdf = root.join("steamapps/libraryfolders.vdf");
            if let Ok(content) = std::fs::read_to_string(vdf) {
                for captures in library_path.captures_iter(&content) {
                    if let Some(path) = captures.get(1) {
                        libraries.insert(PathBuf::from(path.as_str().replace("\\\\", "\\")));
                    }
                }
            }
        }
        libraries
    }
}

#[async_trait]
impl PrefixProvider for SteamProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Steam
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut paths = Self::roots();
        paths.extend(
            Self::roots()
                .iter()
                .map(|root| root.join("steamapps/libraryfolders.vdf")),
        );
        for library in Self::libraries() {
            let compatdata = library.join("steamapps/compatdata");
            paths.push(library.join("steamapps"));
            if let Ok(entries) = std::fs::read_dir(&compatdata) {
                paths.extend(
                    entries
                        .flatten()
                        .map(|entry| entry.path().join("config_info")),
                );
            }
            paths.push(compatdata);
        }
        paths
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let runtime_roots = steam_runtime_roots();
        let mut prefixes = Vec::new();
        for library in Self::libraries() {
            let app_names = steam_app_names(&library);
            let compatdata = library.join("steamapps/compatdata");
            let Ok(entries) = std::fs::read_dir(compatdata) else {
                continue;
            };
            for entry in entries.flatten() {
                let app_id = entry.file_name().to_string_lossy().to_string();
                let prefix = entry.path().join("pfx");
                if prefix.join("drive_c").is_dir() {
                    let (runtime, label) = proton_runtime(&entry.path(), &runtime_roots);
                    prefixes.push(PrefixCandidate {
                        path: prefix,
                        name: Some(
                            app_names
                                .get(&app_id)
                                .cloned()
                                .unwrap_or_else(|| format!("Steam {app_id}")),
                        ),
                        source: PrefixSource::Steam,
                        runtime,
                        runtime_label: Some(label.unwrap_or_else(|| "Steam Proton".into())),
                    });
                }
            }
        }
        Ok(prefixes)
    }
}

/// A runner name taken from another program's configuration: exactly one normal path
/// component, so it can never point outside the runners directory it is joined to.
fn single_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Accepts a launcher-supplied Wine binary only when it is absolute and, after resolving
/// symlinks, a regular file inside one of the launcher's own runtime directories. Anything
/// else leaves the runtime unknown (the planner then blocks the change).
fn trusted_binary(candidate: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    if !candidate.is_absolute() {
        return None;
    }
    let resolved = candidate.canonicalize().ok()?;
    let inside = roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| resolved.starts_with(root));
    (inside && resolved.is_file()).then(|| candidate.to_path_buf())
}

/// Where Steam keeps Proton builds: each library's `steamapps/common` and the
/// `compatibilitytools.d` directories for custom builds.
fn steam_runtime_roots() -> Vec<PathBuf> {
    let mut roots = SteamProvider::libraries()
        .into_iter()
        .map(|library| library.join("steamapps/common"))
        .collect::<Vec<_>>();
    roots.extend(
        SteamProvider::roots()
            .into_iter()
            .map(|root| root.join("compatibilitytools.d")),
    );
    roots.push(PathBuf::from("/usr/share/steam/compatibilitytools.d"));
    roots
}

/// The Proton build that last ran a compatdata prefix: Proton records its version on the
/// first line of `config_info` and its own install paths (`…/files/share/…`) below it.
fn proton_runtime(compatdata: &Path, roots: &[PathBuf]) -> (Option<PathBuf>, Option<String>) {
    let Ok(content) = std::fs::read_to_string(compatdata.join("config_info")) else {
        return (None, None);
    };
    let label = content
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|version| format!("Proton {version}"));
    let runtime = content.lines().skip(1).find_map(|line| {
        ["/files/", "/dist/"].iter().find_map(|marker| {
            let index = line.find(marker)?;
            trusted_binary(
                &Path::new(&line[..index + marker.len()]).join("bin/wine"),
                roots,
            )
        })
    });
    (runtime, label)
}

fn steam_app_names(library: &Path) -> HashMap<String, String> {
    let Ok(entries) = std::fs::read_dir(library.join("steamapps")) else {
        return HashMap::new();
    };

    let mut names = HashMap::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(app_id) = file_name
            .strip_prefix("appmanifest_")
            .and_then(|value| value.strip_suffix(".acf"))
        else {
            continue;
        };
        if app_id.is_empty() || !app_id.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(name) = vdf_string_value(&content, "name") else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() {
            names.insert(app_id.to_owned(), name.to_owned());
        }
    }
    names
}

fn vdf_string_value(content: &str, wanted_key: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let values = vdf_quoted_strings(line);
        (values.len() >= 2 && values[0].eq_ignore_ascii_case(wanted_key)).then(|| values[1].clone())
    })
}

fn vdf_quoted_strings(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut escaped = false;

    for character in line.chars() {
        if !quoted {
            if character == '"' {
                quoted = true;
                value.clear();
            }
            continue;
        }

        if escaped {
            match character {
                '"' | '\\' => value.push(character),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                other => {
                    value.push('\\');
                    value.push(other);
                }
            }
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            values.push(std::mem::take(&mut value));
            quoted = false;
        } else {
            value.push(character);
        }
    }

    values
}

struct BottlesProvider;

impl BottlesProvider {
    fn roots() -> Vec<PathBuf> {
        let Some(home) = home_directory() else {
            return Vec::new();
        };
        vec![
            home.join(".local/share/bottles/bottles"),
            home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
        ]
    }
}

#[async_trait]
impl PrefixProvider for BottlesProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Bottles
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        children_watch_paths(&Self::roots(), "bottle.yml")
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let runner =
            Regex::new(r#"(?m)^Runner:\s*['"]?([^'"\n]+?)['"]?\s*$"#).expect("valid expression");
        let mut prefixes = Vec::new();
        for root in Self::roots() {
            let runners = root.parent().map(|data| data.join("runners"));
            for (path, name) in prefix_children(&root) {
                let (runtime, label) = std::fs::read_to_string(path.join("bottle.yml"))
                    .ok()
                    .and_then(|content| {
                        let name = runner
                            .captures(&content)?
                            .get(1)?
                            .as_str()
                            .trim()
                            .to_owned();
                        let runners = runners.as_ref()?;
                        let wine = single_component(&name)
                            .then(|| {
                                trusted_binary(
                                    &runners.join(&name).join("bin/wine"),
                                    std::slice::from_ref(runners),
                                )
                            })
                            .flatten();
                        Some((wine, Some(name)))
                    })
                    .unwrap_or((None, None));
                prefixes.push(PrefixCandidate {
                    path,
                    name,
                    source: PrefixSource::Bottles,
                    runtime,
                    runtime_label: Some(label.unwrap_or_else(|| "Bottles".into())),
                });
            }
        }
        Ok(prefixes)
    }
}

struct LutrisProvider;

impl LutrisProvider {
    fn games() -> Vec<PathBuf> {
        let Some(home) = home_directory() else {
            return Vec::new();
        };
        let pattern = home.join(".config/lutris/games/*.yml");
        glob(pattern.to_string_lossy().as_ref())
            .into_iter()
            .flatten()
            .flatten()
            .collect()
    }
}

#[async_trait]
impl PrefixProvider for LutrisProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Lutris
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut paths = Self::games();
        if let Some(home) = home_directory() {
            paths.push(home.join(".config/lutris/games"));
        }
        paths
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let Some(home) = home_directory() else {
            return Ok(Vec::new());
        };
        let mut prefixes = Vec::new();
        let expression =
            Regex::new(r#"(?m)^\s*prefix:\s*['"]?([^'"\n]+)"#).expect("valid expression");
        for entry in Self::games() {
            let Ok(content) = std::fs::read_to_string(&entry) else {
                continue;
            };
            let Some(capture) = expression
                .captures(&content)
                .and_then(|captures| captures.get(1))
            else {
                continue;
            };
            let path = expand_home(capture.as_str().trim());
            let name = entry
                .file_stem()
                .and_then(OsStr::to_str)
                .unwrap_or("Lutris prefix");
            let version = lutris_wine_version(&content);
            let runners = home.join(".local/share/lutris/runners/wine");
            let runtime = version.as_ref().and_then(|version| {
                trusted_binary(
                    &runners.join(version).join("bin/wine"),
                    std::slice::from_ref(&runners),
                )
            });
            prefixes.push(PrefixCandidate {
                path,
                name: Some(name.replace('-', " ")),
                source: PrefixSource::Lutris,
                runtime,
                runtime_label: Some(version.unwrap_or_else(|| "Lutris runner".into())),
            });
        }
        Ok(prefixes)
    }
}

/// `version:` inside the top-level `wine:` block of a Lutris game configuration.
fn lutris_wine_version(content: &str) -> Option<String> {
    let mut in_wine = false;
    for line in content.lines() {
        if !line.starts_with([' ', '\t']) {
            in_wine = line.trim_end() == "wine:";
            continue;
        }
        if in_wine && let Some(value) = line.trim().strip_prefix("version:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if single_component(value) {
                return Some(value.to_owned());
            }
        }
    }
    None
}

struct HeroicProvider;

impl HeroicProvider {
    fn config_roots() -> Vec<PathBuf> {
        let Some(home) = home_directory() else {
            return Vec::new();
        };
        vec![
            home.join(".config/heroic"),
            home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
        ]
    }

    fn prefix_roots() -> Vec<PathBuf> {
        let mut roots = home_directory()
            .map(|home| vec![home.join("Games/Heroic/Prefixes")])
            .unwrap_or_default();
        roots.extend(
            Self::config_roots()
                .iter()
                .map(|root| root.join("Prefixes")),
        );
        roots
    }

    /// `GamesConfig/<app>.json` → `{ "<app>": { "winePrefix": …, "wineVersion": { "bin", "type" } } }`.
    fn runtimes() -> HashMap<PathBuf, (PathBuf, Option<String>)> {
        // Heroic's own downloaded builds, Steam's Proton builds, or the system Wine.
        let mut roots = Self::config_roots()
            .iter()
            .map(|root| root.join("tools"))
            .collect::<Vec<_>>();
        roots.extend(steam_runtime_roots());
        let system_wine =
            crate::system::find_command("wine").and_then(|wine| wine.canonicalize().ok());
        let mut runtimes = HashMap::new();
        for root in Self::config_roots() {
            let Ok(entries) = std::fs::read_dir(root.join("GamesConfig")) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(value) = std::fs::read(entry.path())
                    .map_err(|_| ())
                    .and_then(|bytes| {
                        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| ())
                    })
                else {
                    continue;
                };
                let Some(games) = value.as_object() else {
                    continue;
                };
                for game in games.values() {
                    let (Some(prefix), Some(bin)) = (
                        game.get("winePrefix").and_then(serde_json::Value::as_str),
                        game.pointer("/wineVersion/bin")
                            .and_then(serde_json::Value::as_str),
                    ) else {
                        continue;
                    };
                    let bin = PathBuf::from(bin);
                    let wine = if game
                        .pointer("/wineVersion/type")
                        .and_then(serde_json::Value::as_str)
                        == Some("proton")
                    {
                        let directory = bin.parent().map(Path::to_path_buf).unwrap_or_default();
                        ["files/bin/wine", "dist/bin/wine"]
                            .iter()
                            .find_map(|relative| trusted_binary(&directory.join(relative), &roots))
                    } else if system_wine
                        .as_ref()
                        .is_some_and(|system| bin.canonicalize().ok().as_ref() == Some(system))
                    {
                        Some(bin)
                    } else {
                        trusted_binary(&bin, &roots)
                    };
                    if let Some(wine) = wine {
                        let label = game
                            .pointer("/wineVersion/name")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned);
                        runtimes.insert(normalize_path(&expand_home(prefix)), (wine, label));
                    }
                }
            }
        }
        runtimes
    }
}

#[async_trait]
impl PrefixProvider for HeroicProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Heroic
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut paths = children_watch_paths(&Self::prefix_roots(), "system.reg");
        for root in Self::config_roots() {
            let configs = root.join("GamesConfig");
            if let Ok(entries) = std::fs::read_dir(&configs) {
                paths.extend(entries.flatten().map(|entry| entry.path()));
            }
            paths.push(configs);
        }
        paths
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let runtimes = Self::runtimes();
        let mut prefixes = Vec::new();
        for root in Self::prefix_roots() {
            for (path, name) in prefix_children(&root) {
                let (runtime, label) = runtimes
                    .get(&normalize_path(&path))
                    .cloned()
                    .map(|(wine, label)| (Some(wine), label))
                    .unwrap_or((None, None));
                prefixes.push(PrefixCandidate {
                    path,
                    name,
                    source: PrefixSource::Heroic,
                    runtime,
                    runtime_label: Some(label.unwrap_or_else(|| "Heroic".into())),
                });
            }
        }
        Ok(prefixes)
    }
}

fn prefix_children(root: &Path) -> Vec<(PathBuf, Option<String>)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().join("drive_c").is_dir())
        .map(|entry| (entry.path(), entry.file_name().to_str().map(str::to_owned)))
        .collect()
}

fn children_watch_paths(roots: &[PathBuf], file: &str) -> Vec<PathBuf> {
    let mut paths = roots.to_vec();
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(root) {
            paths.extend(entries.flatten().map(|entry| entry.path().join(file)));
        }
    }
    paths
}

fn detect_architecture(prefix: &Path) -> PrefixArchitecture {
    // The arch marker is in the header; never read a multi-megabyte registry in full.
    let header = std::fs::File::open(prefix.join("system.reg"))
        .map(|file| {
            std::io::BufReader::new(file)
                .lines()
                .take(8)
                .map_while(std::result::Result::ok)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if header.iter().any(|line| line.contains("#arch=win64")) {
        if prefix.join("drive_c/windows/syswow64").is_dir() {
            return PrefixArchitecture::Wow64;
        }
        return PrefixArchitecture::Win64;
    }
    if header.iter().any(|line| line.contains("#arch=win32")) {
        return PrefixArchitecture::Win32;
    }
    if prefix.join("drive_c/windows/syswow64").is_dir() {
        PrefixArchitecture::Wow64
    } else if prefix.join("drive_c/windows/system32").is_dir() {
        PrefixArchitecture::Win32
    } else {
        PrefixArchitecture::Unknown
    }
}

fn read_installed_verbs(prefix: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(prefix.join("winetricks.log")) else {
        return Vec::new();
    };
    let mut verbs = Vec::new();
    let mut seen = HashSet::new();
    for verb in content.split_whitespace() {
        let verb = verb.trim().to_string();
        if !verb.is_empty() && seen.insert(verb.clone()) {
            verbs.push(verb);
        }
    }
    verbs
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    })
}

pub fn validate_existing_prefix_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(BettertricksError::Security(
            "Wine prefix paths must be absolute".into(),
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| BettertricksError::PrefixNotFound(path.display().to_string()))?;
    reject_broad_prefix_path(&canonical)?;
    if !canonical.join("drive_c").is_dir() || !canonical.join("system.reg").is_file() {
        return Err(BettertricksError::Conflict(format!(
            "{} is not an initialized Wine prefix",
            canonical.display()
        )));
    }
    Ok(canonical)
}

pub fn validate_new_prefix_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(BettertricksError::Security(
            "Wine prefix paths must be absolute".into(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(BettertricksError::Security(
            "Wine prefix paths cannot contain parent-directory segments".into(),
        ));
    }

    let mut cursor = path;
    let mut missing = Vec::new();
    while !cursor.exists() {
        let name = cursor.file_name().ok_or_else(|| {
            BettertricksError::Security("Wine prefix path has no directory name".into())
        })?;
        missing.push(name.to_os_string());
        cursor = cursor.parent().ok_or_else(|| {
            BettertricksError::Security("Wine prefix path has no existing parent".into())
        })?;
    }
    let mut resolved = cursor.canonicalize()?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    reject_broad_prefix_path(&resolved)?;
    Ok(resolved)
}

fn reject_broad_prefix_path(path: &Path) -> Result<()> {
    if path.parent().is_none() {
        return Err(BettertricksError::Security(
            "the filesystem root cannot be used as a Wine prefix".into(),
        ));
    }
    if let Some(home) = home_directory().and_then(|value| value.canonicalize().ok())
        && path == home
    {
        return Err(BettertricksError::Security(
            "the home directory cannot be used as a Wine prefix".into(),
        ));
    }
    Ok(())
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return home_directory().unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(suffix) = value.strip_prefix("~/")
        && let Some(home) = home_directory()
    {
        return home.join(suffix);
    }
    PathBuf::from(value)
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn provider_priority(source: PrefixSource) -> u8 {
    match source {
        PrefixSource::Manual => 10,
        PrefixSource::Steam
        | PrefixSource::Lutris
        | PrefixSource::Bottles
        | PrefixSource::Heroic => 8,
        PrefixSource::WinePrefixes => 5,
        PrefixSource::DefaultWine => 4,
    }
}

fn source_priority(source: PrefixSource) -> u8 {
    match source {
        PrefixSource::DefaultWine => 0,
        PrefixSource::WinePrefixes => 1,
        PrefixSource::Steam => 2,
        PrefixSource::Lutris => 3,
        PrefixSource::Bottles => 4,
        PrefixSource::Heroic => 5,
        PrefixSource::Manual => 6,
    }
}

fn system_time_to_utc(value: SystemTime) -> DateTime<Utc> {
    value.into()
}

pub fn directory_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::PrefixArchitecture;

    struct CountingProvider {
        path: PathBuf,
        calls: Arc<AtomicUsize>,
        watch: Vec<PathBuf>,
        source: PrefixSource,
    }

    #[async_trait]
    impl PrefixProvider for CountingProvider {
        fn source(&self) -> PrefixSource {
            self.source
        }

        fn watch_paths(&self) -> Vec<PathBuf> {
            self.watch.clone()
        }

        async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![PrefixCandidate {
                path: self.path.clone(),
                name: Some("Cached prefix".into()),
                source: self.source,
                runtime: self
                    .source
                    .is_managed()
                    .then(|| PathBuf::from("/opt/proton/files/bin/wine")),
                runtime_label: Some("Test runtime".into()),
            }])
        }
    }

    #[test]
    fn reads_log_once_in_order() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("winetricks.log"),
            "corefonts\nvcrun2022\ncorefonts\n",
        )
        .unwrap();
        assert_eq!(
            read_installed_verbs(temp.path()),
            ["corefonts", "vcrun2022"]
        );
    }

    #[test]
    fn reads_steam_game_names_from_local_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let steamapps = temp.path().join("steamapps");
        std::fs::create_dir(&steamapps).unwrap();
        std::fs::write(
            steamapps.join("appmanifest_1086940.acf"),
            r#""AppState"
{
    "appid"        "1086940"
    "name"         "Baldur's Gate 3"
}"#,
        )
        .unwrap();
        std::fs::write(steamapps.join("appmanifest_not-an-id.acf"), "ignored").unwrap();

        let names = steam_app_names(temp.path());

        assert_eq!(
            names.get("1086940").map(String::as_str),
            Some("Baldur's Gate 3")
        );
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn decodes_escaped_vdf_names() {
        let content = r#"    "name"    "Sid Meier's Civilization® VI \"DX12\"""#;

        assert_eq!(
            vdf_string_value(content, "name").as_deref(),
            Some("Sid Meier's Civilization® VI \"DX12\"")
        );
    }

    #[test]
    fn validates_only_initialized_specific_prefix_paths() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("game");
        std::fs::create_dir(&prefix).unwrap();
        assert!(validate_existing_prefix_path(&prefix).is_err());

        std::fs::create_dir(prefix.join("drive_c")).unwrap();
        std::fs::write(prefix.join("system.reg"), "REGEDIT4\n").unwrap();
        assert_eq!(
            validate_existing_prefix_path(&prefix).unwrap(),
            prefix.canonicalize().unwrap()
        );
        assert!(validate_existing_prefix_path(Path::new("relative-prefix")).is_err());
        assert!(validate_existing_prefix_path(Path::new("/")).is_err());
    }

    #[test]
    fn resolves_new_prefix_targets_without_parent_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("nested/game");
        assert_eq!(validate_new_prefix_path(&target).unwrap(), target);
        assert!(validate_new_prefix_path(&temp.path().join("nested/../escape")).is_err());
        assert!(validate_new_prefix_path(Path::new("relative-prefix")).is_err());
        assert!(validate_new_prefix_path(Path::new("/")).is_err());
    }

    #[tokio::test]
    async fn reuses_the_discovered_snapshot_for_catalog_lookups() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        std::fs::write(prefix.join("system.reg"), "REGEDIT4\n#arch=win64\n").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let discovery = discovery_with(
            temp.path(),
            CountingProvider {
                path: prefix,
                calls: calls.clone(),
                watch: Vec::new(),
                source: PrefixSource::Manual,
            },
        );

        let prefixes = discovery.discover().await.unwrap();
        let cached = discovery.by_id_cached(prefixes[0].id).await.unwrap();
        let fresh = discovery.by_id(prefixes[0].id).await.unwrap();

        assert_eq!(cached.name, "Cached prefix");
        assert_eq!(fresh.architecture, PrefixArchitecture::Win64);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn discovery_with(root: &Path, provider: CountingProvider) -> PrefixDiscovery {
        let paths = crate::AppPaths::isolated(root.join("app")).unwrap();
        let mut discovery = PrefixDiscovery::new(
            Arc::new(Store::open_in_memory().unwrap()),
            PrefixLocks::new(&paths),
        );
        discovery.providers = vec![Box::new(provider)];
        discovery
    }

    #[tokio::test]
    async fn launcher_scans_are_cached_until_a_watched_path_changes() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("compatdata");
        let prefix = library.join("1/pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        std::fs::write(prefix.join("system.reg"), "REGEDIT4\n#arch=win64\n").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let discovery = discovery_with(
            temp.path(),
            CountingProvider {
                path: prefix.clone(),
                calls: calls.clone(),
                watch: vec![library.clone()],
                source: PrefixSource::Steam,
            },
        );

        discovery.discover().await.unwrap();
        discovery.discover().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir(library.join("2")).unwrap();
        discovery.discover().await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn manual_registration_keeps_launcher_ownership_and_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        std::fs::write(prefix.join("system.reg"), "REGEDIT4\n").unwrap();
        let discovery = discovery_with(
            temp.path(),
            CountingProvider {
                path: prefix.clone(),
                calls: Arc::default(),
                watch: Vec::new(),
                source: PrefixSource::Steam,
            },
        );
        discovery.register_manual(&prefix, "Mine", None).unwrap();

        let found = discovery.discover().await.unwrap();

        assert_eq!(found.len(), 1);
        assert!(found[0].managed);
        assert_eq!(
            found[0].runtime.as_deref(),
            Some(Path::new("/opt/proton/files/bin/wine"))
        );
        assert_eq!(found[0].status, PrefixStatus::Ready);
    }

    #[tokio::test]
    async fn reports_busy_while_another_task_holds_the_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("pfx");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        std::fs::write(prefix.join("system.reg"), "REGEDIT4\n").unwrap();
        let discovery = discovery_with(
            temp.path(),
            CountingProvider {
                path: prefix.clone(),
                calls: Arc::default(),
                watch: Vec::new(),
                source: PrefixSource::Manual,
            },
        );
        let id = discovery.discover().await.unwrap()[0].id;
        let other = PrefixLocks::new(&crate::AppPaths::isolated(temp.path().join("app")).unwrap());
        let _guard = other.acquire(id, "test").unwrap();

        assert_eq!(
            discovery.by_id_cached(id).await.unwrap().status,
            PrefixStatus::Busy
        );
    }

    #[test]
    fn resolves_launcher_runtimes_from_their_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let proton = temp.path().join("Proton 9.0");
        std::fs::create_dir_all(proton.join("files/bin")).unwrap();
        std::fs::write(proton.join("files/bin/wine"), "").unwrap();
        let compatdata = temp.path().join("compatdata/42");
        std::fs::create_dir_all(&compatdata).unwrap();
        std::fs::write(
            compatdata.join("config_info"),
            format!("9.0-204\n{}/files/share/fonts/\n", proton.display()),
        )
        .unwrap();
        let (runtime, label) = proton_runtime(&compatdata, &[temp.path().to_path_buf()]);
        assert_eq!(runtime, Some(proton.join("files/bin/wine")));
        assert_eq!(label.as_deref(), Some("Proton 9.0-204"));
        // A config_info that points outside Steam's runtime directories is not trusted.
        let (runtime, _) = proton_runtime(&compatdata, &[temp.path().join("steamapps/common")]);
        assert_eq!(runtime, None);

        let runners = temp.path().join("runners");
        std::fs::create_dir_all(runners.join("soda/bin")).unwrap();
        std::fs::write(runners.join("soda/bin/wine"), "").unwrap();
        let evil = temp.path().join("evil/bin");
        std::fs::create_dir_all(&evil).unwrap();
        std::fs::write(evil.join("wine"), "").unwrap();
        let resolve = |name: &str| {
            single_component(name)
                .then(|| {
                    trusted_binary(
                        &runners.join(name).join("bin/wine"),
                        std::slice::from_ref(&runners),
                    )
                })
                .flatten()
        };
        assert!(resolve("soda").is_some());
        assert!(resolve(&temp.path().join("evil").display().to_string()).is_none());
        assert!(resolve("../evil").is_none());
        std::os::unix::fs::symlink(temp.path().join("evil"), runners.join("linked")).unwrap();
        assert!(resolve("linked").is_none());

        assert_eq!(
            lutris_wine_version("game:\n  exe: x.exe\nwine:\n  version: lutris-GE-Proton8-26-x86_64\nsystem:\n  version: other\n").as_deref(),
            Some("lutris-GE-Proton8-26-x86_64")
        );
        assert_eq!(lutris_wine_version("wine:\n  version: ../escape/x\n"), None);
    }
}
