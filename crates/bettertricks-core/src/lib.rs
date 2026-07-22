//! Native domain and execution engine for Bettertricks.

mod catalog;
mod catalog_update;
mod domain;
mod error;
mod legacy;
mod operation;
mod paths;
mod planner;
mod prefix;
mod recovery;
mod store;
mod system;

pub use catalog::{Catalog, CatalogSource};
pub use catalog_update::{CatalogIndex, CatalogRelease, CatalogUpdater, rollback_catalog};
pub use domain::*;
pub use error::{BettertricksError, Result};
pub use legacy::{
    LegacyVerbHost, LegacyVerbInfo, MANAGED_WINETRICKS_SHA256, MANAGED_WINETRICKS_TAG,
    MANAGED_WINETRICKS_URL, install_managed_compatibility_host,
};
pub use operation::{OperationEngine, OperationEventSink};
pub use paths::AppPaths;
pub use planner::Planner;
pub use prefix::{
    PrefixDiscovery, PrefixProvider, validate_existing_prefix_path, validate_new_prefix_path,
};
pub use recovery::{ClearRestorePointsSummary, RecoveryManager};
pub use store::Store;
pub use system::SystemInspector;
