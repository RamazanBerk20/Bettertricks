use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bettertricks_core::{
    AppPaths, Catalog, CatalogQuery, CatalogSource, CatalogUpdater, LegacyVerbHost,
    OperationEngine, OperationEvent, OperationEventSink, OperationOptions, OperationRequest,
    Planner, PrefixArchitecture, PrefixDiscovery, RecoveryManager, Store, VerbCategory,
    install_managed_compatibility_host, rollback_catalog,
};
use clap::{ArgAction, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "bettertricks",
    version,
    about = "A recovery-first Wine prefix and compatibility recipe manager",
    trailing_var_arg = true
)]
struct Cli {
    #[arg(long)]
    country: Option<String>,
    #[arg(short = 'f', long)]
    force: bool,
    #[arg(long)]
    gui: Option<Option<String>>,
    #[arg(long)]
    isolate: bool,
    #[arg(long)]
    no_clean: bool,
    #[arg(short = 'q', long)]
    unattended: bool,
    #[arg(short = 't', long)]
    torify: bool,
    #[arg(long)]
    verify: bool,
    #[arg(short = 'v', long, action = ArgAction::Count)]
    verbose: u8,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(
        long = "input",
        value_name = "RECIPE.INPUT=VALUE",
        help = "Supply a value required by a recipe; may be repeated"
    )]
    inputs: Vec<String>,
    #[arg(
        long = "manual-file",
        value_name = "RECIPE.FILE=PATH",
        help = "Checksum-import a required manual download; may be repeated"
    )]
    manual_files: Vec<String>,
    #[arg(
        long,
        help = "Explicitly allow execution of reviewed legacy .verb shell code"
    )]
    allow_legacy_verb: bool,
    #[arg(long)]
    self_update: bool,
    #[arg(long)]
    update_rollback: bool,
    #[arg(
        long,
        help = "Install the checksum-verified Winetricks host matching the active catalog"
    )]
    install_compatibility_host: bool,
    #[arg(long)]
    optin: bool,
    #[arg(long)]
    optout: bool,
    commands: Vec<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("bettertricks: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let filter = match cli.verbose {
        0 => "bettertricks=warn",
        1 => "bettertricks=info",
        _ => "bettertricks=debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    if cli.optin || cli.optout {
        println!("Bettertricks does not collect or transmit usage telemetry.");
        if cli.commands.is_empty()
            && !cli.self_update
            && !cli.update_rollback
            && !cli.install_compatibility_host
            && cli.gui.is_none()
        {
            return Ok(());
        }
    }
    if let Some(gui) = &cli.gui {
        if let Some(legacy) = gui {
            eprintln!(
                "--gui={legacy} is accepted for compatibility; Bettertricks uses its own desktop interface."
            );
        }
        launch_desktop()?;
        if cli.commands.is_empty() {
            return Ok(());
        }
    }

    let paths = AppPaths::discover()?;
    let store = Arc::new(Store::open(&paths)?);
    let bundled_catalog_root = locate_catalog()?;
    let active_catalog = store.active_catalog_version()?;
    let selected_catalog_root = active_catalog
        .as_ref()
        .map(|record| record.path.clone())
        .unwrap_or_else(|| bundled_catalog_root.clone());
    let (selected_version, selected_upstream) = catalog_identity(&selected_catalog_root);
    let loaded = Catalog::load(
        CatalogSource {
            path: selected_catalog_root.clone(),
            version: selected_version,
            upstream_tag: selected_upstream,
        },
        paths.winetricks_cache.clone(),
    );
    let (catalog, catalog_root, signature) = match loaded {
        Ok(catalog) => (
            catalog,
            selected_catalog_root,
            active_catalog.and_then(|record| record.signature),
        ),
        Err(error) if selected_catalog_root != bundled_catalog_root => {
            eprintln!("Ignoring invalid active catalog: {error}");
            let (version, upstream_tag) = catalog_identity(&bundled_catalog_root);
            (
                Catalog::load(
                    CatalogSource {
                        path: bundled_catalog_root.clone(),
                        version,
                        upstream_tag,
                    },
                    paths.winetricks_cache.clone(),
                )?,
                bundled_catalog_root,
                None,
            )
        }
        Err(error) => return Err(error.into()),
    };
    let summary = catalog.summary();
    store.activate_catalog_version(
        &summary.version,
        &summary.upstream_tag,
        &catalog_root,
        signature.as_deref(),
    )?;

    if cli.self_update {
        update_signed_catalog(&paths, &store, &catalog).await?;
    }
    if cli.update_rollback {
        let rolled_back = rollback_catalog(&catalog, &store)?;
        println!("Rolled back to catalog {}.", rolled_back.version);
    }
    if cli.install_compatibility_host {
        let path = install_managed_compatibility_host(&paths, &summary.upstream_tag).await?;
        println!(
            "Installed checksum-verified Winetricks {} host at {}.",
            summary.upstream_tag,
            path.display()
        );
    }
    let mut imported_manual_files = Vec::new();
    for specification in &cli.manual_files {
        let (recipe_id, file_id, path) = parse_manual_file_specification(specification)?;
        let imported = catalog.import_manual_file(recipe_id, file_id, path).await?;
        imported_manual_files.push(imported);
    }
    if !imported_manual_files.is_empty() {
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&imported_manual_files)?);
        } else {
            for path in &imported_manual_files {
                eprintln!("Checksum-verified manual file: {}", path.display());
            }
        }
    }
    if cli.commands.is_empty()
        && (cli.self_update || cli.update_rollback || cli.install_compatibility_host)
    {
        return Ok(());
    }
    if cli.commands.is_empty() && !imported_manual_files.is_empty() {
        return Ok(());
    }
    let discovery = PrefixDiscovery::new(store.clone());
    let mut prefixes = discovery.discover().await?;

    if cli.commands.is_empty() {
        print_categories(cli.json)?;
        return Ok(());
    }

    let mut commands = cli.commands.clone();
    let mut architecture = None;
    let mut prefix_selector = None;
    commands.retain(|argument| {
        if let Some(value) = argument.strip_prefix("arch=") {
            architecture = Some(if value == "32" {
                PrefixArchitecture::Win32
            } else {
                PrefixArchitecture::Win64
            });
            false
        } else if let Some(value) = argument.strip_prefix("prefix=") {
            prefix_selector = Some(value.to_string());
            false
        } else {
            true
        }
    });

    let mut prefix = select_prefix(&paths, &mut prefixes, prefix_selector.as_deref())?;
    if let Some(architecture) = architecture {
        prefix.architecture = architecture;
    }

    if handle_list_command(&cli, &catalog, &prefix, &commands)? {
        return Ok(());
    }

    if commands
        .first()
        .is_some_and(|command| command == "annihilate")
    {
        return Err(
            "prefix deletion is available in the desktop app with recoverable trash protection"
                .into(),
        );
    }

    let options = OperationOptions {
        force: cli.force,
        unattended: cli.unattended,
        verify: cli.verify,
        no_clean: cli.no_clean,
        isolate: cli.isolate,
        torify: cli.torify,
        country: cli.country.clone(),
        create_restore_point: false,
    };
    let legacy_verbs = commands
        .iter()
        .filter(|command| {
            Path::new(command)
                .extension()
                .and_then(|value| value.to_str())
                == Some("verb")
        })
        .cloned()
        .collect::<Vec<_>>();
    if !legacy_verbs.is_empty() {
        let host = LegacyVerbHost::discover_with_paths(&paths);
        let inspected = legacy_verbs
            .iter()
            .map(|path| host.inspect(Path::new(path)))
            .collect::<bettertricks_core::Result<Vec<_>>>()?;
        for info in &inspected {
            eprintln!("Warning for {}: {}", info.path.display(), info.warning);
        }
        if !cli.allow_legacy_verb {
            return Err(
                "legacy .verb files are shell programs; review them, then rerun with --allow-legacy-verb"
                    .into(),
            );
        }
        if prefix.exists {
            eprintln!("Creating a restore point before running legacy shell code...");
            RecoveryManager::new(paths.clone(), store.clone())
                .create(&prefix, None)
                .await?;
        }
        for info in &inspected {
            host.run(&info.path, &prefix, &options, true).await?;
        }
        commands.retain(|command| !legacy_verbs.contains(command));
        if commands.is_empty() {
            return Ok(());
        }
    }

    let recipes = commands
        .into_iter()
        .filter(|command| command != "list")
        .collect::<Vec<_>>();
    let request = OperationRequest {
        prefix_id: prefix.id,
        recipes,
        input_values: parse_input_values(&cli.inputs)?,
        options,
    };
    let plan = Planner::with_paths(catalog.clone(), &paths).plan(request, prefix)?;
    if cli.json || cli.dry_run {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        if cli.dry_run {
            return Ok(());
        }
    }
    if !plan.conflicts.is_empty() && !cli.force {
        return Err("the operation has conflicts; inspect --dry-run output or use --force".into());
    }

    let engine = OperationEngine::new(catalog, paths, store);
    let sink: Arc<dyn OperationEventSink> = Arc::new(ConsoleSink { json: cli.json });
    engine.run(plan, sink).await?;
    Ok(())
}

fn parse_input_values(
    values: &[String],
) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut parsed = BTreeMap::new();
    for value in values {
        let (key, value) = value
            .split_once('=')
            .ok_or_else(|| format!("invalid --input {value:?}; expected RECIPE.INPUT=VALUE"))?;
        if key.is_empty() || value.is_empty() {
            return Err(format!("invalid --input {value:?}; key and value are required").into());
        }
        if parsed.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!("duplicate --input key {key}").into());
        }
    }
    Ok(parsed)
}

fn parse_manual_file_specification(
    value: &str,
) -> Result<(&str, &str, &Path), Box<dyn std::error::Error>> {
    let (key, path) = value
        .split_once('=')
        .ok_or_else(|| format!("invalid --manual-file {value:?}; expected RECIPE.FILE=PATH"))?;
    let (recipe_id, file_id) = key
        .split_once('.')
        .ok_or_else(|| format!("invalid --manual-file {value:?}; expected RECIPE.FILE=PATH"))?;
    if recipe_id.is_empty() || file_id.is_empty() || path.is_empty() {
        return Err(format!(
            "invalid --manual-file {value:?}; recipe, file, and path are required"
        )
        .into());
    }
    Ok((recipe_id, file_id, Path::new(path)))
}

fn handle_list_command(
    cli: &Cli,
    catalog: &Catalog,
    prefix: &bettertricks_core::WinePrefix,
    commands: &[String],
) -> Result<bool, Box<dyn std::error::Error>> {
    let category = commands.first().and_then(|value| parse_category(value));
    let list_command = if category.is_some() {
        commands.get(1).map(String::as_str)
    } else {
        commands.first().map(String::as_str)
    };
    let Some(list_command) = list_command else {
        return Ok(false);
    };
    if list_command == "list" && category.is_none() {
        print_categories(cli.json)?;
        return Ok(true);
    }
    let mut query = CatalogQuery {
        category,
        ..Default::default()
    };
    match list_command {
        "list" | "list-all" => {}
        "list-cached" => query.cached_only = true,
        "list-download" => query.media = Some(bettertricks_core::MediaKind::Download),
        "list-manual-download" => query.media = Some(bettertricks_core::MediaKind::ManualDownload),
        "list-installed" => query.installed_only = true,
        _ => return Ok(false),
    }
    let recipes = catalog.search(&query, Some(prefix));
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&recipes)?);
    } else if list_command == "list-all" || category.is_some() {
        for recipe in recipes {
            let media = match recipe.media {
                bettertricks_core::MediaKind::Download => " [downloadable]",
                bettertricks_core::MediaKind::ManualDownload => " [manual download]",
                bettertricks_core::MediaKind::None => "",
            };
            let execution = match recipe.maturity {
                bettertricks_core::RecipeMaturity::Native => " [native]",
                bettertricks_core::RecipeMaturity::MetadataOnly => " [winetricks host]",
                bettertricks_core::RecipeMaturity::BrokenUpstream => " [broken upstream]",
            };
            println!("{:<24} {}{}{}", recipe.id, recipe.title, media, execution);
        }
    } else {
        for recipe in recipes {
            println!("{}", recipe.id);
        }
    }
    Ok(true)
}

fn print_categories(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let categories = ["apps", "benchmarks", "dlls", "fonts", "settings"];
    if json {
        println!("{}", serde_json::to_string_pretty(&categories)?);
    } else {
        for category in categories {
            println!("{category}");
        }
    }
    Ok(())
}

fn parse_category(value: &str) -> Option<VerbCategory> {
    match value {
        "apps" => Some(VerbCategory::Apps),
        "benchmarks" => Some(VerbCategory::Benchmarks),
        "dlls" => Some(VerbCategory::Dlls),
        "fonts" => Some(VerbCategory::Fonts),
        "settings" => Some(VerbCategory::Settings),
        _ => None,
    }
}

fn select_prefix(
    paths: &AppPaths,
    prefixes: &mut [bettertricks_core::WinePrefix],
    selector: Option<&str>,
) -> Result<bettertricks_core::WinePrefix, Box<dyn std::error::Error>> {
    if let Some(selector) = selector {
        let xdg_data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| paths.data.clone());
        let path = xdg_data.join("wineprefixes").join(selector);
        if let Some(prefix) = prefixes.iter().find(|prefix| prefix.path == path) {
            return Ok(prefix.clone());
        }
        return Ok(bettertricks_core::WinePrefix {
            id: uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_URL,
                path.to_string_lossy().as_bytes(),
            ),
            name: selector.into(),
            path,
            source: bettertricks_core::PrefixSource::WinePrefixes,
            architecture: PrefixArchitecture::Unknown,
            runtime: None,
            runtime_label: Some("System Wine".into()),
            managed: false,
            exists: false,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        });
    }
    prefixes
        .iter()
        .find(|prefix| prefix.source == bettertricks_core::PrefixSource::DefaultWine)
        .cloned()
        .or_else(|| prefixes.first().cloned())
        .ok_or_else(|| "no Wine prefix could be selected".into())
}

fn locate_catalog() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os("BETTERTRICKS_CATALOG") {
        return Ok(PathBuf::from(path));
    }
    let executable_relative = std::env::current_exe().ok().and_then(|executable| {
        executable
            .parent()?
            .parent()
            .map(|prefix| prefix.join("lib/Bettertricks/catalog"))
    });
    let candidates = [
        executable_relative,
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../catalog")),
        Some(PathBuf::from("/usr/lib/Bettertricks/catalog")),
        Some(PathBuf::from("/usr/share/bettertricks/catalog")),
        Some(std::env::current_dir()?.join("catalog")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.is_dir())
        .ok_or_else(|| "Bettertricks catalog not found; set BETTERTRICKS_CATALOG".into())
}

fn catalog_identity(path: &Path) -> (String, String) {
    let manifest = std::fs::read_to_string(path.join("manifest.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok());
    let version = manifest
        .as_ref()
        .and_then(|value| value.get("version"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("bundled")
        .to_owned();
    let upstream_tag = manifest
        .as_ref()
        .and_then(|value| value.get("upstreamTag"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    (version, upstream_tag)
}

async fn update_signed_catalog(
    paths: &AppPaths,
    store: &Arc<Store>,
    catalog: &Catalog,
) -> Result<(), Box<dyn std::error::Error>> {
    let public_key = match std::env::var("BETTERTRICKS_CATALOG_PUBLIC_KEY") {
        Ok(value) => value,
        Err(_) => {
            println!("No signed catalog update channel is configured.");
            println!("Package-managed binaries must be updated with the system package manager.");
            return Ok(());
        }
    };
    let index_url = std::env::var("BETTERTRICKS_CATALOG_INDEX")
        .map_err(|_| "BETTERTRICKS_CATALOG_INDEX is required when a catalog key is configured")?;
    let index_url = url::Url::parse(&index_url)?;
    let updater =
        CatalogUpdater::from_hex_key(paths.clone(), catalog.clone(), store.clone(), &public_key)?;
    let index = updater.fetch_index(&index_url).await?;
    let current = catalog.summary().version;
    if let Some(release) = index.latest_after(&current) {
        let installed = updater.install(release).await?;
        println!("Installed signed catalog {}.", installed.version);
    } else {
        println!("Catalog {current} is already current.");
    }
    println!("Package-managed binaries must be updated with the system package manager.");
    Ok(())
}

fn launch_desktop() -> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::current_exe()?
        .parent()
        .unwrap_or(Path::new("."))
        .join("bettertricks-desktop");
    std::process::Command::new(binary).spawn()?;
    Ok(())
}

struct ConsoleSink {
    json: bool,
}

impl OperationEventSink for ConsoleSink {
    fn emit(&self, event: OperationEvent) {
        if self.json {
            if let Ok(value) = serde_json::to_string(&event) {
                println!("{value}");
            }
        } else if let Some(line) = event.log_line {
            eprintln!("  {line}");
        } else {
            eprintln!("[{}/{}] {}", event.step, event.total_steps, event.title);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manual_file_specifications_without_truncating_paths() {
        let (recipe, file, path) =
            parse_manual_file_specification("installer.archive=/tmp/file=name.exe").unwrap();
        assert_eq!(recipe, "installer");
        assert_eq!(file, "archive");
        assert_eq!(path, Path::new("/tmp/file=name.exe"));
        assert!(parse_manual_file_specification("missing-separators").is_err());
        assert!(parse_manual_file_specification("recipe.=path").is_err());
    }
}
