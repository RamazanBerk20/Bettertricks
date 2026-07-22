use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const RECIPE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerbCategory {
    Apps,
    Benchmarks,
    Dlls,
    Fonts,
    Settings,
}

impl VerbCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apps => "apps",
            Self::Benchmarks => "benchmarks",
            Self::Dlls => "dlls",
            Self::Fonts => "fonts",
            Self::Settings => "settings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    #[default]
    None,
    Download,
    ManualDownload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecipeMaturity {
    Native,
    #[default]
    MetadataOnly,
    BrokenUpstream,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecipeSource {
    pub upstream_tag: String,
    pub upstream_commit: Option<String>,
    pub upstream_verb: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecipeConstraints {
    pub architectures: Vec<PrefixArchitecture>,
    pub min_wine: Option<String>,
    pub max_wine: Option<String>,
    pub new_wow64_supported: Option<bool>,
    pub broken_reason: Option<String>,
    pub bug_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeFile {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub cache_path: Option<String>,
    #[serde(default)]
    pub urls: Vec<String>,
    pub sha256: Option<String>,
    #[serde(default)]
    pub manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeInput {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectRule {
    pub path: String,
    #[serde(default)]
    pub kind: DetectKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeFont {
    pub source: String,
    pub filename: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeFontReplacement {
    pub alias: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DetectKind {
    #[default]
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub schema: u32,
    pub id: String,
    pub category: VerbCategory,
    pub title: String,
    pub publisher: Option<String>,
    pub year: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub media: MediaKind,
    #[serde(default)]
    pub maturity: RecipeMaturity,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub constraints: RecipeConstraints,
    #[serde(default)]
    pub files: Vec<RecipeFile>,
    #[serde(default)]
    pub inputs: Vec<RecipeInput>,
    #[serde(default)]
    pub detect: Vec<DetectRule>,
    #[serde(default)]
    pub steps: Vec<RecipeStep>,
    #[serde(default)]
    pub verify: Vec<RecipeStep>,
    #[serde(default)]
    pub source: RecipeSource,
}

impl Recipe {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.schema != RECIPE_SCHEMA_VERSION {
            return Err(format!(
                "recipe {} uses schema {}, expected {}",
                self.id, self.schema, RECIPE_SCHEMA_VERSION
            ));
        }
        if self.id.is_empty()
            || !self.id.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '_' | '=')
            })
        {
            return Err(format!("recipe id {:?} is invalid", self.id));
        }
        if self.title.trim().is_empty() {
            return Err(format!("recipe {} has no title", self.id));
        }
        if self.source.upstream_tag.trim().is_empty() || self.source.upstream_verb.trim().is_empty()
        {
            return Err(format!(
                "recipe {} has incomplete upstream provenance",
                self.id
            ));
        }
        if self.maturity == RecipeMaturity::Native && self.steps.is_empty() {
            return Err(format!("native recipe {} has no executable steps", self.id));
        }
        if self.maturity != RecipeMaturity::Native
            && (!self.inputs.is_empty() || !self.steps.is_empty() || !self.verify.is_empty())
        {
            return Err(format!(
                "non-native recipe {} cannot contain executable steps",
                self.id
            ));
        }

        let mut file_ids = HashSet::new();
        let mut filenames = HashSet::new();
        for file in &self.files {
            if file.id.is_empty()
                || !file.id.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
                })
            {
                return Err(format!("recipe {} has an invalid file id", self.id));
            }
            if !file_ids.insert(file.id.as_str()) {
                return Err(format!("recipe {} has duplicate file ids", self.id));
            }
            if !filenames.insert(file.filename.as_str()) {
                return Err(format!(
                    "recipe {} has duplicate download filenames",
                    self.id
                ));
            }
            if let Some(cache_path) = &file.cache_path {
                let cache_path = Path::new(cache_path);
                if cache_path.is_absolute()
                    || cache_path
                        .components()
                        .any(|component| !matches!(component, Component::Normal(_)))
                    || cache_path.file_name().and_then(|name| name.to_str())
                        != Some(file.filename.as_str())
                {
                    return Err(format!("recipe {} has an unsafe file cache path", self.id));
                }
            }
            let mut filename_components = Path::new(&file.filename).components();
            if file.filename.contains('\\')
                || !matches!(filename_components.next(), Some(Component::Normal(_)))
                || filename_components.next().is_some()
            {
                return Err(format!(
                    "recipe {} has an unsafe download filename",
                    self.id
                ));
            }
            if let Some(hash) = &file.sha256
                && (hash.len() != 64
                    || !hash.chars().all(|character| character.is_ascii_hexdigit()))
            {
                return Err(format!("recipe {} has an invalid SHA-256", self.id));
            }
            if self.maturity == RecipeMaturity::Native {
                if file.sha256.is_none() {
                    return Err(format!(
                        "native recipe {} has a file without SHA-256",
                        self.id
                    ));
                }
                if !file.manual && file.urls.is_empty() {
                    return Err(format!(
                        "native recipe {} has an automated download without URLs",
                        self.id
                    ));
                }
                for value in &file.urls {
                    let url = url::Url::parse(value)
                        .map_err(|_| format!("recipe {} has an invalid download URL", self.id))?;
                    if url.scheme() != "https" {
                        return Err(format!(
                            "native recipe {} has a non-HTTPS automated download",
                            self.id
                        ));
                    }
                }
            }
        }

        let mut input_ids = HashSet::new();
        for input in &self.inputs {
            if input.id.is_empty()
                || !input.id.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || matches!(character, '_' | '-')
                })
                || !input_ids.insert(input.id.as_str())
            {
                return Err(format!(
                    "recipe {} has invalid or duplicate input ids",
                    self.id
                ));
            }
            if input.label.trim().is_empty() {
                return Err(format!("recipe {} has an input without a label", self.id));
            }
            if input.environment.as_deref().is_some_and(|name| {
                name.is_empty()
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            }) {
                return Err(format!(
                    "recipe {} has an invalid input environment variable",
                    self.id
                ));
            }
        }

        let mut pending_steps = self.steps.iter().chain(&self.verify).collect::<Vec<_>>();
        while let Some(step) = pending_steps.pop() {
            let referenced_file = match step {
                RecipeStep::Download { file } | RecipeStep::Extract { file, .. } => Some(file),
                _ => None,
            };
            if let Some(file) = referenced_file
                && !file_ids.contains(file.as_str())
            {
                return Err(format!("recipe {} references missing file {file}", self.id));
            }
            match step {
                RecipeStep::EnsureDirectory { path }
                | RecipeStep::EnsureFile { path }
                | RecipeStep::VerifyPath { path, .. }
                | RecipeStep::RemoveSymlink { path }
                | RecipeStep::Remove { path, .. }
                    if path.trim().is_empty() =>
                {
                    return Err(format!("recipe {} has an empty mutation path", self.id));
                }
                RecipeStep::Copy { from, to } | RecipeStep::Move { from, to }
                    if from.trim().is_empty() || to.trim().is_empty() =>
                {
                    return Err(format!("recipe {} has an empty mutation path", self.id));
                }
                RecipeStep::Extract {
                    destination,
                    include,
                    ..
                } => {
                    validate_extraction_paths(&self.id, destination, include)?;
                }
                RecipeStep::ExtractPath {
                    source,
                    destination,
                    include,
                    ..
                } => {
                    if source.trim().is_empty() {
                        return Err(format!("recipe {} has an empty extraction source", self.id));
                    }
                    validate_extraction_paths(&self.id, destination, include)?;
                }
                RecipeStep::Registry { content, .. } if content.trim().is_empty() => {
                    return Err(format!("recipe {} has an empty registry update", self.id));
                }
                RecipeStep::Notice { title, message, .. }
                    if title.trim().is_empty() || message.trim().is_empty() =>
                {
                    return Err(format!("recipe {} has an empty notice", self.id));
                }
                RecipeStep::DllOverride {
                    libraries,
                    application,
                    ..
                } => {
                    if libraries.is_empty() {
                        return Err(format!("recipe {} has no DLL override targets", self.id));
                    }
                    if libraries
                        .iter()
                        .any(|library| !is_safe_registry_segment(library))
                    {
                        return Err(format!(
                            "recipe {} has an unsafe DLL override target",
                            self.id
                        ));
                    }
                    if application
                        .as_deref()
                        .is_some_and(|application| !is_safe_registry_segment(application))
                    {
                        return Err(format!(
                            "recipe {} has an unsafe DLL override application",
                            self.id
                        ));
                    }
                }
                RecipeStep::InstallFonts { fonts, .. } => {
                    if fonts.is_empty() {
                        return Err(format!("recipe {} has no fonts to install", self.id));
                    }
                    let mut destinations = BTreeMap::new();
                    let mut registrations = HashSet::new();
                    for font in fonts {
                        let valid_name = |value: &str| {
                            !value.is_empty()
                                && Path::new(value).file_name().and_then(|name| name.to_str())
                                    == Some(value)
                                && !value.contains('\\')
                        };
                        let destination = font.filename.to_ascii_lowercase();
                        let source = font.source.to_ascii_lowercase();
                        let mismatched_source = destinations
                            .insert(destination.clone(), source.clone())
                            .is_some_and(|existing| existing != source);
                        if !valid_name(&font.source)
                            || !valid_name(&font.filename)
                            || font.display_name.trim().is_empty()
                            || mismatched_source
                            || !registrations.insert((destination, font.display_name.clone()))
                        {
                            return Err(format!(
                                "recipe {} has an invalid font definition",
                                self.id
                            ));
                        }
                    }
                }
                RecipeStep::FontReplacements { replacements } => {
                    if replacements.is_empty() {
                        return Err(format!("recipe {} has no font replacements", self.id));
                    }
                    let mut aliases = HashSet::new();
                    for replacement in replacements {
                        let valid = |value: &str| {
                            !value.trim().is_empty()
                                && !value
                                    .chars()
                                    .any(|value| matches!(value, '\0' | '\r' | '\n'))
                        };
                        if !valid(&replacement.alias)
                            || !valid(&replacement.replacement)
                            || !aliases.insert(replacement.alias.to_lowercase())
                        {
                            return Err(format!(
                                "recipe {} has an invalid font replacement",
                                self.id
                            ));
                        }
                    }
                }
                RecipeStep::On64BitPrefix { steps } => {
                    if steps.is_empty() {
                        return Err(format!(
                            "recipe {} has an empty 64-bit conditional",
                            self.id
                        ));
                    }
                    pending_steps.extend(steps);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn is_safe_registry_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_extraction_paths(
    recipe_id: &str,
    destination: &str,
    include: &[String],
) -> std::result::Result<(), String> {
    if destination.trim().is_empty() {
        return Err(format!(
            "recipe {recipe_id} has an empty extraction destination"
        ));
    }
    if include.iter().any(|pattern| {
        pattern.trim().is_empty()
            || pattern.contains('\\')
            || Path::new(pattern)
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
    }) {
        return Err(format!(
            "recipe {recipe_id} has an unsafe extraction pattern"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod recipe_tests {
    use super::*;

    fn recipe_with_filename(filename: &str) -> Recipe {
        Recipe {
            schema: RECIPE_SCHEMA_VERSION,
            id: "safe".into(),
            category: VerbCategory::Dlls,
            title: "Safe".into(),
            publisher: None,
            year: None,
            description: None,
            media: MediaKind::Download,
            maturity: RecipeMaturity::Native,
            tags: Vec::new(),
            dependencies: Vec::new(),
            conflicts: Vec::new(),
            constraints: RecipeConstraints::default(),
            files: vec![RecipeFile {
                id: "installer".into(),
                filename: filename.into(),
                cache_path: None,
                urls: Vec::new(),
                sha256: Some(
                    "93413c6ce1e4603c4adaa057e4ad0ce316e72589ba485ecc573e4a1331706139".into(),
                ),
                manual: true,
            }],
            inputs: Vec::new(),
            detect: Vec::new(),
            steps: vec![RecipeStep::NativeAction {
                action: NativeAction::Noop,
                parameters: BTreeMap::new(),
            }],
            verify: Vec::new(),
            source: RecipeSource {
                upstream_tag: "test".into(),
                upstream_commit: None,
                upstream_verb: "safe".into(),
            },
        }
    }

    #[test]
    fn rejects_download_filename_traversal() {
        assert!(recipe_with_filename("installer.exe").validate().is_ok());
        assert!(recipe_with_filename("../installer.exe").validate().is_err());
        assert!(
            recipe_with_filename("folder/installer.exe")
                .validate()
                .is_err()
        );
        assert!(
            recipe_with_filename("folder\\installer.exe")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn rejects_archive_include_traversal() {
        let mut recipe = recipe_with_filename("archive.tar.gz");
        recipe.steps = vec![RecipeStep::Extract {
            file: "installer".into(),
            destination: "${temp}/safe".into(),
            format: ArchiveFormat::TarGz,
            include: vec!["../outside".into()],
        }];
        assert!(recipe.validate().is_err());

        if let RecipeStep::Extract { include, .. } = &mut recipe.steps[0] {
            *include = vec!["folder/*.ttf".into()];
        }
        assert!(recipe.validate().is_ok());
    }

    #[test]
    fn validates_steps_inside_64_bit_conditionals() {
        let mut recipe = recipe_with_filename("installer.exe");
        recipe.steps = vec![RecipeStep::On64BitPrefix { steps: Vec::new() }];
        assert!(recipe.validate().is_err());

        recipe.steps = vec![RecipeStep::On64BitPrefix {
            steps: vec![RecipeStep::Copy {
                from: String::new(),
                to: "${system32_64}/component.dll".into(),
            }],
        }];
        assert!(recipe.validate().is_err());

        recipe.steps = vec![RecipeStep::On64BitPrefix {
            steps: vec![RecipeStep::Download {
                file: "missing".into(),
            }],
        }];
        assert!(recipe.validate().is_err());
    }

    #[test]
    fn rejects_unknown_native_actions_during_deserialization() {
        let content = r#"
schema = 1
id = "unsafe"
category = "settings"
title = "Unsafe"

[[steps]]
type = "native_action"
action = "run_arbitrary_shell"
"#;

        assert!(toml::from_str::<Recipe>(content).is_err());
    }

    #[test]
    fn omitted_maturity_fails_closed() {
        let content = r#"
schema = 1
id = "tracked"
category = "settings"
title = "Tracked"

[source]
upstream_tag = "test"
upstream_verb = "tracked"
"#;
        let recipe = toml::from_str::<Recipe>(content).unwrap();
        assert_eq!(recipe.maturity, RecipeMaturity::MetadataOnly);
    }

    #[test]
    fn rejects_registry_injection_in_dll_overrides() {
        let mut recipe = recipe_with_filename("installer.exe");
        recipe.steps = vec![RecipeStep::DllOverride {
            mode: DllOverrideMode::Native,
            libraries: vec!["safe.dll".into(), "bad\"\n[value]".into()],
            application: None,
        }];
        assert!(recipe.validate().is_err());

        recipe.steps = vec![RecipeStep::DllOverride {
            mode: DllOverrideMode::Builtin,
            libraries: vec!["d3d11".into()],
            application: Some("..\\unsafe".into()),
        }];
        assert!(recipe.validate().is_err());
    }

    #[test]
    fn validates_recipe_input_identifiers_and_environment_names() {
        let mut recipe = recipe_with_filename("installer.exe");
        recipe.inputs = vec![RecipeInput {
            id: "device".into(),
            label: "MIDI device".into(),
            description: None,
            placeholder: None,
            environment: Some("MIDI_DEVICE".into()),
            required: true,
        }];
        assert!(recipe.validate().is_ok());

        recipe.inputs[0].environment = Some("BAD-NAME".into());
        assert!(recipe.validate().is_err());
        recipe.inputs[0].environment = None;
        recipe.inputs.push(recipe.inputs[0].clone());
        assert!(recipe.validate().is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecipeStep {
    Download {
        file: String,
    },
    EnsureDirectory {
        path: String,
    },
    EnsureFile {
        path: String,
    },
    VerifyPath {
        path: String,
        kind: DetectKind,
    },
    Copy {
        from: String,
        to: String,
    },
    Move {
        from: String,
        to: String,
    },
    Remove {
        path: String,
        recursive: bool,
    },
    RemoveSymlink {
        path: String,
    },
    Extract {
        file: String,
        destination: String,
        format: ArchiveFormat,
        #[serde(default)]
        include: Vec<String>,
    },
    ExtractPath {
        source: String,
        destination: String,
        format: ArchiveFormat,
        #[serde(default)]
        include: Vec<String>,
    },
    InstallFonts {
        source: String,
        fonts: Vec<RecipeFont>,
    },
    FontReplacements {
        replacements: Vec<RecipeFontReplacement>,
    },
    On64BitPrefix {
        steps: Vec<RecipeStep>,
    },
    Wine {
        program: String,
        #[serde(default)]
        arguments: Vec<String>,
        #[serde(default)]
        unattended_arguments: Vec<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
    },
    Registry {
        content: String,
        architecture: RegistryArchitecture,
    },
    DllOverride {
        mode: DllOverrideMode,
        libraries: Vec<String>,
        application: Option<String>,
    },
    WindowsVersion {
        version: String,
        application: Option<String>,
    },
    Call {
        recipe: String,
    },
    Notice {
        level: PromptLevel,
        title: String,
        message: String,
    },
    Prompt {
        level: PromptLevel,
        title: String,
        message: String,
    },
    NativeAction {
        action: NativeAction,
        #[serde(default)]
        parameters: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAction {
    Noop,
    FontSmoothing,
    WinebootUpdate,
    FontfixCheck,
    IsolateHome,
    NativeMdac,
    RemoveMono,
    SetMidiDevice,
    SetUserPath,
    IntentionalFailure,
}

impl NativeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::FontSmoothing => "font_smoothing",
            Self::WinebootUpdate => "wineboot_update",
            Self::FontfixCheck => "fontfix_check",
            Self::IsolateHome => "isolate_home",
            Self::NativeMdac => "native_mdac",
            Self::RemoveMono => "remove_mono",
            Self::SetMidiDevice => "set_midi_device",
            Self::SetUserPath => "set_user_path",
            Self::IntentionalFailure => "intentional_failure",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    Zip,
    SevenZip,
    Cabinet,
    Tar,
    TarGz,
    TarXz,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryArchitecture {
    Prefix,
    Win32,
    Win64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DllOverrideMode {
    Native,
    Builtin,
    NativeBuiltin,
    BuiltinNative,
    Disabled,
    Default,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptLevel {
    Info,
    Warning,
    Confirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrefixArchitecture {
    Win32,
    Win64,
    Wow64,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefixSource {
    DefaultWine,
    WinePrefixes,
    Steam,
    Lutris,
    Bottles,
    Heroic,
    Manual,
}

impl PrefixSource {
    pub fn is_managed(self) -> bool {
        matches!(
            self,
            Self::Steam | Self::Lutris | Self::Bottles | Self::Heroic
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinePrefix {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub source: PrefixSource,
    pub architecture: PrefixArchitecture,
    pub runtime: Option<PathBuf>,
    pub runtime_label: Option<String>,
    pub managed: bool,
    pub exists: bool,
    pub installed_verbs: Vec<String>,
    pub size_bytes: Option<u64>,
    pub last_modified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WineRuntime {
    pub id: String,
    pub label: String,
    pub wine_binary: PathBuf,
    pub wineserver_binary: Option<PathBuf>,
    pub version: Option<String>,
    pub source: RuntimeSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSource {
    System,
    Steam,
    Lutris,
    Bottles,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSummary {
    pub version: String,
    pub upstream_tag: String,
    pub recipe_count: usize,
    pub native_count: usize,
    pub metadata_only_count: usize,
    pub categories: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogVersionRecord {
    pub version: String,
    pub upstream_tag: String,
    pub path: PathBuf,
    pub signature: Option<String>,
    pub active: bool,
    pub installed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogQuery {
    pub search: Option<String>,
    pub category: Option<VerbCategory>,
    pub media: Option<MediaKind>,
    pub installed_only: bool,
    pub cached_only: bool,
    pub compatible_only: bool,
    pub prefix_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeListItem {
    pub id: String,
    pub category: VerbCategory,
    pub title: String,
    pub publisher: Option<String>,
    pub year: Option<String>,
    pub description: Option<String>,
    pub media: MediaKind,
    pub maturity: RecipeMaturity,
    pub tags: Vec<String>,
    pub installed: bool,
    pub cached: bool,
    pub compatible: bool,
    pub compatibility_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationOptions {
    pub force: bool,
    pub unattended: bool,
    pub verify: bool,
    pub no_clean: bool,
    pub isolate: bool,
    pub torify: bool,
    pub country: Option<String>,
    pub create_restore_point: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRequest {
    pub prefix_id: Uuid,
    pub recipes: Vec<String>,
    #[serde(default)]
    pub input_values: BTreeMap<String, String>,
    #[serde(default)]
    pub options: OperationOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedStep {
    pub recipe_id: String,
    pub recipe_title: String,
    pub step_index: usize,
    pub label: String,
    pub destructive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPlan {
    pub id: Uuid,
    pub prefix: WinePrefix,
    pub requested_recipes: Vec<String>,
    pub resolved_recipes: Vec<String>,
    pub steps: Vec<PlannedStep>,
    pub inputs: Vec<PlannedInput>,
    pub downloads: Vec<PlannedDownload>,
    pub conflicts: Vec<PlanIssue>,
    pub warnings: Vec<PlanIssue>,
    pub restore_recommended: bool,
    pub estimated_download_bytes: Option<u64>,
    pub options: OperationOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedInput {
    pub key: String,
    pub recipe_id: String,
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub required: bool,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedDownload {
    pub recipe_id: String,
    pub file_id: String,
    pub filename: String,
    pub urls: Vec<String>,
    pub cached: bool,
    pub manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanIssue {
    pub code: String,
    pub title: String,
    pub message: String,
    pub recipe_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Planned,
    Preflight,
    Running,
    WaitingForUser,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeFailureKind {
    Failed,
    SkippedDependency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeFailure {
    pub recipe_id: String,
    pub recipe_title: String,
    pub kind: RecipeFailureKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRecord {
    pub id: Uuid,
    pub prefix_id: Uuid,
    pub prefix_name: String,
    pub recipes: Vec<String>,
    pub state: OperationState,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub current_step: usize,
    pub total_steps: usize,
    pub message: Option<String>,
    #[serde(default)]
    pub failures: Vec<RecipeFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEvent {
    pub operation_id: Uuid,
    pub sequence: u64,
    pub state: OperationState,
    pub step: usize,
    pub total_steps: usize,
    pub recipe_id: Option<String>,
    pub title: String,
    pub detail: Option<String>,
    pub progress: Option<f64>,
    pub prompt: Option<OperationPrompt>,
    pub log_line: Option<String>,
    #[serde(default)]
    pub failure: Option<RecipeFailure>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationPrompt {
    pub id: Uuid,
    pub level: PromptLevel,
    pub title: String,
    pub message: String,
    pub choices: Vec<PromptChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptChoice {
    pub id: String,
    pub label: String,
    pub destructive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptResponse {
    pub operation_id: Uuid,
    pub prompt_id: Uuid,
    pub choice_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestorePoint {
    pub id: Uuid,
    pub prefix_id: Uuid,
    pub prefix_name: String,
    pub prefix_path: PathBuf,
    pub storage_path: PathBuf,
    pub method: RestoreMethod,
    pub created_at: DateTime<Utc>,
    pub size_bytes: Option<u64>,
    pub operation_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMethod {
    Btrfs,
    Reflink,
    Archive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCheck {
    pub id: String,
    pub label: String,
    pub required: bool,
    pub available: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemReport {
    pub ready: bool,
    pub os: String,
    pub architecture: String,
    pub desktop: Option<String>,
    pub dependencies: Vec<DependencyCheck>,
    pub runtimes: Vec<WineRuntime>,
    pub data_directory: PathBuf,
    pub cache_directory: PathBuf,
    pub state_directory: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub path: PathBuf,
    pub file_count: usize,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: ThemePreference,
    pub language: String,
    pub catalog_auto_update: bool,
    pub restore_before_managed_changes: bool,
    pub show_advanced: bool,
    pub reduced_motion: bool,
    pub custom_wine_binary: Option<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            language: "system".into(),
            catalog_auto_update: true,
            restore_before_managed_changes: true,
            show_advanced: false,
            reduced_motion: false,
            custom_wine_binary: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}
