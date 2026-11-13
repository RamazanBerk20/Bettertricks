use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use uuid::Uuid;

use crate::{
    AppPaths, AppSettings, BettertricksError, CatalogVersionRecord, OperationRecord,
    OperationState, PrefixSource, RecipeFailure, RestoreMethod, RestorePoint, Result,
};

pub type RegisteredPrefix = (PathBuf, String, PrefixSource, Option<PathBuf>);

pub struct Store {
    connection: Mutex<Connection>,
    recovery_lock: AsyncMutex<()>,
}

impl Store {
    pub fn open(paths: &AppPaths) -> Result<Self> {
        let connection = Connection::open(&paths.database)?;
        // The desktop app and the CLI share this WAL database.
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let store = Self {
            connection: Mutex::new(connection),
            recovery_lock: AsyncMutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let store = Self {
            connection: Mutex::new(Connection::open_in_memory()?),
            recovery_lock: AsyncMutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Runs in one IMMEDIATE transaction (behind the busy timeout) so a desktop app and a CLI
    /// opening an upgraded database at the same moment migrate it exactly once.
    fn migrate(&self) -> Result<()> {
        let mut guard = self.connection.lock();
        let connection =
            guard.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                payload TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS registered_prefixes (
                path TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source TEXT NOT NULL,
                runtime TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS operations (
                id TEXT PRIMARY KEY,
                prefix_id TEXT NOT NULL,
                prefix_name TEXT NOT NULL,
                recipes TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                current_step INTEGER NOT NULL,
                total_steps INTEGER NOT NULL,
                message TEXT,
                failures TEXT NOT NULL DEFAULT '[]'
            );

            CREATE INDEX IF NOT EXISTS operations_created_at_idx
                ON operations(created_at DESC);

            CREATE TABLE IF NOT EXISTS restore_points (
                id TEXT PRIMARY KEY,
                prefix_id TEXT NOT NULL,
                prefix_name TEXT NOT NULL,
                prefix_path TEXT NOT NULL,
                storage_path TEXT NOT NULL,
                method TEXT NOT NULL,
                created_at TEXT NOT NULL,
                size_bytes INTEGER,
                operation_id TEXT
            );

            CREATE TABLE IF NOT EXISTS catalog_versions (
                version TEXT PRIMARY KEY,
                upstream_tag TEXT NOT NULL,
                path TEXT NOT NULL,
                signature TEXT,
                active INTEGER NOT NULL DEFAULT 0,
                installed_at TEXT NOT NULL
            );

            INSERT OR IGNORE INTO schema_migrations(version, applied_at)
                VALUES (1, datetime('now'));
            "#,
        )?;

        add_column(
            &connection,
            "operations",
            "failures",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (2, datetime('now'))",
            [],
        )?;

        for (table, column, definition) in [
            ("restore_points", "prefix_source", "TEXT"),
            ("restore_points", "runtime", "TEXT"),
            ("catalog_versions", "previous", "TEXT"),
            ("catalog_versions", "pinned", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            add_column(&connection, table, column, definition)?;
        }
        connection.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (3, datetime('now'))",
            [],
        )?;
        connection.commit()?;
        Ok(())
    }

    pub fn settings(&self) -> Result<AppSettings> {
        let payload: Option<String> = self
            .connection
            .lock()
            .query_row("SELECT payload FROM settings WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        match payload {
            Some(payload) => Ok(serde_json::from_str(&payload)?),
            None => Ok(AppSettings::default()),
        }
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<()> {
        let payload = serde_json::to_string(settings)?;
        self.connection.lock().execute(
            r#"INSERT INTO settings(id, payload, updated_at)
               VALUES (1, ?1, ?2)
               ON CONFLICT(id) DO UPDATE SET payload=excluded.payload, updated_at=excluded.updated_at"#,
            params![payload, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn register_prefix(
        &self,
        path: &Path,
        name: &str,
        source: PrefixSource,
        runtime: Option<&Path>,
    ) -> Result<()> {
        self.connection.lock().execute(
            r#"INSERT INTO registered_prefixes(path, name, source, runtime, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5)
               ON CONFLICT(path) DO UPDATE SET name=excluded.name, source=excluded.source,
                 runtime=excluded.runtime"#,
            params![
                path.to_string_lossy(),
                name,
                serde_json::to_string(&source)?,
                runtime.map(|path| path.to_string_lossy().to_string()),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn unregister_prefix(&self, path: &Path) -> Result<()> {
        self.connection.lock().execute(
            "DELETE FROM registered_prefixes WHERE path = ?1",
            [path.to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    pub fn registered_prefixes(&self) -> Result<Vec<RegisteredPrefix>> {
        let connection = self.connection.lock();
        let mut statement = connection
            .prepare("SELECT path, name, source, runtime FROM registered_prefixes ORDER BY name")?;
        let rows = statement.query_map([], |row| {
            let path: String = row.get(0)?;
            let name: String = row.get(1)?;
            let source: String = row.get(2)?;
            let runtime: Option<String> = row.get(3)?;
            Ok((path, name, source, runtime))
        })?;

        let mut prefixes = Vec::new();
        for row in rows {
            let (path, name, source, runtime) = row?;
            prefixes.push((
                PathBuf::from(path),
                name,
                serde_json::from_str(&source)?,
                runtime.map(PathBuf::from),
            ));
        }
        Ok(prefixes)
    }

    pub fn upsert_operation(&self, operation: &OperationRecord) -> Result<()> {
        self.connection.lock().execute(
            r#"INSERT INTO operations(
                 id, prefix_id, prefix_name, recipes, state, created_at, started_at, finished_at,
                 current_step, total_steps, message, failures
               ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
               ON CONFLICT(id) DO UPDATE SET state=excluded.state, started_at=excluded.started_at,
                 finished_at=excluded.finished_at, current_step=excluded.current_step,
                 total_steps=excluded.total_steps, message=excluded.message,
                 failures=excluded.failures"#,
            params![
                operation.id.to_string(),
                operation.prefix_id.to_string(),
                operation.prefix_name,
                serde_json::to_string(&operation.recipes)?,
                serde_json::to_string(&operation.state)?,
                operation.created_at.to_rfc3339(),
                operation.started_at.map(|date| date.to_rfc3339()),
                operation.finished_at.map(|date| date.to_rfc3339()),
                operation.current_step as i64,
                operation.total_steps as i64,
                operation.message,
                serde_json::to_string(&operation.failures)?,
            ],
        )?;
        Ok(())
    }

    pub fn operations(&self, limit: usize) -> Result<Vec<OperationRecord>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            r#"SELECT id, prefix_id, prefix_name, recipes, state, created_at, started_at,
                      finished_at, current_step, total_steps, message, failures
               FROM operations ORDER BY created_at DESC LIMIT ?1"#,
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, String>(11)?,
            ))
        })?;

        let mut operations = Vec::new();
        for row in rows {
            let (
                id,
                prefix_id,
                prefix_name,
                recipes,
                state,
                created,
                started,
                finished,
                step,
                total,
                message,
                failures,
            ) = row?;
            operations.push(OperationRecord {
                id: Uuid::parse_str(&id).map_err(serialization)?,
                prefix_id: Uuid::parse_str(&prefix_id).map_err(serialization)?,
                prefix_name,
                recipes: serde_json::from_str(&recipes)?,
                state: serde_json::from_str::<OperationState>(&state)?,
                created_at: parse_date(&created)?,
                started_at: started.as_deref().map(parse_date).transpose()?,
                finished_at: finished.as_deref().map(parse_date).transpose()?,
                current_step: step as usize,
                total_steps: total as usize,
                message,
                failures: serde_json::from_str::<Vec<RecipeFailure>>(&failures)?,
            });
        }
        Ok(operations)
    }

    pub fn clear_operation_history(&self) -> Result<usize> {
        let mut deleted = 0;
        for record in self.operations(usize::MAX >> 1)? {
            if record.state.is_terminal() {
                deleted += self.connection.lock().execute(
                    "DELETE FROM operations WHERE id = ?1",
                    [record.id.to_string()],
                )?;
            }
        }
        Ok(deleted)
    }

    /// Rows whose state is not terminal (live, or orphaned by a crash).
    pub fn unfinished_operations(&self) -> Result<Vec<OperationRecord>> {
        Ok(self
            .operations(usize::MAX >> 1)?
            .into_iter()
            .filter(|record| !record.state.is_terminal())
            .collect())
    }

    /// Updates a row's state only while it is still live (never overwrites a terminal state).
    pub fn set_live_operation_state(
        &self,
        id: Uuid,
        state: OperationState,
        message: &str,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT state FROM operations WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(current) = current
            && !serde_json::from_str::<OperationState>(&current)?.is_terminal()
        {
            transaction.execute(
                "UPDATE operations SET state = ?2, message = ?3 WHERE id = ?1",
                params![id.to_string(), serde_json::to_string(&state)?, message],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Marks an orphaned row interrupted, but only while it is still non-terminal: an
    /// operation that finished after the caller listed it keeps its real outcome.
    /// Returns whether the row was changed.
    pub fn mark_interrupted(&self, id: Uuid, message: &str) -> Result<bool> {
        let terminal = [
            OperationState::Succeeded,
            OperationState::Failed,
            OperationState::Cancelled,
            OperationState::Interrupted,
        ]
        .map(|state| serde_json::to_string(&state))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
        let changed = self.connection.lock().execute(
            "UPDATE operations SET state = ?2, finished_at = ?3, message = ?4
             WHERE id = ?1 AND state NOT IN (?5, ?6, ?7, ?8)",
            params![
                id.to_string(),
                terminal[3],
                Utc::now().to_rfc3339(),
                message,
                terminal[0],
                terminal[1],
                terminal[2],
                terminal[3],
            ],
        )?;
        Ok(changed > 0)
    }

    /// Whether the user pinned the active catalog by rolling back to it.
    pub fn active_catalog_pinned(&self) -> Result<bool> {
        Ok(self
            .connection
            .lock()
            .query_row(
                "SELECT pinned FROM catalog_versions WHERE active = 1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    pub fn pin_active_catalog(&self) -> Result<()> {
        self.connection
            .lock()
            .execute("UPDATE catalog_versions SET pinned = (active = 1)", [])?;
        Ok(())
    }

    pub fn add_restore_point(&self, point: &RestorePoint) -> Result<()> {
        self.connection.lock().execute(
            r#"INSERT INTO restore_points(
                 id, prefix_id, prefix_name, prefix_path, storage_path, method, created_at,
                 size_bytes, operation_id, prefix_source, runtime
               ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                point.id.to_string(),
                point.prefix_id.to_string(),
                point.prefix_name,
                point.prefix_path.to_string_lossy(),
                point.storage_path.to_string_lossy(),
                serde_json::to_string(&point.method)?,
                point.created_at.to_rfc3339(),
                point.size_bytes.map(|value| value as i64),
                point.operation_id.map(|id| id.to_string()),
                point
                    .prefix_source
                    .map(|source| serde_json::to_string(&source))
                    .transpose()?,
                point
                    .runtime
                    .as_ref()
                    .map(|path| path.to_string_lossy().to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn restore_points(&self, prefix_id: Option<Uuid>) -> Result<Vec<RestorePoint>> {
        let connection = self.connection.lock();
        let sql = if prefix_id.is_some() {
            "SELECT id, prefix_id, prefix_name, prefix_path, storage_path, method, created_at, size_bytes, operation_id, prefix_source, runtime FROM restore_points WHERE prefix_id = ?1 ORDER BY created_at DESC"
        } else {
            "SELECT id, prefix_id, prefix_name, prefix_path, storage_path, method, created_at, size_bytes, operation_id, prefix_source, runtime FROM restore_points ORDER BY created_at DESC"
        };
        let mut statement = connection.prepare(sql)?;
        let id_string = prefix_id.map(|id| id.to_string());
        let mapper = |row: &rusqlite::Row<'_>| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        };

        let mut points = Vec::new();
        if let Some(id) = id_string {
            for row in statement.query_map([id], mapper)? {
                points.push(parse_restore_row(row?)?);
            }
        } else {
            for row in statement.query_map([], mapper)? {
                points.push(parse_restore_row(row?)?);
            }
        }
        Ok(points)
    }

    pub(crate) async fn lock_recovery(&self) -> AsyncMutexGuard<'_, ()> {
        self.recovery_lock.lock().await
    }

    pub(crate) fn restore_point_is_protected(&self, point: &RestorePoint) -> Result<bool> {
        let Some(operation_id) = point.operation_id else {
            return Ok(false);
        };
        let state = self
            .connection
            .lock()
            .query_row(
                "SELECT state FROM operations WHERE id = ?1",
                [operation_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(state) = state else {
            return Ok(false);
        };
        let state = serde_json::from_str::<OperationState>(&state)?;
        Ok(!state.is_terminal())
    }

    pub(crate) fn remove_restore_point(&self, id: Uuid) -> Result<bool> {
        Ok(self
            .connection
            .lock()
            .execute("DELETE FROM restore_points WHERE id = ?1", [id.to_string()])?
            > 0)
    }

    pub fn catalog_versions(&self) -> Result<Vec<CatalogVersionRecord>> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            r#"SELECT version, upstream_tag, path, signature, active, installed_at, previous
               FROM catalog_versions ORDER BY installed_at DESC"#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(parse_catalog_row(row?)?);
        }
        Ok(versions)
    }

    pub fn active_catalog_version(&self) -> Result<Option<CatalogVersionRecord>> {
        Ok(self
            .catalog_versions()?
            .into_iter()
            .find(|version| version.active && version.path.is_dir()))
    }

    /// Makes `version` the active catalog. Re-activating the version that is already active
    /// only refreshes its path (an AppImage mounts at a new path on every launch) and keeps
    /// its rollback pointer; switching versions records the old one as `previous`.
    pub fn activate_catalog_version(
        &self,
        version: &str,
        upstream_tag: &str,
        path: &Path,
        signature: Option<&str>,
    ) -> Result<()> {
        let mut connection = self.connection.lock();
        let transaction = connection.transaction()?;
        let current: Option<String> = transaction
            .query_row(
                "SELECT version FROM catalog_versions WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if current.as_deref() == Some(version) {
            transaction.execute(
                "UPDATE catalog_versions SET path = ?2, upstream_tag = ?3, signature = COALESCE(?4, signature) WHERE version = ?1",
                params![version, path.to_string_lossy(), upstream_tag, signature],
            )?;
            transaction.commit()?;
            return Ok(());
        }
        // Activating a different catalog (an explicit update or a rollback) replaces any pin;
        // a rollback re-pins its target afterwards.
        transaction.execute(
            "UPDATE catalog_versions SET active = 0, pinned = 0 WHERE active = 1 OR pinned = 1",
            [],
        )?;
        transaction.execute(
            r#"INSERT INTO catalog_versions(
                 version, upstream_tag, path, signature, active, installed_at, previous
               ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)
               ON CONFLICT(version) DO UPDATE SET upstream_tag=excluded.upstream_tag,
                 path=excluded.path, signature=excluded.signature, active=1,
                 installed_at=excluded.installed_at, previous=excluded.previous"#,
            params![
                version,
                upstream_tag,
                path.to_string_lossy(),
                signature,
                Utc::now().to_rfc3339(),
                current,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

type RestoreRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);
type CatalogRow = (
    String,
    String,
    String,
    Option<String>,
    bool,
    String,
    Option<String>,
);

fn add_column(connection: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    if has_column(connection, table, column)? {
        return Ok(());
    }
    match connection.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    ) {
        Err(error) if error.to_string().contains("duplicate column") => Ok(()),
        result => result.map(|_| ()).map_err(Into::into),
    }
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for name in columns {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn parse_catalog_row(row: CatalogRow) -> Result<CatalogVersionRecord> {
    Ok(CatalogVersionRecord {
        version: row.0,
        upstream_tag: row.1,
        path: PathBuf::from(row.2),
        signature: row.3,
        active: row.4,
        installed_at: parse_date(&row.5)?,
        previous: row.6,
    })
}

fn parse_restore_row(row: RestoreRow) -> Result<RestorePoint> {
    Ok(RestorePoint {
        id: Uuid::parse_str(&row.0).map_err(serialization)?,
        prefix_id: Uuid::parse_str(&row.1).map_err(serialization)?,
        prefix_name: row.2,
        prefix_path: PathBuf::from(row.3),
        storage_path: PathBuf::from(row.4),
        method: serde_json::from_str::<RestoreMethod>(&row.5)?,
        created_at: parse_date(&row.6)?,
        size_bytes: row.7.map(|value| value as u64),
        operation_id: row
            .8
            .map(|id| Uuid::parse_str(&id).map_err(serialization))
            .transpose()?,
        prefix_source: row
            .9
            .map(|source| serde_json::from_str(&source))
            .transpose()?,
        runtime: row.10.map(PathBuf::from),
    })
}

fn parse_date(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(serialization)
}

fn serialization(error: impl std::fmt::Display) -> BettertricksError {
    BettertricksError::Serialization(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_existing_operation_history_with_empty_failures() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let operation_id = Uuid::new_v4();
        let prefix_id = Uuid::new_v4();
        let connection = Connection::open(&paths.database).unwrap();
        connection
            .execute_batch(&format!(
                r#"
                CREATE TABLE operations (
                    id TEXT PRIMARY KEY,
                    prefix_id TEXT NOT NULL,
                    prefix_name TEXT NOT NULL,
                    recipes TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    finished_at TEXT,
                    current_step INTEGER NOT NULL,
                    total_steps INTEGER NOT NULL,
                    message TEXT
                );
                INSERT INTO operations(
                    id, prefix_id, prefix_name, recipes, state, created_at, started_at,
                    finished_at, current_step, total_steps, message
                ) VALUES (
                    '{operation_id}', '{prefix_id}', 'Existing prefix', '["corefonts"]',
                    '"failed"', '2026-07-22T12:00:00+00:00', NULL,
                    '2026-07-22T12:00:01+00:00', 1, 1, 'Old failure'
                );
                "#
            ))
            .unwrap();
        drop(connection);

        let store = Store::open(&paths).unwrap();
        let history = store.operations(10).unwrap();

        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, operation_id);
        assert!(history[0].failures.is_empty());
        let migration_exists = store
            .connection
            .lock()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 2)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap();
        assert!(migration_exists);
    }

    #[test]
    fn settings_saved_by_v1_keep_every_restore_point() {
        let store = Store::open_in_memory().unwrap();
        store
            .connection
            .lock()
            .execute(
                "INSERT INTO settings(id, payload, updated_at) VALUES (1, ?1, 'now')",
                [r#"{"theme":"system","language":"system","catalog_auto_update":true,"restore_before_managed_changes":true,"show_advanced":false,"reduced_motion":false,"custom_wine_binary":null}"#],
            )
            .unwrap();
        assert_eq!(store.settings().unwrap().restore_point_retention, 0);
        assert_eq!(AppSettings::default().restore_point_retention, 0);
    }

    #[test]
    fn interruption_never_overwrites_a_finished_operation() {
        let store = Store::open_in_memory().unwrap();
        let record = |state| OperationRecord {
            id: Uuid::new_v4(),
            prefix_id: Uuid::new_v4(),
            prefix_name: "P".into(),
            recipes: Vec::new(),
            state,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            current_step: 0,
            total_steps: 0,
            message: None,
            failures: Vec::new(),
        };
        // Listed as running by a reconciler, then finished before it wrote.
        let finished = record(OperationState::Succeeded);
        let orphan = record(OperationState::Running);
        store.upsert_operation(&finished).unwrap();
        store.upsert_operation(&orphan).unwrap();

        assert!(!store.mark_interrupted(finished.id, "Interrupted").unwrap());
        assert!(store.mark_interrupted(orphan.id, "Interrupted").unwrap());
        let states = store
            .operations(10)
            .unwrap()
            .into_iter()
            .map(|row| (row.id, row.state))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(states[&finished.id], OperationState::Succeeded);
        assert_eq!(states[&orphan.id], OperationState::Interrupted);
    }

    #[test]
    fn clears_only_finished_operation_history() {
        let store = Store::open_in_memory().unwrap();
        let prefix_id = Uuid::new_v4();
        let operation = |state: OperationState| OperationRecord {
            id: Uuid::new_v4(),
            prefix_id,
            prefix_name: "Test prefix".into(),
            recipes: vec!["corefonts".into()],
            state,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: matches!(
                state,
                OperationState::Succeeded | OperationState::Failed | OperationState::Cancelled
            )
            .then(Utc::now),
            current_step: 1,
            total_steps: 1,
            message: None,
            failures: Vec::new(),
        };
        let running = operation(OperationState::Running);
        let running_id = running.id;
        for record in [
            running,
            operation(OperationState::Succeeded),
            operation(OperationState::Failed),
            operation(OperationState::Cancelled),
        ] {
            store.upsert_operation(&record).unwrap();
        }

        store
            .upsert_operation(&operation(OperationState::Interrupted))
            .unwrap();
        assert_eq!(store.clear_operation_history().unwrap(), 4);
        let remaining = store.operations(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, running_id);
        assert_eq!(remaining[0].state, OperationState::Running);
    }
}
