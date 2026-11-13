use std::path::PathBuf;
use std::sync::Arc;

use bettertricks_core::{
    AppPaths, AppSettings, CacheStats, Catalog, CatalogQuery, CatalogRelease, CatalogSummary,
    CatalogUpdater, LegacyVerbHost, LegacyVerbInfo, LiveOperation, OperationEngine,
    OperationEventSink, OperationOptions, OperationPlan, OperationRecord, OperationRequest,
    Planner, PrefixArchitecture, PrefixDiscovery, PromptResponse, Recipe, RecipeListItem,
    RecoveryManager, RestorePoint, Session, Store, SystemInspector, SystemReport, WinePrefix,
    install_managed_compatibility_host, rollback_catalog as rollback_active_catalog,
    rollback_target,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::process::Command;
use uuid::Uuid;

struct AppState {
    paths: AppPaths,
    store: Arc<Store>,
    catalog: Catalog,
    discovery: PrefixDiscovery,
    planner: Planner,
    engine: OperationEngine,
    recovery: RecoveryManager,
    inspector: SystemInspector,
    legacy_host: LegacyVerbHost,
    catalog_updater: Option<CatalogUpdateConfig>,
}

#[derive(Clone)]
struct CatalogUpdateConfig {
    updater: CatalogUpdater,
    index_url: url::Url,
}

#[derive(Debug, Serialize)]
struct BootstrapPayload {
    system: SystemReport,
    prefixes: Vec<WinePrefix>,
    catalog: CatalogSummary,
    settings: AppSettings,
    cache: CacheStats,
    operations: Vec<OperationRecord>,
    restore_points: Vec<RestorePoint>,
    catalog_signed: bool,
    catalog_rollback_available: bool,
    live_operations: Vec<LiveOperation>,
}

#[derive(Debug, Serialize)]
struct CatalogUpdateStatus {
    configured: bool,
    signed: bool,
    rollback_available: bool,
    available: Option<CatalogRelease>,
    message: String,
}

#[derive(Debug, Serialize)]
struct ClearRestorePointsResponse {
    cleared: usize,
    protected: usize,
    restore_points: Vec<RestorePoint>,
}

#[derive(Debug, Deserialize)]
struct RegisterPrefixRequest {
    path: PathBuf,
    name: String,
    runtime: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CreatePrefixRequest {
    path: PathBuf,
    name: String,
    architecture: PrefixArchitecture,
    runtime: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct PrefixToolRequest {
    prefix_id: Uuid,
    tool: String,
    file: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct LegacyVerbRunRequest {
    prefix_id: Uuid,
    path: PathBuf,
    trusted: bool,
    options: OperationOptions,
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> CommandResult<BootstrapPayload> {
    state
        .engine
        .reconcile_interrupted()
        .map_err(command_error)?;
    let system = state.inspector.inspect().await.map_err(command_error)?;
    let prefixes = state.discovery.discover().await.map_err(command_error)?;
    Ok(BootstrapPayload {
        system,
        prefixes,
        catalog: state.catalog.summary(),
        settings: state.store.settings().map_err(command_error)?,
        cache: state.inspector.cache_stats(),
        operations: state.store.operations(100).map_err(command_error)?,
        restore_points: state.recovery.list(None).map_err(command_error)?,
        catalog_signed: state
            .store
            .active_catalog_version()
            .map_err(command_error)?
            .and_then(|version| version.signature)
            .is_some(),
        catalog_rollback_available: rollback_target(&state.store)
            .map_err(command_error)?
            .is_some(),
        live_operations: state.engine.live_operations(),
    })
}

/// Live operations with their latest event and pending prompt; the UI re-reads this after a
/// reload so a prompt is never lost with the webview's state.
#[tauri::command]
fn active_operations(state: State<'_, AppState>) -> Vec<LiveOperation> {
    state.engine.live_operations()
}

#[tauri::command]
async fn inspect_system(state: State<'_, AppState>) -> CommandResult<SystemReport> {
    state.inspector.inspect().await.map_err(command_error)
}

#[tauri::command]
async fn install_compatibility_host(state: State<'_, AppState>) -> CommandResult<SystemReport> {
    install_managed_compatibility_host(&state.paths, &state.catalog)
        .await
        .map_err(command_error)?;
    state.inspector.inspect().await.map_err(command_error)
}

#[tauri::command]
async fn list_prefixes(state: State<'_, AppState>) -> CommandResult<Vec<WinePrefix>> {
    state.discovery.discover().await.map_err(command_error)
}

#[tauri::command]
async fn register_prefix(
    state: State<'_, AppState>,
    request: RegisterPrefixRequest,
) -> CommandResult<Vec<WinePrefix>> {
    if request.name.trim().is_empty() {
        return Err("Prefix name cannot be empty".into());
    }
    state
        .discovery
        .register_manual(
            &request.path,
            request.name.trim(),
            request.runtime.as_deref(),
        )
        .map_err(command_error)?;
    state.discovery.discover().await.map_err(command_error)
}

#[tauri::command]
async fn unregister_prefix(
    state: State<'_, AppState>,
    path: PathBuf,
) -> CommandResult<Vec<WinePrefix>> {
    state
        .discovery
        .unregister_manual(&path)
        .map_err(command_error)?;
    state.discovery.discover().await.map_err(command_error)
}

#[tauri::command]
async fn create_prefix(
    state: State<'_, AppState>,
    request: CreatePrefixRequest,
) -> CommandResult<Vec<WinePrefix>> {
    if request.name.trim().is_empty() {
        return Err("Prefix name cannot be empty".into());
    }
    state
        .engine
        .create_prefix(
            &request.path,
            request.name.trim(),
            request.architecture,
            request.runtime.as_deref(),
        )
        .await
        .map_err(command_error)?;
    state.discovery.discover().await.map_err(command_error)
}

#[tauri::command]
async fn catalog_search(
    state: State<'_, AppState>,
    query: CatalogQuery,
) -> CommandResult<Vec<RecipeListItem>> {
    let prefix = if let Some(prefix_id) = query.prefix_id {
        Some(
            state
                .discovery
                .by_id_cached(prefix_id)
                .await
                .map_err(command_error)?,
        )
    } else {
        None
    };
    Ok(state.catalog.search(&query, prefix.as_ref()))
}

#[tauri::command]
fn catalog_summary(state: State<'_, AppState>) -> CatalogSummary {
    state.catalog.summary()
}

#[tauri::command]
fn get_recipe(state: State<'_, AppState>, id: String) -> CommandResult<Recipe> {
    state.catalog.get(&id).map_err(command_error)
}

#[tauri::command]
async fn import_manual_file(
    state: State<'_, AppState>,
    recipe_id: String,
    file_id: String,
    path: PathBuf,
) -> CommandResult<PathBuf> {
    state
        .catalog
        .import_manual_file(&recipe_id, &file_id, &path)
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn plan_operation(
    state: State<'_, AppState>,
    request: OperationRequest,
) -> CommandResult<OperationPlan> {
    let prefix = state
        .discovery
        .by_id(request.prefix_id)
        .await
        .map_err(command_error)?;
    plan_blocking(&state.planner, request, prefix).await
}

/// Planning may probe the system `winetricks` (cached after the first call); keep it off the
/// async workers.
async fn plan_blocking(
    planner: &Planner,
    request: OperationRequest,
    prefix: WinePrefix,
) -> CommandResult<OperationPlan> {
    let planner = planner.clone();
    tauri::async_runtime::spawn_blocking(move || planner.plan(request, prefix))
        .await
        .map_err(command_error)?
        .map_err(command_error)
}

#[tauri::command]
async fn start_operation(
    app: AppHandle,
    state: State<'_, AppState>,
    plan: OperationPlan,
) -> CommandResult<Uuid> {
    // The review plan crosses the webview boundary. Rebuild it from trusted catalog and
    // discovery state so a forged IPC payload cannot substitute a prefix path or native steps.
    let request = OperationRequest {
        prefix_id: plan.prefix.id,
        recipes: plan.requested_recipes,
        input_values: plan
            .inputs
            .into_iter()
            .filter_map(|input| input.value.map(|value| (input.key, value)))
            .collect(),
        options: plan.options,
    };
    let prefix = state
        .discovery
        .by_id(request.prefix_id)
        .await
        .map_err(command_error)?;
    let verified_plan = plan_blocking(&state.planner, request, prefix).await?;
    let sink: Arc<dyn OperationEventSink> =
        Arc::new(move |event: bettertricks_core::OperationEvent| {
            let _ = app.emit("operation-event", event);
        });
    state
        .engine
        .start(verified_plan, sink)
        .await
        .map_err(command_error)
}

#[tauri::command]
fn cancel_operation(state: State<'_, AppState>, operation_id: Uuid) -> CommandResult<()> {
    state.engine.cancel(operation_id).map_err(command_error)
}

#[tauri::command]
fn respond_to_prompt(state: State<'_, AppState>, response: PromptResponse) -> CommandResult<()> {
    state.engine.respond(response).map_err(command_error)
}

#[tauri::command]
fn operation_history(state: State<'_, AppState>) -> CommandResult<Vec<OperationRecord>> {
    state
        .engine
        .reconcile_interrupted()
        .map_err(command_error)?;
    state.store.operations(200).map_err(command_error)
}

#[tauri::command]
fn clear_operation_history(state: State<'_, AppState>) -> CommandResult<Vec<OperationRecord>> {
    state
        .store
        .clear_operation_history()
        .map_err(command_error)?;
    state.store.operations(200).map_err(command_error)
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> CommandResult<AppSettings> {
    state.store.settings().map_err(command_error)
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: AppSettings) -> CommandResult<AppSettings> {
    state
        .store
        .save_settings(&settings)
        .map_err(command_error)?;
    Ok(settings)
}

#[tauri::command]
fn cache_stats(state: State<'_, AppState>) -> CacheStats {
    state.inspector.cache_stats()
}

#[tauri::command]
async fn clear_cache(state: State<'_, AppState>) -> CommandResult<CacheStats> {
    let root = &state.paths.winetricks_cache;
    let mut entries = tokio::fs::read_dir(root).await.map_err(command_error)?;
    while let Some(entry) = entries.next_entry().await.map_err(command_error)? {
        let path = entry.path();
        if path.is_dir() {
            tokio::fs::remove_dir_all(path)
                .await
                .map_err(command_error)?;
        } else {
            tokio::fs::remove_file(path).await.map_err(command_error)?;
        }
    }
    Ok(state.inspector.cache_stats())
}

#[tauri::command]
fn list_restore_points(
    state: State<'_, AppState>,
    prefix_id: Option<Uuid>,
) -> CommandResult<Vec<RestorePoint>> {
    state.recovery.list(prefix_id).map_err(command_error)
}

#[tauri::command]
async fn create_restore_point(
    state: State<'_, AppState>,
    prefix_id: Uuid,
) -> CommandResult<RestorePoint> {
    let prefix = state
        .discovery
        .by_id(prefix_id)
        .await
        .map_err(command_error)?;
    state
        .engine
        .create_restore_point(&prefix)
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn delete_restore_point(
    state: State<'_, AppState>,
    restore_point_id: Uuid,
) -> CommandResult<Vec<RestorePoint>> {
    state
        .recovery
        .delete(restore_point_id)
        .await
        .map_err(command_error)?;
    state.recovery.list(None).map_err(command_error)
}

#[tauri::command]
async fn clear_restore_points(
    state: State<'_, AppState>,
) -> CommandResult<ClearRestorePointsResponse> {
    let summary = state.recovery.clear().await.map_err(command_error)?;
    Ok(ClearRestorePointsResponse {
        cleared: summary.cleared,
        protected: summary.protected,
        restore_points: state.recovery.list(None).map_err(command_error)?,
    })
}

#[tauri::command]
async fn restore_prefix(
    state: State<'_, AppState>,
    restore_point_id: Uuid,
    replace_existing: Option<bool>,
) -> CommandResult<()> {
    let restore_point = state
        .recovery
        .list(None)
        .map_err(command_error)?
        .into_iter()
        .find(|point| point.id == restore_point_id)
        .ok_or_else(|| format!("Restore point {restore_point_id} was not found"))?;
    state
        .engine
        .restore(&restore_point, replace_existing.unwrap_or(false))
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn launch_prefix_tool(
    state: State<'_, AppState>,
    request: PrefixToolRequest,
) -> CommandResult<()> {
    let prefix = state
        .discovery
        .by_id(request.prefix_id)
        .await
        .map_err(command_error)?;
    let (program, arguments): (&str, Vec<String>) = match request.tool.as_str() {
        "winecfg" => ("winecfg", Vec::new()),
        "regedit" => ("regedit", Vec::new()),
        "taskmgr" => ("taskmgr", Vec::new()),
        "explorer" => ("explorer", Vec::new()),
        "uninstaller" => ("uninstaller", Vec::new()),
        "cmd" => ("cmd", Vec::new()),
        "installer" => {
            let file = request
                .file
                .ok_or_else(|| "Select an installer file".to_string())?;
            let file = tokio::fs::canonicalize(&file)
                .await
                .map_err(|error| format!("The selected installer is unavailable: {error}"))?;
            if !tokio::fs::metadata(&file)
                .await
                .map_err(command_error)?
                .is_file()
            {
                return Err("The selected installer is not a regular file".into());
            }
            let extension = file
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .ok_or_else(|| "Select an EXE or MSI installer".to_string())?;
            let file = file.to_string_lossy().to_string();
            match extension.as_str() {
                "exe" => ("start", vec!["/unix".into(), file]),
                "msi" => ("msiexec", vec!["/i".into(), file]),
                _ => return Err("Select an EXE or MSI installer".into()),
            }
        }
        _ => return Err(format!("Unknown prefix tool: {}", request.tool)),
    };
    state
        .engine
        .launch_tool(&prefix, program, &arguments)
        .await
        .map_err(command_error)
}

/// Opens a directory in the file manager: only known prefixes and the app's own data,
/// state and cache directories, never arbitrary files (a `.desktop` file could execute).
#[tauri::command]
async fn open_path(state: State<'_, AppState>, path: PathBuf) -> CommandResult<()> {
    let path = std::fs::canonicalize(&path).map_err(command_error)?;
    let allowed_root = [
        &state.paths.data,
        &state.paths.state,
        &state.paths.winetricks_cache,
    ]
    .iter()
    .filter_map(|root| std::fs::canonicalize(root).ok())
    .any(|root| path.starts_with(root));
    if !path.is_dir() || !(allowed_root || state.discovery.is_known_prefix_path(&path)) {
        return Err("Only Wine prefixes and Bettertricks folders can be opened".into());
    }
    Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map_err(command_error)?;
    Ok(())
}

#[tauri::command]
async fn open_url(url: String) -> CommandResult<()> {
    if url.len() > 4_096 {
        return Err("The source URL is too long".into());
    }
    let parsed = url::Url::parse(&url).map_err(|_| "The source URL is invalid".to_string())?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Only credential-free HTTPS source URLs can be opened".into());
    }
    Command::new("xdg-open")
        .arg(parsed.as_str())
        .spawn()
        .map_err(command_error)?;
    Ok(())
}

/// Fixed, argument-free opener for the owner's sponsor page; deliberately not routed through `open_url`.
#[tauri::command]
async fn open_sponsor_page() -> CommandResult<()> {
    Command::new("xdg-open")
        .arg("https://github.com/sponsors/RamazanBerk20")
        .spawn()
        .map_err(command_error)?;
    Ok(())
}

#[tauri::command]
async fn trash_prefix(
    state: State<'_, AppState>,
    prefix_id: Uuid,
    confirmation: String,
) -> CommandResult<Vec<WinePrefix>> {
    let prefix = state
        .discovery
        .by_id(prefix_id)
        .await
        .map_err(command_error)?;
    if confirmation.trim() != prefix.name {
        return Err(format!(
            "Type {} exactly to move this prefix to Trash",
            prefix.name
        ));
    }
    state
        .engine
        .trash_prefix(&prefix)
        .await
        .map_err(command_error)?;
    state.discovery.discover().await.map_err(command_error)
}

#[tauri::command]
fn inspect_legacy_verb(state: State<'_, AppState>, path: PathBuf) -> CommandResult<LegacyVerbInfo> {
    state.legacy_host.inspect(&path).map_err(command_error)
}

#[tauri::command]
async fn run_legacy_verb(
    state: State<'_, AppState>,
    request: LegacyVerbRunRequest,
) -> CommandResult<()> {
    if !request.trusted {
        return Err("Review the legacy verb and explicitly confirm trust before running it".into());
    }
    let prefix = state
        .discovery
        .by_id(request.prefix_id)
        .await
        .map_err(command_error)?;
    let info = state
        .legacy_host
        .inspect(&request.path)
        .map_err(command_error)?;
    state
        .engine
        .run_legacy_verb(&info, &prefix, &request.options)
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn check_catalog_update(state: State<'_, AppState>) -> CommandResult<CatalogUpdateStatus> {
    let rollback_available = rollback_target(&state.store)
        .map_err(command_error)?
        .is_some();
    let Some(config) = &state.catalog_updater else {
        return Ok(CatalogUpdateStatus {
            configured: false,
            signed: state
                .store
                .active_catalog_version()
                .map_err(command_error)?
                .and_then(|version| version.signature)
                .is_some(),
            rollback_available,
            available: None,
            message: "No signed catalog update channel is configured.".into(),
        });
    };
    let index = config
        .updater
        .fetch_index(&config.index_url)
        .await
        .map_err(command_error)?;
    let current = state.catalog.summary().version;
    let available = index.latest_after(&current).cloned();
    Ok(CatalogUpdateStatus {
        configured: true,
        signed: state
            .store
            .active_catalog_version()
            .map_err(command_error)?
            .and_then(|version| version.signature)
            .is_some(),
        rollback_available,
        message: if available.is_some() {
            "A signed catalog update is available.".into()
        } else {
            "The recipe catalog is up to date.".into()
        },
        available,
    })
}

#[tauri::command]
async fn install_catalog_update(
    state: State<'_, AppState>,
    release: CatalogRelease,
) -> CommandResult<CatalogSummary> {
    let config = state
        .catalog_updater
        .as_ref()
        .ok_or_else(|| "No catalog update channel is configured".to_string())?;
    config
        .updater
        .install(&release)
        .await
        .map_err(command_error)?;
    Ok(state.catalog.summary())
}

#[tauri::command]
fn rollback_catalog_version(state: State<'_, AppState>) -> CommandResult<CatalogSummary> {
    rollback_active_catalog(&state.catalog, &state.store).map_err(command_error)?;
    Ok(state.catalog.summary())
}

fn build_state(app: &tauri::App) -> Result<AppState, String> {
    let paths = AppPaths::discover().map_err(command_error)?;
    let session = Session::open(paths, &catalog_path(app)?).map_err(command_error)?;
    let catalog_updater =
        configured_catalog_updater(&session.paths, &session.catalog, &session.store);
    Ok(AppState {
        paths: session.paths,
        store: session.store,
        catalog: session.catalog,
        discovery: session.discovery,
        planner: session.planner,
        engine: session.engine,
        recovery: session.recovery,
        inspector: session.inspector,
        legacy_host: session.legacy_host,
        catalog_updater,
    })
}

fn configured_catalog_updater(
    paths: &AppPaths,
    catalog: &Catalog,
    store: &Arc<Store>,
) -> Option<CatalogUpdateConfig> {
    let key = std::env::var("BETTERTRICKS_CATALOG_PUBLIC_KEY").ok()?;
    let index_url = match std::env::var("BETTERTRICKS_CATALOG_INDEX")
        .ok()
        .and_then(|value| url::Url::parse(&value).ok())
    {
        Some(url) => url,
        None => {
            tracing::warn!(
                "catalog public key is set, but BETTERTRICKS_CATALOG_INDEX is missing or invalid"
            );
            return None;
        }
    };
    match CatalogUpdater::from_hex_key(paths.clone(), catalog.clone(), store.clone(), &key) {
        Ok(updater) => Some(CatalogUpdateConfig { updater, index_url }),
        Err(error) => {
            tracing::warn!(%error, "catalog update channel is invalid");
            None
        }
    }
}

fn catalog_path(app: &tauri::App) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("BETTERTRICKS_CATALOG") {
        return Ok(PathBuf::from(path));
    }
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../catalog");
    if development.is_dir() {
        return Ok(development);
    }
    let resource = app.path().resource_dir().map_err(command_error)?;
    for candidate in [resource.join("catalog"), resource.join("_up_/catalog")] {
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    let system_catalog = PathBuf::from("/usr/share/bettertricks/catalog");
    if system_catalog.is_dir() {
        return Ok(system_catalog);
    }
    Err("Bundled recipe catalog not found".into())
}

type CommandResult<T> = std::result::Result<T, String>;

fn command_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bettertricks=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = build_state(app)?;
            // A catalog the user rolled back to stays until they update explicitly.
            let automatic_update = state.store.settings()?.catalog_auto_update
                && !state.store.active_catalog_pinned()?;
            let update = state.catalog_updater.clone();
            let current_version = state.catalog.summary().version;
            app.manage(state);
            if automatic_update && let Some(config) = update {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let result = async {
                        let index = config.updater.fetch_index(&config.index_url).await?;
                        if let Some(release) = index.latest_after(&current_version).cloned() {
                            config.updater.install(&release).await?;
                            let _ = handle.emit("catalog-updated", release);
                        }
                        bettertricks_core::Result::Ok(())
                    }
                    .await;
                    if let Err(error) = result {
                        tracing::warn!(%error, "automatic catalog update failed");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            inspect_system,
            install_compatibility_host,
            list_prefixes,
            register_prefix,
            unregister_prefix,
            create_prefix,
            catalog_search,
            catalog_summary,
            get_recipe,
            import_manual_file,
            plan_operation,
            start_operation,
            cancel_operation,
            respond_to_prompt,
            active_operations,
            operation_history,
            clear_operation_history,
            get_settings,
            save_settings,
            cache_stats,
            clear_cache,
            list_restore_points,
            create_restore_point,
            delete_restore_point,
            clear_restore_points,
            restore_prefix,
            launch_prefix_tool,
            open_path,
            open_url,
            open_sponsor_page,
            trash_prefix,
            inspect_legacy_verb,
            run_legacy_verb,
            check_catalog_update,
            install_catalog_update,
            rollback_catalog_version,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Bettertricks")
        .run(|_, event| {
            // Tool windows and installers run in their own process groups; stop them with
            // the app instead of leaving them writing to an unlocked prefix.
            if let tauri::RunEvent::Exit = event {
                bettertricks_core::stop_all_groups();
            }
        });
}
