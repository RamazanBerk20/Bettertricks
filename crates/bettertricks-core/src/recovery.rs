use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use tokio::process::Command;
use uuid::Uuid;

use crate::{
    AppPaths, BettertricksError, RestoreMethod, RestorePoint, Result, Store, WinePrefix,
    prefix::directory_size,
};

#[derive(Clone)]
pub struct RecoveryManager {
    paths: AppPaths,
    store: Arc<Store>,
}

impl RecoveryManager {
    pub fn new(paths: AppPaths, store: Arc<Store>) -> Self {
        Self { paths, store }
    }

    pub fn list(&self, prefix_id: Option<Uuid>) -> Result<Vec<RestorePoint>> {
        self.store.restore_points(prefix_id)
    }

    pub async fn create(
        &self,
        prefix: &WinePrefix,
        operation_id: Option<Uuid>,
    ) -> Result<RestorePoint> {
        let _recovery_guard = self.store.lock_recovery().await;
        if !prefix.path.is_dir() {
            return Err(BettertricksError::PrefixNotFound(
                prefix.path.display().to_string(),
            ));
        }
        let id = Uuid::new_v4();
        let root = self.paths.backups.join(prefix.id.to_string());
        tokio::fs::create_dir_all(&root).await?;

        let (storage_path, method) = if can_reflink(&prefix.path, &root).await {
            let path = root.join(id.to_string());
            let partial = root.join(format!(".{id}.partial"));
            remove_path(&partial).await;
            if let Err(error) = run_checked(
                Command::new("cp")
                    .arg("-a")
                    .arg("--reflink=always")
                    .arg(&prefix.path)
                    .arg(&partial),
                "cp",
            )
            .await
            {
                remove_path(&partial).await;
                return Err(error);
            }
            if let Err(error) = tokio::fs::rename(&partial, &path).await {
                remove_path(&partial).await;
                return Err(error.into());
            }
            (path, RestoreMethod::Reflink)
        } else {
            let path = root.join(format!("{id}.tar.zst"));
            let partial = root.join(format!(".{id}.tar.zst.partial"));
            let parent = prefix.path.parent().ok_or_else(|| {
                BettertricksError::Security(
                    "cannot archive a prefix without a parent directory".into(),
                )
            })?;
            let name = prefix.path.file_name().ok_or_else(|| {
                BettertricksError::Security("cannot archive an unnamed prefix".into())
            })?;
            remove_path(&partial).await;
            if let Err(error) = run_checked(
                Command::new("tar")
                    .arg("--zstd")
                    .arg("-cf")
                    .arg(&partial)
                    .arg("-C")
                    .arg(parent)
                    .arg(name),
                "tar",
            )
            .await
            {
                remove_path(&partial).await;
                return Err(error);
            }
            if let Err(error) = tokio::fs::rename(&partial, &path).await {
                remove_path(&partial).await;
                return Err(error.into());
            }
            (path, RestoreMethod::Archive)
        };

        let size_bytes = if storage_path.is_dir() {
            Some(directory_size(&storage_path))
        } else {
            std::fs::metadata(&storage_path)
                .ok()
                .map(|metadata| metadata.len())
        };
        let point = RestorePoint {
            id,
            prefix_id: prefix.id,
            prefix_name: prefix.name.clone(),
            prefix_path: prefix.path.clone(),
            storage_path,
            method,
            created_at: Utc::now(),
            size_bytes,
            operation_id,
        };
        if let Err(error) = self.store.add_restore_point(&point) {
            remove_path(&point.storage_path).await;
            return Err(error);
        }
        Ok(point)
    }

    pub async fn restore(&self, point: &RestorePoint) -> Result<()> {
        let _recovery_guard = self.store.lock_recovery().await;
        if !point.prefix_path.is_absolute() {
            return Err(BettertricksError::Security(
                "restore targets must be absolute paths".into(),
            ));
        }
        if point.prefix_path.exists() {
            return Err(BettertricksError::Conflict(format!(
                "move or trash {} before restoring",
                point.prefix_path.display()
            )));
        }
        let parent = point
            .prefix_path
            .parent()
            .ok_or_else(|| BettertricksError::Security("restore target has no parent".into()))?;
        let name = point
            .prefix_path
            .file_name()
            .ok_or_else(|| BettertricksError::Security("restore target has no file name".into()))?;
        let storage_path = self.validated_storage_path(point)?;
        tokio::fs::create_dir_all(parent).await?;
        match point.method {
            RestoreMethod::Reflink | RestoreMethod::Btrfs => {
                let staging = parent.join(format!(
                    ".{}.bettertricks-restore-{}.partial",
                    name.to_string_lossy(),
                    point.id
                ));
                remove_path(&staging).await;
                if let Err(error) = run_checked(
                    Command::new("cp")
                        .arg("-a")
                        .arg("--reflink=auto")
                        .arg(&storage_path)
                        .arg(&staging),
                    "cp",
                )
                .await
                {
                    remove_path(&staging).await;
                    return Err(error);
                }
                if let Err(error) = rename_noreplace(&staging, &point.prefix_path) {
                    remove_path(&staging).await;
                    return Err(error);
                }
                Ok(())
            }
            RestoreMethod::Archive => {
                let staging = parent.join(format!(".bettertricks-restore-{}.partial", point.id));
                remove_path(&staging).await;
                tokio::fs::create_dir(&staging).await?;
                let extraction = run_checked(
                    Command::new("tar")
                        .arg("--zstd")
                        .arg("-xf")
                        .arg(&storage_path)
                        .arg("-C")
                        .arg(&staging)
                        .arg("--no-same-owner")
                        .arg("--no-same-permissions"),
                    "tar",
                )
                .await;
                if let Err(error) = extraction {
                    remove_path(&staging).await;
                    return Err(error);
                }
                let restored = staging.join(name);
                let restored_is_directory = tokio::fs::symlink_metadata(&restored)
                    .await
                    .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
                if !restored_is_directory {
                    remove_path(&staging).await;
                    return Err(BettertricksError::Security(
                        "restore archive did not contain the expected prefix directory".into(),
                    ));
                }
                if let Err(error) = rename_noreplace(&restored, &point.prefix_path) {
                    remove_path(&staging).await;
                    return Err(error);
                }
                remove_path(&staging).await;
                Ok(())
            }
        }
    }

    /// Permanently removes committed restore points while preserving snapshots
    /// attached to operations that can still need them for recovery.
    pub async fn clear(&self) -> Result<ClearRestorePointsSummary> {
        let _recovery_guard = self.store.lock_recovery().await;
        let points = self.store.restore_points(None)?;
        let mut summary = ClearRestorePointsSummary::default();

        for point in points {
            if self.store.restore_point_is_protected(&point)? {
                summary.protected += 1;
                continue;
            }

            match tokio::fs::symlink_metadata(&point.storage_path).await {
                Ok(_) => {
                    let storage_path = self.validated_storage_path(&point)?;
                    remove_path_checked(&storage_path).await?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    // A stale database row is safe to forget because there is no
                    // corresponding filesystem object to remove.
                }
                Err(error) => return Err(error.into()),
            }

            if self.store.remove_restore_point(point.id)? {
                summary.cleared += 1;
            }
            if let Some(parent) = point.storage_path.parent()
                && parent != self.paths.backups
            {
                let _ = tokio::fs::remove_dir(parent).await;
            }
        }

        Ok(summary)
    }

    fn validated_storage_path(&self, point: &RestorePoint) -> Result<PathBuf> {
        let backup_root = std::fs::canonicalize(&self.paths.backups)?;
        let link_metadata = std::fs::symlink_metadata(&point.storage_path)?;
        if link_metadata.file_type().is_symlink() {
            return Err(BettertricksError::Security(
                "restore storage cannot be a symbolic link".into(),
            ));
        }
        let storage_path = std::fs::canonicalize(&point.storage_path)?;
        if !storage_path.starts_with(&backup_root) {
            return Err(BettertricksError::Security(format!(
                "restore storage is outside {}",
                backup_root.display()
            )));
        }
        let metadata = std::fs::metadata(&storage_path)?;
        let expected_kind = match point.method {
            RestoreMethod::Reflink | RestoreMethod::Btrfs => metadata.is_dir(),
            RestoreMethod::Archive => metadata.is_file(),
        };
        if !expected_kind {
            return Err(BettertricksError::Security(
                "restore storage type does not match its recorded method".into(),
            ));
        }
        Ok(storage_path)
    }
}

fn rename_noreplace(source: &Path, destination: &Path) -> Result<()> {
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        BettertricksError::Security("restore staging path contains a NUL byte".into())
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        BettertricksError::Security("restore target path contains a NUL byte".into())
    })?;
    // Both paths are on the target filesystem. RENAME_NOREPLACE closes the race
    // between the initial existence check and publication of the restored prefix.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return Err(BettertricksError::Conflict(format!(
            "{} appeared while the restore was running; it was not overwritten",
            Path::new(destination.to_str().unwrap_or("restore target")).display()
        )));
    }
    Err(error.into())
}

async fn can_reflink(source: &Path, destination_root: &Path) -> bool {
    let Some(test_file) = first_regular_file(source) else {
        return false;
    };
    let probe = destination_root.join(".reflink-probe");
    let status = Command::new("cp")
        .arg("--reflink=always")
        .arg(&test_file)
        .arg(&probe)
        .status()
        .await
        .is_ok_and(|status| status.success());
    let _ = tokio::fs::remove_file(probe).await;
    status
}

fn first_regular_file(root: &Path) -> Option<PathBuf> {
    walkdir::WalkDir::new(root)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .find(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
}

async fn run_checked(command: &mut Command, program: &str) -> Result<()> {
    let status = command.status().await?;
    if !status.success() {
        return Err(BettertricksError::CommandFailed {
            program: program.into(),
            code: status.code(),
        });
    }
    Ok(())
}

async fn remove_path(path: &Path) {
    let Ok(metadata) = tokio::fs::symlink_metadata(path).await else {
        return;
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        let _ = tokio::fs::remove_dir_all(path).await;
    } else {
        let _ = tokio::fs::remove_file(path).await;
    }
}

async fn remove_path_checked(path: &Path) -> Result<()> {
    let metadata = tokio::fs::symlink_metadata(path).await?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClearRestorePointsSummary {
    pub cleared: usize,
    pub protected: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OperationRecord, OperationState};

    fn restore_point(
        storage_path: PathBuf,
        target: PathBuf,
        method: RestoreMethod,
    ) -> RestorePoint {
        RestorePoint {
            id: Uuid::new_v4(),
            prefix_id: Uuid::new_v4(),
            prefix_name: "Game".into(),
            prefix_path: target,
            storage_path,
            method,
            created_at: Utc::now(),
            size_bytes: None,
            operation_id: None,
        }
    }

    #[tokio::test]
    async fn failed_restore_leaves_no_prefix_or_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let manager = RecoveryManager::new(paths, Arc::new(Store::open_in_memory().unwrap()));
        let target = temp.path().join("prefixes/game");
        let point = restore_point(
            temp.path().join("missing-backup"),
            target.clone(),
            RestoreMethod::Reflink,
        );

        assert!(manager.restore(&point).await.is_err());
        assert!(!target.exists());
        let parent = target.parent().unwrap();
        if parent.exists() {
            assert!(std::fs::read_dir(parent).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".partial")
            }));
        }
    }

    #[tokio::test]
    async fn corrupted_archive_restore_removes_staging_and_target() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let archive = paths.backups.join("corrupted.tar.zst");
        std::fs::write(&archive, b"not a compressed archive").unwrap();
        let manager = RecoveryManager::new(paths, Arc::new(Store::open_in_memory().unwrap()));
        let target = temp.path().join("prefixes/game");
        let point = restore_point(archive, target.clone(), RestoreMethod::Archive);

        assert!(manager.restore(&point).await.is_err());
        assert!(!target.exists());
        let parent = target.parent().unwrap();
        assert!(std::fs::read_dir(parent).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("bettertricks-restore")
        }));
    }

    #[tokio::test]
    async fn restore_rejects_storage_outside_the_backup_root() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let outside = temp.path().join("outside-backup");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("marker"), b"keep").unwrap();
        let manager = RecoveryManager::new(paths, Arc::new(Store::open_in_memory().unwrap()));
        let target = temp.path().join("prefixes/game");
        let point = restore_point(outside, target.clone(), RestoreMethod::Reflink);

        assert!(matches!(
            manager.restore(&point).await,
            Err(BettertricksError::Security(_))
        ));
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn clear_removes_snapshots_but_keeps_points_for_active_operations() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = RecoveryManager::new(paths.clone(), store.clone());

        let unprotected_path = paths.backups.join("unprotected.tar.zst");
        std::fs::write(&unprotected_path, b"unprotected snapshot").unwrap();
        let unprotected = restore_point(
            unprotected_path.clone(),
            temp.path().join("prefixes/unprotected"),
            RestoreMethod::Archive,
        );
        store.add_restore_point(&unprotected).unwrap();

        let protected_path = paths.backups.join("protected.tar.zst");
        std::fs::write(&protected_path, b"protected snapshot").unwrap();
        let mut protected = restore_point(
            protected_path.clone(),
            temp.path().join("prefixes/protected"),
            RestoreMethod::Archive,
        );
        let operation_id = Uuid::new_v4();
        protected.operation_id = Some(operation_id);
        store.add_restore_point(&protected).unwrap();
        let mut operation = OperationRecord {
            id: operation_id,
            prefix_id: protected.prefix_id,
            prefix_name: protected.prefix_name.clone(),
            recipes: vec!["corefonts".into()],
            state: OperationState::Running,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            current_step: 1,
            total_steps: 2,
            message: None,
            failures: Vec::new(),
        };
        store.upsert_operation(&operation).unwrap();

        assert_eq!(
            manager.clear().await.unwrap(),
            ClearRestorePointsSummary {
                cleared: 1,
                protected: 1,
            }
        );
        assert!(!unprotected_path.exists());
        assert!(protected_path.exists());
        let remaining = manager.list(None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, protected.id);

        operation.state = OperationState::Succeeded;
        operation.finished_at = Some(Utc::now());
        store.upsert_operation(&operation).unwrap();
        assert_eq!(
            manager.clear().await.unwrap(),
            ClearRestorePointsSummary {
                cleared: 1,
                protected: 0,
            }
        );
        assert!(!protected_path.exists());
        assert!(manager.list(None).unwrap().is_empty());
    }

    #[tokio::test]
    async fn clear_never_deletes_restore_storage_outside_the_backup_root() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = RecoveryManager::new(paths, store.clone());
        let outside = temp.path().join("outside.tar.zst");
        std::fs::write(&outside, b"keep").unwrap();
        let point = restore_point(
            outside.clone(),
            temp.path().join("prefixes/game"),
            RestoreMethod::Archive,
        );
        store.add_restore_point(&point).unwrap();

        assert!(matches!(
            manager.clear().await,
            Err(BettertricksError::Security(_))
        ));
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        let remaining = manager.list(None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, point.id);
    }

    #[test]
    fn restore_publication_never_replaces_a_path_that_appeared() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let target = temp.path().join("target");
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("restored"), b"new").unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("live"), b"keep").unwrap();

        assert!(matches!(
            rename_noreplace(&staging, &target),
            Err(BettertricksError::Conflict(_))
        ));
        assert_eq!(std::fs::read(target.join("live")).unwrap(), b"keep");
        assert_eq!(std::fs::read(staging.join("restored")).unwrap(), b"new");
    }
}
