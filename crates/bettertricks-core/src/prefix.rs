use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use glob::glob;
use parking_lot::RwLock;
use regex::Regex;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{BettertricksError, PrefixArchitecture, PrefixSource, Result, Store, WinePrefix};

const PREFIX_NAMESPACE: Uuid = Uuid::from_u128(0xe28d612d_05a9_467a_97ad_9d46c8303467);

#[async_trait]
pub trait PrefixProvider: Send + Sync {
    fn source(&self) -> PrefixSource;
    async fn discover(&self) -> Result<Vec<PrefixCandidate>>;
}

#[derive(Debug, Clone)]
pub struct PrefixCandidate {
    pub path: PathBuf,
    pub name: Option<String>,
    pub source: PrefixSource,
    pub runtime: Option<PathBuf>,
    pub runtime_label: Option<String>,
}

pub struct PrefixDiscovery {
    store: Arc<Store>,
    providers: Vec<Box<dyn PrefixProvider>>,
    cached_prefixes: RwLock<HashMap<Uuid, WinePrefix>>,
}

impl PrefixDiscovery {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            providers: vec![
                Box::new(StandardWineProvider),
                Box::new(SteamProvider),
                Box::new(LutrisProvider),
                Box::new(BottlesProvider),
                Box::new(HeroicProvider),
            ],
            cached_prefixes: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_provider(mut self, provider: Box<dyn PrefixProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub async fn discover(&self) -> Result<Vec<WinePrefix>> {
        let mut candidates = Vec::new();
        for provider in &self.providers {
            match provider.discover().await {
                Ok(mut discovered) => candidates.append(&mut discovered),
                Err(error) => {
                    tracing::warn!(source = ?provider.source(), %error, "prefix provider failed")
                }
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

        let mut unique: HashMap<PathBuf, PrefixCandidate> = HashMap::new();
        for candidate in candidates {
            let normalized = normalize_path(&candidate.path);
            if let Some(existing) = unique.get(&normalized)
                && provider_priority(existing.source) >= provider_priority(candidate.source)
            {
                continue;
            }
            unique.insert(normalized, candidate);
        }

        let mut prefixes = Vec::new();
        for (_, candidate) in unique {
            prefixes.push(inspect_candidate(candidate).await?);
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
        *self.cached_prefixes.write() = prefixes
            .iter()
            .cloned()
            .map(|prefix| (prefix.id, prefix))
            .collect();
        Ok(prefixes)
    }

    /// Returns the latest discovered snapshot when available. Catalog filtering calls this on
    /// every query and does not need to rescan every launcher directory to rediscover a prefix
    /// that bootstrap already inspected. A cold cache still performs a full discovery.
    pub async fn by_id_cached(&self, id: Uuid) -> Result<WinePrefix> {
        let cached = self.cached_prefixes.read().get(&id).cloned();
        if let Some(prefix) = cached {
            return Ok(prefix);
        }
        self.by_id(id).await
    }

    pub async fn by_id(&self, id: Uuid) -> Result<WinePrefix> {
        self.discover()
            .await?
            .into_iter()
            .find(|prefix| prefix.id == id)
            .ok_or_else(|| BettertricksError::PrefixNotFound(id.to_string()))
    }

    pub fn register_manual(&self, path: &Path, name: &str, runtime: Option<&Path>) -> Result<()> {
        let path = validate_existing_prefix_path(path)?;
        self.store
            .register_prefix(&path, name, PrefixSource::Manual, runtime)
    }

    pub fn unregister_manual(&self, path: &Path) -> Result<()> {
        self.store.unregister_prefix(path)
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
            let xdg_data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"));
            let root = xdg_data.join("wineprefixes");
            if let Ok(entries) = std::fs::read_dir(root) {
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

struct SteamProvider;

#[async_trait]
impl PrefixProvider for SteamProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Steam
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let Some(home) = home_directory() else {
            return Ok(Vec::new());
        };
        let roots = [
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        ];
        let library_path = Regex::new(r#""path"\s+"([^"]+)""#).expect("valid expression");
        let mut libraries = HashSet::new();
        for root in roots {
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

        let mut prefixes = Vec::new();
        for library in libraries {
            let app_names = steam_app_names(&library);
            let compatdata = library.join("steamapps/compatdata");
            let Ok(entries) = std::fs::read_dir(compatdata) else {
                continue;
            };
            for entry in entries.flatten() {
                let app_id = entry.file_name().to_string_lossy().to_string();
                let prefix = entry.path().join("pfx");
                if prefix.join("drive_c").is_dir() {
                    prefixes.push(PrefixCandidate {
                        path: prefix,
                        name: Some(
                            app_names
                                .get(&app_id)
                                .cloned()
                                .unwrap_or_else(|| format!("Steam {app_id}")),
                        ),
                        source: PrefixSource::Steam,
                        runtime: None,
                        runtime_label: Some("Steam Proton".into()),
                    });
                }
            }
        }
        Ok(prefixes)
    }
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

#[async_trait]
impl PrefixProvider for BottlesProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Bottles
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let Some(home) = home_directory() else {
            return Ok(Vec::new());
        };
        let roots = [
            home.join(".local/share/bottles/bottles"),
            home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
        ];
        Ok(discover_children(&roots, PrefixSource::Bottles, "Bottles"))
    }
}

struct LutrisProvider;

#[async_trait]
impl PrefixProvider for LutrisProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Lutris
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let Some(home) = home_directory() else {
            return Ok(Vec::new());
        };
        let pattern = home.join(".config/lutris/games/*.yml");
        let mut prefixes = Vec::new();
        let expression =
            Regex::new(r#"(?m)^\s*prefix:\s*['"]?([^'"\n]+)"#).expect("valid expression");
        for entry in glob(pattern.to_string_lossy().as_ref())
            .into_iter()
            .flatten()
            .flatten()
        {
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
            prefixes.push(PrefixCandidate {
                path,
                name: Some(name.replace('-', " ")),
                source: PrefixSource::Lutris,
                runtime: None,
                runtime_label: Some("Lutris runner".into()),
            });
        }
        Ok(prefixes)
    }
}

struct HeroicProvider;

#[async_trait]
impl PrefixProvider for HeroicProvider {
    fn source(&self) -> PrefixSource {
        PrefixSource::Heroic
    }

    async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
        let Some(home) = home_directory() else {
            return Ok(Vec::new());
        };
        let roots = [
            home.join("Games/Heroic/Prefixes"),
            home.join(".config/heroic/Prefixes"),
            home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/Prefixes"),
        ];
        Ok(discover_children(&roots, PrefixSource::Heroic, "Heroic"))
    }
}

fn discover_children(
    roots: &[PathBuf],
    source: PrefixSource,
    runtime_label: &str,
) -> Vec<PrefixCandidate> {
    let mut prefixes = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("drive_c").is_dir() {
                prefixes.push(PrefixCandidate {
                    path,
                    name: entry.file_name().to_str().map(str::to_owned),
                    source,
                    runtime: None,
                    runtime_label: Some(runtime_label.into()),
                });
            }
        }
    }
    prefixes
}

async fn inspect_candidate(candidate: PrefixCandidate) -> Result<WinePrefix> {
    let path = normalize_path(&candidate.path);
    let exists = path.join("drive_c").is_dir() && path.join("system.reg").is_file();
    let architecture = if exists {
        detect_architecture(&path)
    } else {
        PrefixArchitecture::Unknown
    };
    let installed_verbs = read_installed_verbs(&path);
    let name = candidate.name.unwrap_or_else(|| {
        path.file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("Wine prefix")
            .to_string()
    });
    let id = Uuid::new_v5(&PREFIX_NAMESPACE, path.to_string_lossy().as_bytes());
    let last_modified = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(system_time_to_utc);

    Ok(WinePrefix {
        id,
        name,
        path,
        source: candidate.source,
        architecture,
        runtime: candidate.runtime,
        runtime_label: candidate.runtime_label,
        managed: candidate.source.is_managed(),
        exists,
        installed_verbs,
        size_bytes: None,
        last_modified,
    })
}

fn detect_architecture(prefix: &Path) -> PrefixArchitecture {
    if let Ok(content) = std::fs::read_to_string(prefix.join("system.reg")) {
        if content
            .lines()
            .take(8)
            .any(|line| line.contains("#arch=win64"))
        {
            if prefix.join("drive_c/windows/syswow64").is_dir() {
                return PrefixArchitecture::Wow64;
            }
            return PrefixArchitecture::Win64;
        }
        if content
            .lines()
            .take(8)
            .any(|line| line.contains("#arch=win32"))
        {
            return PrefixArchitecture::Win32;
        }
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

    struct CountingProvider {
        path: PathBuf,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl PrefixProvider for CountingProvider {
        fn source(&self) -> PrefixSource {
            PrefixSource::Manual
        }

        async fn discover(&self) -> Result<Vec<PrefixCandidate>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![PrefixCandidate {
                path: self.path.clone(),
                name: Some("Cached prefix".into()),
                source: PrefixSource::Manual,
                runtime: None,
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
        let discovery = PrefixDiscovery {
            store: Arc::new(Store::open_in_memory().unwrap()),
            providers: vec![Box::new(CountingProvider {
                path: prefix,
                calls: calls.clone(),
            })],
            cached_prefixes: RwLock::new(HashMap::new()),
        };

        let prefixes = discovery.discover().await.unwrap();
        let cached = discovery.by_id_cached(prefixes[0].id).await.unwrap();

        assert_eq!(cached.name, "Cached prefix");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
