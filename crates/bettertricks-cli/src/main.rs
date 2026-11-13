use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bettertricks_core::{
    AppPaths, Catalog, CatalogQuery, CatalogUpdater, OperationEngine, OperationEvent,
    OperationEventSink, OperationOptions, OperationRequest, PrefixArchitecture, PromptResponse,
    Session, Store, VerbCategory, install_managed_compatibility_host, prefix_id_for_path,
    rollback_catalog, validate_new_prefix_path,
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
    let session = Session::open(paths, &locate_catalog()?)?;
    let Session {
        paths,
        store,
        catalog,
        discovery,
        planner,
        engine,
        ..
    } = session;
    let summary = catalog.summary();

    if cli.self_update {
        update_signed_catalog(&paths, &store, &catalog).await?;
    }
    if cli.update_rollback {
        let rolled_back = rollback_catalog(&catalog, &store)?;
        println!("Rolled back to catalog {}.", rolled_back.version);
    }
    if cli.install_compatibility_host {
        let path = install_managed_compatibility_host(&paths, &catalog).await?;
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
    if cli.commands.first().is_some_and(|command| command == "ops") && !catalog.contains("ops") {
        return operations_command(&cli, &store, &engine);
    }
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
    // arch= only chooses the architecture of a prefix that is about to be created.
    if let Some(architecture) = architecture
        && !prefix.exists
    {
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
        let host = bettertricks_core::LegacyVerbHost::for_catalog(&paths, &catalog);
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
        }
        for info in &inspected {
            engine.run_legacy_verb(info, &prefix, &options).await?;
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
    let plan = planner.plan(request, prefix)?;
    if cli.json || cli.dry_run {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        if cli.dry_run {
            return Ok(());
        }
    }
    if !plan.conflicts.is_empty() && !cli.force {
        return Err("the operation has conflicts; inspect --dry-run output or use --force".into());
    }

    let sink: Arc<dyn OperationEventSink> = Arc::new(ConsoleSink {
        json: cli.json,
        engine: engine.clone(),
    });
    let operation_id = plan.id;
    let interrupt = engine.clone();
    tokio::spawn(async move {
        // Ctrl-C cancels like the desktop's Cancel button: the process group is stopped and
        // the prefix's wineserver is killed before the lock is released.
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = interrupt.cancel(operation_id);
            eprintln!("Cancelling; press Ctrl-C again to force quit.");
        }
        // A second Ctrl-C force-quits even if cancellation is stuck in a non-cancellable
        // wait: kill every process group this CLI started, then exit like SIGINT would.
        if tokio::signal::ctrl_c().await.is_ok() {
            bettertricks_core::stop_all_groups();
            std::process::exit(130);
        }
    });
    engine.run(plan, sink).await?;
    Ok(())
}

/// `bettertricks ops list|clear [--json]`: the operation journal with lifecycle states, the
/// same records the desktop Activity view shows.
fn operations_command(
    cli: &Cli,
    store: &Store,
    engine: &OperationEngine,
) -> Result<(), Box<dyn std::error::Error>> {
    engine.reconcile_interrupted()?;
    match cli.commands.get(1).map(String::as_str).unwrap_or("list") {
        "list" => {
            let operations = store.operations(200)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&operations)?);
            } else {
                for operation in operations {
                    println!(
                        "{}  {:<16} {:<24} {}",
                        operation.id,
                        serde_json::to_value(operation.state)?
                            .as_str()
                            .unwrap_or_default(),
                        operation.prefix_name,
                        operation.recipes.join(" ")
                    );
                }
            }
        }
        "clear" => {
            let cleared = store.clear_operation_history()?;
            if cli.json {
                println!("{}", serde_json::json!({ "cleared": cleared }));
            } else {
                println!("Cleared {cleared} finished operation(s).");
            }
        }
        other => return Err(format!("unknown ops command {other:?}; use list or clear").into()),
    }
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
        if selector.is_empty() || selector.contains('/') || selector == "." || selector == ".." {
            return Err(format!(
                "prefix={selector:?} must be a prefix name under wineprefixes, not a path"
            )
            .into());
        }
        let xdg_data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| paths.data.clone());
        let path = validate_new_prefix_path(&xdg_data.join("wineprefixes").join(selector))?;
        if let Some(prefix) = prefixes.iter().find(|prefix| prefix.path == path) {
            return Ok(prefix.clone());
        }
        return Ok(bettertricks_core::WinePrefix {
            id: prefix_id_for_path(&path),
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
            status: bettertricks_core::PrefixStatus::Missing,
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
    engine: OperationEngine,
}

impl OperationEventSink for ConsoleSink {
    fn emit(&self, event: OperationEvent) {
        if let Some(prompt) = &event.prompt {
            answer_prompt(
                self.engine.clone(),
                event.operation_id,
                prompt.clone(),
                self.json,
            );
        }
        if self.json {
            // Every event carries the operation `state`, so agents can follow the lifecycle.
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

/// Reads one answer from stdin: a choice id (`continue`, `cancel`) or, for agents, a JSON
/// object `{"choice_id": "continue"}`. End of input cancels.
fn answer_prompt(
    engine: OperationEngine,
    operation_id: uuid::Uuid,
    prompt: bettertricks_core::OperationPrompt,
    json: bool,
) {
    if !json {
        eprintln!("{}: {}", prompt.title, prompt.message);
        let choices = prompt
            .choices
            .iter()
            .map(|choice| choice.id.as_str())
            .collect::<Vec<_>>();
        eprintln!("Answer with one of: {}", choices.join(", "));
    }
    std::thread::spawn(move || {
        let mut line = String::new();
        let choice = match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => "cancel".to_owned(),
            Ok(_) => parse_prompt_answer(&line),
        };
        let _ = engine.respond(PromptResponse {
            operation_id,
            prompt_id: prompt.id,
            choice_id: choice,
        });
    });
}

fn parse_prompt_answer(line: &str) -> String {
    let line = line.trim();
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("choice_id")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| line.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_selectors_cannot_escape_the_wineprefixes_directory() {
        let temp = tempfile_dir();
        let paths = AppPaths::isolated(&temp).unwrap();
        for selector in ["..", ".", "/home/user", "../outside", ""] {
            assert!(
                select_prefix(&paths, &mut [], Some(selector)).is_err(),
                "{selector}"
            );
        }
        let prefix = select_prefix(&paths, &mut [], Some("game")).unwrap();
        assert!(prefix.path.ends_with("wineprefixes/game"));
        assert_eq!(prefix.id, prefix_id_for_path(&prefix.path));
        assert_eq!(
            parse_prompt_answer("{\"choice_id\":\"continue\"}\n"),
            "continue"
        );
        assert_eq!(parse_prompt_answer(" cancel\n"), "cancel");
        let _ = std::fs::remove_dir_all(temp);
    }

    fn tempfile_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("bettertricks-cli-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

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
