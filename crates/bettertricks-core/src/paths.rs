use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::{BettertricksError, Result};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub winetricks_cache: PathBuf,
    pub compatibility_hosts: PathBuf,
    pub catalogs: PathBuf,
    pub backups: PathBuf,
    pub logs: PathBuf,
    pub database: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project = ProjectDirs::from("io", "Bettertricks", "Bettertricks")
            .ok_or_else(|| BettertricksError::Io(std::io::Error::other("no home directory")))?;

        let xdg_cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(|| project.cache_dir().to_path_buf());

        Self::from_directories(
            project.config_dir(),
            project.data_dir(),
            project.data_local_dir(),
            project.cache_dir(),
            &xdg_cache,
        )
    }

    pub fn isolated(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        Self::from_directories(
            &root.join("config"),
            &root.join("data"),
            &root.join("state"),
            &root.join("cache"),
            &root.join("cache"),
        )
    }

    fn from_directories(
        config: &Path,
        data: &Path,
        state: &Path,
        cache: &Path,
        xdg_cache: &Path,
    ) -> Result<Self> {
        let paths = Self {
            config: config.to_path_buf(),
            data: data.to_path_buf(),
            state: state.to_path_buf(),
            cache: cache.to_path_buf(),
            winetricks_cache: xdg_cache.join("winetricks"),
            compatibility_hosts: data.join("compatibility-hosts"),
            catalogs: data.join("catalogs"),
            backups: state.join("backups"),
            logs: state.join("logs"),
            database: state.join("bettertricks.sqlite3"),
        };
        paths.ensure()?;
        Ok(paths)
    }

    pub fn ensure(&self) -> Result<()> {
        for directory in [
            &self.config,
            &self.data,
            &self.state,
            &self.cache,
            &self.winetricks_cache,
            &self.compatibility_hosts,
            &self.catalogs,
            &self.backups,
            &self.logs,
        ] {
            std::fs::create_dir_all(directory)?;
        }
        Ok(())
    }
}
