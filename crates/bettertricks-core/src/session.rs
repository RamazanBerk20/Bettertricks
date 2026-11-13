use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{
    AppPaths, BettertricksError, Catalog, CatalogManifest, CatalogVersion, LegacyVerbHost,
    OperationEngine, Planner, PrefixDiscovery, RecoveryManager, Result, Store, SystemInspector,
};

/// Everything the desktop app and the CLI share. Both open it the same way, so catalog
/// selection, crash reconciliation, the prefix lock table and the operation lifecycle cannot
/// drift apart between the two front ends.
pub struct Session {
    pub paths: AppPaths,
    pub store: Arc<Store>,
    pub catalog: Catalog,
    pub discovery: PrefixDiscovery,
    pub planner: Planner,
    pub engine: OperationEngine,
    pub recovery: RecoveryManager,
    pub inspector: SystemInspector,
    pub legacy_host: LegacyVerbHost,
}

impl Session {
    pub fn open(paths: AppPaths, bundled_catalog: &Path) -> Result<Self> {
        let store = Arc::new(Store::open(&paths)?);
        let catalog = open_catalog(&paths, &store, bundled_catalog)?;
        let engine = OperationEngine::new(catalog.clone(), paths.clone(), store.clone());
        let interrupted = engine.reconcile_interrupted()?;
        if interrupted > 0 {
            tracing::warn!(
                interrupted,
                "marked operations left running by a previous exit"
            );
        }
        Ok(Self {
            discovery: PrefixDiscovery::new(store.clone(), engine.locks().clone()),
            planner: Planner::with_paths(catalog.clone(), &paths),
            recovery: engine.recovery().clone(),
            inspector: SystemInspector::new(paths.clone(), catalog.clone()),
            legacy_host: LegacyVerbHost::for_catalog(&paths, &catalog),
            engine,
            catalog,
            store,
            paths,
        })
    }
}

/// Activates the newer of the bundled catalog and the last activated (downloaded) one, so an
/// app upgrade that ships a newer catalog wins over an older signed download, while a
/// downloaded fix newer than the bundle keeps winning. Falls back to the other candidate when
/// the preferred one no longer loads.
fn open_catalog(paths: &AppPaths, store: &Store, bundled: &Path) -> Result<Catalog> {
    let bundled_manifest = CatalogManifest::read(bundled).unwrap_or_else(|_| CatalogManifest {
        version: "bundled".into(),
        upstream_tag: "unknown".into(),
        compatibility_host: None,
    });
    let active = store.active_catalog_version()?;
    let pinned = store.active_catalog_pinned()?;
    let bundled_candidate = (bundled.to_path_buf(), bundled_manifest.clone(), None);
    let mut candidates: Vec<(PathBuf, CatalogManifest, Option<String>)> = vec![bundled_candidate];
    if let Some(active) = active.filter(|active| active.path != bundled) {
        let manifest = CatalogManifest {
            version: active.version.clone(),
            upstream_tag: active.upstream_tag.clone(),
            compatibility_host: None,
        };
        let candidate = (active.path.clone(), manifest, active.signature.clone());
        // A rollback pins the user's choice even when the bundled catalog is newer.
        if pinned
            || CatalogVersion::parse(&active.version)
                >= CatalogVersion::parse(&bundled_manifest.version)
        {
            candidates.insert(0, candidate);
        } else {
            candidates.push(candidate);
        }
    }
    let mut last_error = None;
    for (path, manifest, signature) in candidates {
        match Catalog::load(
            manifest.source(path.clone()),
            paths.winetricks_cache.clone(),
        ) {
            Ok(catalog) => {
                let summary = catalog.summary();
                store.activate_catalog_version(
                    &summary.version,
                    &summary.upstream_tag,
                    &path,
                    signature.as_deref(),
                )?;
                return Ok(catalog);
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "catalog candidate is invalid");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| BettertricksError::Catalog("no catalog available".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_catalog(root: &Path, tag: &str, version: &str, title: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("manifest.json"),
            format!(r#"{{"version":"{version}","upstreamTag":"{tag}"}}"#),
        )
        .unwrap();
        std::fs::write(
            root.join("probe.toml"),
            format!("schema=1\nid=\"probe\"\ncategory=\"settings\"\ntitle=\"{title}\"\n[source]\nupstream_tag=\"{tag}\"\nupstream_verb=\"probe\"\n"),
        )
        .unwrap();
    }

    #[test]
    fn a_newer_bundled_catalog_wins_after_an_app_upgrade() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path().join("app")).unwrap();
        let downloaded = paths.catalogs.join("winetricks-20260125");
        write_catalog(&downloaded, "20260125", "winetricks-20260125", "Downloaded");
        Store::open(&paths)
            .unwrap()
            .activate_catalog_version("winetricks-20260125", "20260125", &downloaded, Some("sig"))
            .unwrap();

        let bundled = temp.path().join("bundled");
        write_catalog(&bundled, "20260601", "winetricks-20260601", "Bundled");
        let session = Session::open(paths.clone(), &bundled).unwrap();
        assert_eq!(session.catalog.summary().version, "winetricks-20260601");
        drop(session);

        // A downloaded revision newer than the bundle keeps winning.
        let fix = paths.catalogs.join("winetricks-20260601-r2");
        write_catalog(&fix, "20260601", "winetricks-20260601-r2", "Fix");
        Store::open(&paths)
            .unwrap()
            .activate_catalog_version("winetricks-20260601-r2", "20260601", &fix, Some("sig"))
            .unwrap();
        let session = Session::open(paths.clone(), &bundled).unwrap();
        assert_eq!(session.catalog.get("probe").unwrap().title, "Fix");

        // A rollback to the older download is the user's choice and survives a restart,
        // although the bundled catalog is newer.
        session
            .store
            .activate_catalog_version("winetricks-20260125", "20260125", &downloaded, Some("sig"))
            .unwrap();
        session.store.pin_active_catalog().unwrap();
        drop(session);
        let session = Session::open(paths, &bundled).unwrap();
        assert_eq!(session.catalog.summary().version, "winetricks-20260125");
        assert!(session.store.active_catalog_pinned().unwrap());
    }
}
