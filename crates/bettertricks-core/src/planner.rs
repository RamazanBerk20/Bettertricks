use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::system::find_command;
use crate::{
    AppPaths, ArchiveFormat, BettertricksError, Catalog, DetectKind, HostStatus, LegacyVerbHost,
    OperationPlan, OperationRequest, PlanIssue, PlannedDownload, PlannedInput, PlannedStep,
    PrefixStatus, Recipe, RecipeMaturity, RecipeStep, Result, WinePrefix,
};

#[derive(Clone)]
pub struct Planner {
    catalog: Catalog,
    legacy_host: LegacyVerbHost,
}

impl Planner {
    /// A planner with no Winetricks host (tracked recipes always get the missing-host
    /// warning) that never executes anything; the app and CLI use [`Self::with_paths`].
    pub fn new(catalog: Catalog) -> Self {
        Self {
            catalog,
            legacy_host: LegacyVerbHost::none(),
        }
    }

    pub fn with_paths(catalog: Catalog, paths: &AppPaths) -> Self {
        Self {
            legacy_host: LegacyVerbHost::for_catalog(paths, &catalog),
            catalog,
        }
    }

    pub fn plan(&self, mut request: OperationRequest, prefix: WinePrefix) -> Result<OperationPlan> {
        if request.recipes.is_empty() {
            return Err(BettertricksError::Recipe(
                "select at least one recipe".into(),
            ));
        }

        // Normalise first (`--country us` worked in v1.0), then validate.
        if let Some(country) = &mut request.options.country {
            *country = country.trim().to_ascii_uppercase();
        }
        if let Some(country) = &request.options.country
            && (country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase()))
        {
            return Err(BettertricksError::Recipe(
                "country must be a two-letter ISO code such as US".into(),
            ));
        }

        let mut resolved = Vec::new();
        let mut permanent = HashSet::new();
        let mut visiting = HashSet::new();
        for recipe in &request.recipes {
            self.resolve(recipe, &mut visiting, &mut permanent, &mut resolved)?;
        }

        let installed: HashSet<_> = prefix.installed_verbs.iter().cloned().collect();
        let mut conflicts = Vec::new();
        let mut warnings = Vec::new();
        // Dependencies the user did not ask for are skipped when already present, unless forced.
        let already_installed = if request.options.force {
            Vec::new()
        } else {
            let mut skipped = Vec::new();
            for id in &resolved {
                if !request.recipes.contains(id)
                    && (installed.contains(id) || detected(&self.catalog.get(id)?, &prefix))
                {
                    skipped.push(id.clone());
                }
            }
            skipped
        };
        resolved.retain(|id| !already_installed.contains(id));
        if !already_installed.is_empty() {
            warnings.push(PlanIssue {
                code: "already_installed".into(),
                title: "Installed dependencies will be skipped".into(),
                message: format!(
                    "Already present in this prefix: {}. Enable force to reinstall them.",
                    already_installed.join(", ")
                ),
                recipe_id: None,
            });
        }
        let selected: HashSet<_> = resolved.iter().cloned().collect();
        if prefix.status == PrefixStatus::Busy {
            conflicts.push(PlanIssue {
                code: "prefix_busy".into(),
                title: "This prefix is busy".into(),
                message: "Another Bettertricks task is changing this prefix. Wait for it to finish, then review again.".into(),
                recipe_id: None,
            });
        }
        if prefix.managed && prefix.runtime.is_none() {
            conflicts.push(PlanIssue {
                code: "runtime_unknown".into(),
                title: "The launcher's Wine runtime was not found".into(),
                message: "Bettertricks could not determine which Proton or Wine build owns this prefix. Running system Wine against it can upgrade and break the prefix; start the game once from its launcher, or enable force to continue anyway.".into(),
                recipe_id: None,
            });
        }
        let mut steps = Vec::new();
        let mut downloads = Vec::new();
        let mut inputs = Vec::new();
        let mut known_input_keys = HashSet::new();
        let mut missing_tools: HashSet<&'static str> = HashSet::new();
        let mut compatibility_hosted = Vec::new();

        for recipe_id in &resolved {
            let recipe = self.catalog.get(recipe_id)?;
            for conflict in &recipe.conflicts {
                if installed.contains(conflict) || selected.contains(conflict) {
                    conflicts.push(PlanIssue {
                        code: "verb_conflict".into(),
                        title: format!("{} conflicts with {conflict}", recipe.title),
                        message: if installed.contains(conflict) {
                            format!("{conflict} is already recorded as installed in this prefix.")
                        } else {
                            "Both verbs are included in the requested operation.".into()
                        },
                        recipe_id: Some(recipe.id.clone()),
                    });
                }
            }
            if recipe.maturity == RecipeMaturity::MetadataOnly {
                compatibility_hosted.push((
                    recipe.id.clone(),
                    recipe.title.clone(),
                    recipe.source.upstream_tag.clone(),
                ));
                steps.push(PlannedStep {
                    recipe_id: recipe.id.clone(),
                    recipe_title: recipe.title.clone(),
                    step_index: 0,
                    label: format!("Run {} through Winetricks", recipe.id),
                    destructive: true,
                });
                if !missing_tools.contains("winetricks")
                    && !matches!(
                        self.legacy_host.status(&recipe.source.upstream_tag),
                        HostStatus::Ready { .. }
                    )
                {
                    missing_tools.insert("winetricks");
                    warnings.push(PlanIssue {
                        code: "missing_tool".into(),
                        title: "Winetricks is required".into(),
                        message: format!("Install the checksum-verified Winetricks {} compatibility host from Bettertricks settings before starting. No prefix changes will be made while it is missing.", recipe.source.upstream_tag),
                        recipe_id: Some(recipe.id.clone()),
                    });
                }
            }
            if let Some(reason) = &recipe.constraints.broken_reason {
                warnings.push(PlanIssue {
                    code: "upstream_broken".into(),
                    title: format!("{} is marked broken upstream", recipe.title),
                    message: reason.clone(),
                    recipe_id: Some(recipe.id.clone()),
                });
            }

            for file in &recipe.files {
                downloads.push(PlannedDownload {
                    recipe_id: recipe.id.clone(),
                    file_id: file.id.clone(),
                    filename: file.filename.clone(),
                    urls: file.urls.clone(),
                    cached: self.catalog.cache_path(&recipe.id, file).is_file(),
                    manual: file.manual,
                    cache_state: self.catalog.cache_state(&recipe.id, file),
                });
            }
            for input in &recipe.inputs {
                let key = format!("{}.{}", recipe.id, input.id);
                known_input_keys.insert(key.clone());
                let value = request.input_values.get(&key).cloned().or_else(|| {
                    input
                        .environment
                        .as_deref()
                        .and_then(|name| std::env::var(name).ok())
                });
                if let Some(value) = &value {
                    validate_input_value(&key, value)?;
                }
                inputs.push(PlannedInput {
                    key,
                    recipe_id: recipe.id.clone(),
                    id: input.id.clone(),
                    label: input.label.clone(),
                    description: input.description.clone(),
                    placeholder: input.placeholder.clone(),
                    required: input.required,
                    value,
                });
            }

            append_steps(&recipe, &mut steps);
            for step in recipe.steps.iter().chain(&recipe.verify) {
                for tool in step_required_tools(step) {
                    if find_command(tool).is_none() && missing_tools.insert(tool) {
                        warnings.push(PlanIssue {
                            code: "missing_tool".into(),
                            title: format!("{} is required", tool),
                            message: format!(
                                "Install {tool} before starting this operation. No prefix changes will be made while it is missing."
                            ),
                            recipe_id: Some(recipe.id.clone()),
                        });
                    }
                }
            }
        }

        if let Some((recipe_id, title, upstream_tag)) = compatibility_hosted.first() {
            let count = compatibility_hosted.len();
            warnings.insert(
                0,
                PlanIssue {
                    code: "winetricks_compatibility_host".into(),
                    title: if count == 1 {
                        format!("{title} uses the Winetricks compatibility host")
                    } else {
                        format!("{count} selected recipes use the Winetricks compatibility host")
                    },
                    message: if count == 1 {
                        format!("This tracked recipe runs through checksum-verified Winetricks {upstream_tag}. Bettertricks still locks the prefix, records activity, and offers a restore point; it is not counted as a native port.")
                    } else {
                        format!("These tracked recipes run through checksum-verified Winetricks {upstream_tag}. Bettertricks still locks the prefix, records activity, and offers a restore point; they are not counted as native ports.")
                    },
                    recipe_id: (count == 1).then(|| recipe_id.clone()),
                },
            );
        }

        if let Some(unknown) = request
            .input_values
            .keys()
            .find(|key| !known_input_keys.contains(*key))
        {
            return Err(BettertricksError::Recipe(format!(
                "unknown operation input {unknown}"
            )));
        }

        if request.options.create_restore_point {
            for tool in ["tar", "zstd"] {
                if find_command(tool).is_none() && missing_tools.insert(tool) {
                    warnings.push(PlanIssue {
                        code: "missing_tool".into(),
                        title: format!("{} is required", tool),
                        message: format!(
                            "Install {tool} before creating a restore point. No prefix changes will be made while it is missing."
                        ),
                        recipe_id: None,
                    });
                }
            }
        }

        if prefix.managed {
            warnings.push(PlanIssue {
                code: "managed_prefix".into(),
                title: "This prefix belongs to another launcher".into(),
                message: "Close the game and its launcher before making changes. Bettertricks will not modify launcher configuration.".into(),
                recipe_id: None,
            });
        }
        if !prefix.exists {
            warnings.push(PlanIssue {
                code: "prefix_creation".into(),
                title: "The prefix will be created".into(),
                message: "Wine will initialize the selected path before recipes run.".into(),
                recipe_id: None,
            });
        }

        let has_destructive_steps = steps.iter().any(|step| step.destructive);
        Ok(OperationPlan {
            id: Uuid::new_v4(),
            prefix: prefix.clone(),
            requested_recipes: request.recipes,
            resolved_recipes: resolved,
            steps,
            inputs,
            downloads,
            conflicts,
            warnings,
            restore_recommended: prefix.managed || request.options.force || has_destructive_steps,
            estimated_download_bytes: None,
            options: request.options,
        })
    }

    fn resolve(
        &self,
        id: &str,
        visiting: &mut HashSet<String>,
        permanent: &mut HashSet<String>,
        output: &mut Vec<String>,
    ) -> Result<()> {
        if permanent.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.to_string()) {
            return Err(BettertricksError::Recipe(format!(
                "dependency cycle includes {id}"
            )));
        }
        let recipe = self.catalog.get(id)?;
        for dependency in &recipe.dependencies {
            self.resolve(dependency, visiting, permanent, output)?;
        }
        for called_recipe in called_recipes(&recipe.steps) {
            self.resolve(called_recipe, visiting, permanent, output)?;
        }
        visiting.remove(id);
        permanent.insert(id.to_string());
        output.push(id.to_string());
        Ok(())
    }
}

pub fn step_required_tools(step: &RecipeStep) -> Vec<&'static str> {
    match step {
        RecipeStep::Extract { format, .. } | RecipeStep::ExtractPath { format, .. } => match format
        {
            ArchiveFormat::Zip => vec!["unzip"],
            ArchiveFormat::SevenZip => vec!["7z"],
            ArchiveFormat::Cabinet => vec!["cabextract"],
            ArchiveFormat::Tar => vec!["tar"],
            ArchiveFormat::TarGz => vec!["tar", "gzip"],
            ArchiveFormat::TarXz => vec!["tar", "xz"],
        },
        RecipeStep::On64BitPrefix { steps } => {
            let mut tools = steps
                .iter()
                .flat_map(step_required_tools)
                .collect::<Vec<_>>();
            tools.sort_unstable();
            tools.dedup();
            tools
        }
        _ => Vec::new(),
    }
}

fn called_recipes(steps: &[RecipeStep]) -> Vec<&str> {
    steps
        .iter()
        .flat_map(|step| match step {
            RecipeStep::Call { recipe } => vec![recipe.as_str()],
            RecipeStep::On64BitPrefix { steps } => called_recipes(steps),
            _ => Vec::new(),
        })
        .collect()
}

pub(crate) fn recipe_dependency_ids(recipe: &Recipe) -> Vec<&str> {
    let mut dependencies = recipe
        .dependencies
        .iter()
        .map(String::as_str)
        .chain(called_recipes(&recipe.steps))
        .collect::<Vec<_>>();
    dependencies.sort_unstable();
    dependencies.dedup();
    dependencies
}

/// A recipe counts as present when it declares detection rules and every one matches.
fn detected(recipe: &Recipe, prefix: &WinePrefix) -> bool {
    !recipe.detect.is_empty()
        && recipe.detect.iter().all(|rule| {
            let mut path = rule.path.clone();
            for (marker, replacement) in [
                ("${prefix}", prefix.path.clone()),
                ("${drive_c}", prefix.path.join("drive_c")),
                ("${windows}", prefix.path.join("drive_c/windows")),
                ("${system32}", prefix.path.join("drive_c/windows/system32")),
                ("${syswow64}", prefix.path.join("drive_c/windows/syswow64")),
            ] {
                path = path.replace(marker, replacement.to_string_lossy().as_ref());
            }
            if path.contains("${") {
                return false;
            }
            let path = std::path::Path::new(&path);
            match rule.kind {
                DetectKind::File => path.is_file(),
                DetectKind::Directory => path.is_dir(),
            }
        })
}

fn validate_input_value(key: &str, value: &str) -> Result<()> {
    if value.len() > 8192
        || value
            .chars()
            .any(|value| matches!(value, '\0' | '\r' | '\n'))
    {
        return Err(BettertricksError::Recipe(format!(
            "operation input {key} contains unsupported data"
        )));
    }
    Ok(())
}

fn append_steps(recipe: &Recipe, output: &mut Vec<PlannedStep>) {
    for (index, step) in recipe.steps.iter().enumerate() {
        output.push(PlannedStep {
            recipe_id: recipe.id.clone(),
            recipe_title: recipe.title.clone(),
            step_index: index,
            label: step_label(step),
            destructive: step_is_destructive(step),
        });
    }
}

pub fn step_label(step: &RecipeStep) -> String {
    match step {
        RecipeStep::Download { file } => format!("Download {file}"),
        RecipeStep::EnsureDirectory { path } => format!("Prepare {path}"),
        RecipeStep::EnsureFile { path } => format!("Ensure {path} exists"),
        RecipeStep::VerifyPath { path, .. } => format!("Verify {path}"),
        RecipeStep::Copy { to, .. } => format!("Copy files to {to}"),
        RecipeStep::Move { to, .. } => format!("Move files to {to}"),
        RecipeStep::Remove { path, .. } => format!("Remove {path}"),
        RecipeStep::RemoveSymlink { path } => format!("Remove symlink {path}"),
        RecipeStep::Extract {
            destination,
            format,
            ..
        }
        | RecipeStep::ExtractPath {
            destination,
            format,
            ..
        } => {
            let kind = match format {
                ArchiveFormat::Zip => "ZIP",
                ArchiveFormat::SevenZip => "7-Zip",
                ArchiveFormat::Cabinet => "cabinet",
                ArchiveFormat::Tar => "tar",
                ArchiveFormat::TarGz => "tar.gz",
                ArchiveFormat::TarXz => "tar.xz",
            };
            format!("Extract {kind} to {destination}")
        }
        RecipeStep::InstallFonts { fonts, .. } => {
            format!("Install and register {} font file(s)", fonts.len())
        }
        RecipeStep::FontReplacements { replacements } => {
            format!("Configure {} font replacement(s)", replacements.len())
        }
        RecipeStep::On64BitPrefix { steps } => {
            format!("Apply {} 64-bit prefix step(s)", steps.len())
        }
        RecipeStep::Wine { program, .. } => format!("Run {program} with Wine"),
        RecipeStep::Registry { .. } => "Update the Wine registry".into(),
        RecipeStep::DllOverride { libraries, .. } => {
            format!("Configure {} DLL override(s)", libraries.len())
        }
        RecipeStep::WindowsVersion { version, .. } => format!("Set Windows version to {version}"),
        RecipeStep::Call { recipe } => format!("Apply dependency {recipe}"),
        RecipeStep::Notice { title, .. } => title.clone(),
        RecipeStep::Prompt { title, .. } => title.clone(),
        RecipeStep::NativeAction { action, .. } => action.as_str().replace('_', " "),
    }
}

fn step_is_destructive(step: &RecipeStep) -> bool {
    matches!(
        step,
        RecipeStep::Remove { .. }
            | RecipeStep::RemoveSymlink { .. }
            | RecipeStep::NativeAction {
                action: crate::NativeAction::IsolateHome | crate::NativeAction::RemoveMono,
                ..
            }
    ) || matches!(step, RecipeStep::On64BitPrefix { steps } if steps.iter().any(step_is_destructive))
}

pub fn plan_recipe_map(catalog: &Catalog, plan: &OperationPlan) -> Result<HashMap<String, Recipe>> {
    plan.resolved_recipes
        .iter()
        .map(|id| catalog.get(id).map(|recipe| (id.clone(), recipe)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_steps_declare_their_external_tools() {
        let step = RecipeStep::ExtractPath {
            source: "${temp}/outer.cab".into(),
            destination: "${temp}/nested".into(),
            format: ArchiveFormat::Cabinet,
            include: vec!["font.ttf".into()],
        };
        assert_eq!(step_required_tools(&step), vec!["cabextract"]);

        let step = RecipeStep::Extract {
            file: "archive".into(),
            destination: "${temp}/fonts".into(),
            format: ArchiveFormat::TarXz,
            include: Vec::new(),
        };
        assert_eq!(step_required_tools(&step), vec!["tar", "xz"]);

        let step = RecipeStep::On64BitPrefix {
            steps: vec![
                RecipeStep::Extract {
                    file: "archive".into(),
                    destination: "${temp}/first".into(),
                    format: ArchiveFormat::Zip,
                    include: Vec::new(),
                },
                RecipeStep::ExtractPath {
                    source: "${temp}/nested.cab".into(),
                    destination: "${temp}/second".into(),
                    format: ArchiveFormat::Cabinet,
                    include: Vec::new(),
                },
            ],
        };
        assert_eq!(step_required_tools(&step), vec!["cabextract", "unzip"]);
    }
    use crate::{CatalogSource, PrefixArchitecture, PrefixSource};

    #[test]
    fn dependencies_are_before_dependants() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("base.toml"),
            r#"
schema = 1
id = "base"
category = "dlls"
title = "Base"
maturity = "native"

[[steps]]
type = "native_action"
action = "noop"

[source]
upstream_tag = "test"
upstream_verb = "base"
"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("top.toml"),
            r#"
schema = 1
id = "top"
category = "dlls"
title = "Top"
maturity = "native"
dependencies = ["base"]

[[steps]]
type = "native_action"
action = "noop"

[source]
upstream_tag = "test"
upstream_verb = "top"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(
            CatalogSource {
                path: temp.path().into(),
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            temp.path().join("cache"),
        )
        .unwrap();
        let planner = Planner::new(catalog);
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: temp.path().join("prefix"),
            source: PrefixSource::Manual,
            architecture: PrefixArchitecture::Win64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: false,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
            status: Default::default(),
        };
        let plan = planner
            .plan(
                OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["top".into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap();
        assert_eq!(plan.resolved_recipes, ["base", "top"]);
    }

    #[test]
    fn skips_installed_dependencies_unless_forced_and_validates_country() {
        let temp = tempfile::tempdir().unwrap();
        for (id, dependencies) in [("base", ""), ("top", "dependencies = [\"base\"]")] {
            std::fs::write(
                temp.path().join(format!("{id}.toml")),
                format!("schema=1\nid=\"{id}\"\ncategory=\"dlls\"\ntitle=\"{id}\"\nmaturity=\"native\"\n{dependencies}\n[[steps]]\ntype=\"native_action\"\naction=\"noop\"\n[source]\nupstream_tag=\"test\"\nupstream_verb=\"{id}\"\n"),
            )
            .unwrap();
        }
        let catalog = Catalog::load(
            CatalogSource {
                path: temp.path().into(),
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            temp.path().join("cache"),
        )
        .unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: temp.path().join("prefix"),
            source: PrefixSource::Manual,
            architecture: PrefixArchitecture::Win64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: vec!["base".into()],
            size_bytes: None,
            last_modified: None,
            status: Default::default(),
        };
        let request = |force: bool, country: Option<&str>| OperationRequest {
            prefix_id: prefix.id,
            recipes: vec!["top".into()],
            input_values: Default::default(),
            options: crate::OperationOptions {
                force,
                country: country.map(str::to_owned),
                ..Default::default()
            },
        };
        let planner = Planner::new(catalog);
        let plan = planner.plan(request(false, None), prefix.clone()).unwrap();
        assert_eq!(plan.resolved_recipes, ["top"]);
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.code == "already_installed")
        );
        let forced = planner
            .plan(request(true, Some("us")), prefix.clone())
            .unwrap();
        assert_eq!(forced.options.country.as_deref(), Some("US"));
        assert_eq!(forced.resolved_recipes, ["base", "top"]);
        assert!(
            planner
                .plan(request(false, Some("us; rm -rf")), prefix)
                .is_err()
        );
    }

    #[test]
    fn summarizes_multiple_compatibility_host_warnings_once() {
        let temp = tempfile::tempdir().unwrap();
        for (id, title) in [("first", "First"), ("second", "Second")] {
            std::fs::write(
                temp.path().join(format!("{id}.toml")),
                format!(
                    "schema=1\nid=\"{id}\"\ncategory=\"dlls\"\ntitle=\"{title}\"\nmaturity=\"metadata_only\"\n[source]\nupstream_tag=\"test\"\nupstream_verb=\"{id}\"\n"
                ),
            )
            .unwrap();
        }
        let catalog = Catalog::load(
            CatalogSource {
                path: temp.path().into(),
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            temp.path().join("cache"),
        )
        .unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: temp.path().join("prefix"),
            source: PrefixSource::Manual,
            architecture: PrefixArchitecture::Win64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: false,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
            status: Default::default(),
        };

        let plan = Planner::new(catalog)
            .plan(
                OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["first".into(), "second".into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap();
        let compatibility_warnings: Vec<_> = plan
            .warnings
            .iter()
            .filter(|warning| warning.code == "winetricks_compatibility_host")
            .collect();

        assert_eq!(compatibility_warnings.len(), 1);
        assert_eq!(
            compatibility_warnings[0].title,
            "2 selected recipes use the Winetricks compatibility host"
        );
        assert_eq!(compatibility_warnings[0].recipe_id, None);
    }

    #[test]
    fn call_steps_are_resolved_as_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("base.toml"),
            "schema=1\nid=\"base\"\ncategory=\"dlls\"\ntitle=\"Base\"\nmaturity=\"native\"\n[[steps]]\ntype=\"native_action\"\naction=\"noop\"\n[source]\nupstream_tag=\"test\"\nupstream_verb=\"base\"\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("top.toml"),
            r#"
schema = 1
id = "top"
category = "dlls"
title = "Top"
maturity = "native"

[[steps]]
type = "call"
recipe = "base"

[source]
upstream_tag = "test"
upstream_verb = "top"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(
            CatalogSource {
                path: temp.path().into(),
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            temp.path().join("cache"),
        )
        .unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: temp.path().join("prefix"),
            source: PrefixSource::Manual,
            architecture: PrefixArchitecture::Win64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: false,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
            status: Default::default(),
        };
        let plan = Planner::new(catalog)
            .plan(
                OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["top".into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap();

        assert_eq!(plan.resolved_recipes, ["base", "top"]);
    }

    #[test]
    fn carries_only_declared_recipe_inputs_into_the_plan() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("midi.toml"),
            r#"
schema = 1
id = "midi"
category = "settings"
title = "MIDI"
maturity = "native"

[[inputs]]
id = "device"
label = "MIDI device"
environment = "BETTERTRICKS_TEST_UNUSED"

[[steps]]
type = "native_action"
action = "set_midi_device"

[source]
upstream_tag = "test"
upstream_verb = "midi"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(
            CatalogSource {
                path: temp.path().into(),
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            temp.path().join("cache"),
        )
        .unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: temp.path().join("prefix"),
            source: PrefixSource::Manual,
            architecture: PrefixArchitecture::Win64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: false,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
            status: Default::default(),
        };
        let input_values = [("midi.device".into(), "FluidSynth".into())]
            .into_iter()
            .collect();
        let plan = Planner::new(catalog.clone())
            .plan(
                OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["midi".into()],
                    input_values,
                    options: Default::default(),
                },
                prefix.clone(),
            )
            .unwrap();
        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(plan.inputs[0].key, "midi.device");
        assert_eq!(plan.inputs[0].value.as_deref(), Some("FluidSynth"));

        let error = Planner::new(catalog)
            .plan(
                OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["midi".into()],
                    input_values: [("midi.unknown".into(), "value".into())]
                        .into_iter()
                        .collect(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap_err();
        assert!(error.to_string().contains("unknown operation input"));
    }
}
