use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::{
    AppPaths, CacheStats, DependencyCheck, LegacyVerbHost, Result, RuntimeSource, SystemReport,
    WineRuntime,
};

#[derive(Clone)]
pub struct SystemInspector {
    paths: AppPaths,
}

impl SystemInspector {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    pub async fn inspect(&self) -> Result<SystemReport> {
        let dependencies = self.dependencies().await;
        let runtimes = self.runtimes().await;
        let ready = dependencies
            .iter()
            .filter(|item| item.required)
            .all(|item| item.available)
            && !runtimes.is_empty();
        Ok(SystemReport {
            ready,
            os: read_os_name(),
            architecture: std::env::consts::ARCH.into(),
            desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
            dependencies,
            runtimes,
            data_directory: self.paths.data.clone(),
            cache_directory: self.paths.winetricks_cache.clone(),
            state_directory: self.paths.state.clone(),
        })
    }

    pub fn cache_stats(&self) -> CacheStats {
        let mut file_count = 0;
        let mut size_bytes = 0;
        for entry in walkdir::WalkDir::new(&self.paths.winetricks_cache)
            .follow_links(false)
            .into_iter()
            .filter_map(std::result::Result::ok)
        {
            if let Ok(metadata) = entry.metadata()
                && metadata.is_file()
            {
                file_count += 1;
                size_bytes += metadata.len();
            }
        }
        CacheStats {
            path: self.paths.winetricks_cache.clone(),
            file_count,
            size_bytes,
        }
    }

    async fn dependencies(&self) -> Vec<DependencyCheck> {
        let definitions = [
            ("wine", "Wine", true, "Install Wine from your distribution"),
            (
                "wineserver",
                "Wine server",
                true,
                "Install the complete Wine package",
            ),
            (
                "cabextract",
                "Cabinet extraction",
                true,
                "Install cabextract",
            ),
            ("7z", "7-Zip extraction", true, "Install 7zip or p7zip"),
            ("unzip", "ZIP extraction", true, "Install unzip"),
            ("gzip", "Gzip extraction", true, "Install gzip"),
            ("xz", "XZ extraction", true, "Install xz"),
            (
                "unrar",
                "RAR extraction",
                false,
                "Install unrar or unrar-free",
            ),
            (
                "aria2c",
                "Parallel and torrent downloads",
                false,
                "Install aria2",
            ),
            ("tor", "Tor routing", false, "Install and start Tor"),
            (
                "btrfs",
                "Btrfs restore points",
                false,
                "Install btrfs-progs",
            ),
            ("tar", "Archive restore points", true, "Install tar"),
            ("zstd", "Compressed restore points", true, "Install zstd"),
            (
                "winetricks",
                "Winetricks compatibility host",
                false,
                "Install Bettertricks' checksum-verified compatibility host for tracked recipes and custom .verb files",
            ),
        ];
        let mut checks = Vec::new();
        let legacy_host = LegacyVerbHost::discover_with_paths(&self.paths);
        for (id, label, required, remediation) in definitions {
            let (path, resolution_error) = if id == "winetricks" {
                match legacy_host.binary_path() {
                    Ok(path) => (path, None),
                    Err(error) => (None, Some(error.to_string())),
                }
            } else {
                (find_command(id), None)
            };
            let version = match (&path, resolution_error) {
                (Some(path), _) => command_version(path).await,
                (None, Some(error)) => Some(format!("Integrity check failed: {error}")),
                (None, None) => None,
            };
            checks.push(DependencyCheck {
                id: id.into(),
                label: label.into(),
                required,
                available: path.is_some(),
                path,
                version,
                remediation: Some(remediation.into()),
            });
        }
        checks
    }

    async fn runtimes(&self) -> Vec<WineRuntime> {
        let mut binaries = Vec::new();
        if let Some(wine) = find_command("wine") {
            binaries.push((wine, RuntimeSource::System, "System Wine".to_string()));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            collect_runtime_glob(
                &home.join(".local/share/Steam/steamapps/common/Proton */files/bin/wine"),
                RuntimeSource::Steam,
                "Steam Proton",
                &mut binaries,
            );
            collect_runtime_glob(
                &home.join(".local/share/lutris/runners/wine/*/bin/wine"),
                RuntimeSource::Lutris,
                "Lutris",
                &mut binaries,
            );
            collect_runtime_glob(
                &home.join(".local/share/bottles/runners/*/bin/wine"),
                RuntimeSource::Bottles,
                "Bottles",
                &mut binaries,
            );
        }

        let mut seen = HashSet::new();
        let mut runtimes = Vec::new();
        for (binary, source, label) in binaries {
            let binary = binary.canonicalize().unwrap_or(binary);
            if !seen.insert(binary.clone()) {
                continue;
            }
            let version = command_version(&binary).await;
            let wineserver = binary
                .parent()
                .map(|parent| parent.join("wineserver"))
                .filter(|path| path.is_file());
            let id = hex::encode(sha2::Sha256::digest(binary.to_string_lossy().as_bytes()));
            runtimes.push(WineRuntime {
                id: id[..16].to_string(),
                label: version
                    .as_ref()
                    .map(|version| format!("{label} {version}"))
                    .unwrap_or(label),
                wine_binary: binary,
                wineserver_binary: wineserver,
                version,
                source,
            });
        }
        runtimes
    }
}

fn collect_runtime_glob(
    pattern: &Path,
    source: RuntimeSource,
    label: &str,
    output: &mut Vec<(PathBuf, RuntimeSource, String)>,
) {
    if let Ok(paths) = glob::glob(pattern.to_string_lossy().as_ref()) {
        for path in paths.flatten() {
            if path.is_file() {
                output.push((path, source, label.into()));
            }
        }
    }
}

pub fn find_command(command: &str) -> Option<PathBuf> {
    if command.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(command);
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(command))
            .find(|candidate| candidate.is_file())
    })
}

async fn command_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().await.ok()?;
    let value = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    let value = String::from_utf8_lossy(&value);
    value
        .lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}

fn read_os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| std::env::consts::OS.into())
}

use sha2::Digest;
