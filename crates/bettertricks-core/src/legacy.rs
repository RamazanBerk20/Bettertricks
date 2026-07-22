use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use uuid::Uuid;

use crate::{
    AppPaths, BettertricksError, OperationOptions, Result, VerbCategory, WinePrefix,
    system::find_command,
};

const MAX_LEGACY_VERB_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MANAGED_HOST_BYTES: u64 = 4 * 1024 * 1024;
pub const MANAGED_WINETRICKS_TAG: &str = "20260125";
pub const MANAGED_WINETRICKS_SHA256: &str =
    "431f82fc74000e6c864409f1d8fb495d696c03928808e3e8acffc45179312a7b";
pub const MANAGED_WINETRICKS_URL: &str =
    "https://raw.githubusercontent.com/Winetricks/winetricks/20260125/src/winetricks";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyVerbInfo {
    pub path: PathBuf,
    pub id: String,
    pub category: VerbCategory,
    pub title: Option<String>,
    pub size_bytes: u64,
    pub warning: String,
}

#[derive(Debug, Clone)]
pub struct LegacyVerbHost {
    managed: Option<ManagedHost>,
    system: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ManagedHost {
    path: PathBuf,
    tag: String,
    sha256: String,
}

impl Default for LegacyVerbHost {
    fn default() -> Self {
        Self::discover()
    }
}

impl LegacyVerbHost {
    pub fn discover() -> Self {
        Self {
            managed: None,
            system: find_command("winetricks"),
        }
    }

    pub fn discover_with_paths(paths: &AppPaths) -> Self {
        Self {
            managed: Some(ManagedHost {
                path: managed_host_path(paths, MANAGED_WINETRICKS_TAG),
                tag: MANAGED_WINETRICKS_TAG.into(),
                sha256: MANAGED_WINETRICKS_SHA256.into(),
            }),
            system: find_command("winetricks"),
        }
    }

    pub fn with_binary(binary: PathBuf) -> Self {
        Self {
            managed: None,
            system: Some(binary),
        }
    }

    pub fn available(&self) -> bool {
        self.binary_path().is_ok_and(|path| path.is_some())
    }

    pub fn binary_path(&self) -> Result<Option<PathBuf>> {
        if let Some(managed) = &self.managed
            && managed.path.exists()
        {
            verify_managed_host(managed)?;
            return Ok(Some(managed.path.clone()));
        }
        Ok(self.system.clone())
    }

    pub async fn require_baseline(&self, expected_tag: &str) -> Result<()> {
        let binary = self.binary_for_baseline(expected_tag)?.ok_or_else(|| {
            BettertricksError::Unsupported(
                "install the verified Winetricks compatibility host from Bettertricks settings"
                    .into(),
            )
        })?;
        let output = Command::new(&binary).arg("--version").output().await?;
        if !output.status.success() {
            return Err(BettertricksError::CommandFailed {
                program: binary.display().to_string(),
                code: output.status.code(),
            });
        }
        let version = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if version
            .split_whitespace()
            .any(|token| token == expected_tag)
        {
            return Ok(());
        }
        let installed = version
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("unknown version");
        Err(BettertricksError::Unsupported(format!(
            "installed Winetricks ({installed}) does not match the catalog baseline {expected_tag}"
        )))
    }

    pub fn recipe_command(
        &self,
        recipe_id: &str,
        expected_tag: &str,
        prefix: &WinePrefix,
        options: &OperationOptions,
    ) -> Result<Command> {
        if recipe_id.is_empty()
            || !recipe_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'='
            })
        {
            return Err(BettertricksError::Security(format!(
                "invalid Winetricks recipe identifier {recipe_id:?}"
            )));
        }
        let binary = self.binary_for_baseline(expected_tag)?.ok_or_else(|| {
            BettertricksError::Unsupported(
                "install the verified Winetricks compatibility host from Bettertricks settings"
                    .into(),
            )
        })?;
        let mut command = Command::new(binary);
        apply_options(&mut command, options);
        command
            .arg(recipe_id)
            .env("WINEPREFIX", &prefix.path)
            .env("WINETRICKS_OPT_SHAREDPREFIX", "1");
        if let Some(runtime) = &prefix.runtime {
            command.env("WINE", runtime);
        }
        Ok(command)
    }

    pub fn inspect(&self, path: &Path) -> Result<LegacyVerbInfo> {
        let path = canonical_verb(path)?;
        let metadata = std::fs::metadata(&path)?;
        if metadata.len() > MAX_LEGACY_VERB_BYTES {
            return Err(BettertricksError::Security(format!(
                "legacy verb exceeds the {} MiB limit",
                MAX_LEGACY_VERB_BYTES / 1024 / 1024
            )));
        }
        let content = std::fs::read_to_string(&path)?;
        let declaration = Regex::new(
            r"(?m)^\s*w_metadata\s+([a-z0-9_=]+)\s+(apps|benchmarks|dlls|fonts|settings)(?:\s|\\|$)",
        )
        .expect("legacy metadata regex")
        .captures_iter(&content)
        .collect::<Vec<_>>();
        if declaration.len() != 1 {
            return Err(BettertricksError::Recipe(
                "legacy verb must contain exactly one w_metadata declaration".into(),
            ));
        }
        let declaration = &declaration[0];
        let id = declaration[1].to_string();
        let filename_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if id != filename_id {
            return Err(BettertricksError::Security(format!(
                "legacy verb declares {id}, but its filename is {filename_id}.verb"
            )));
        }
        let title = Regex::new(r#"(?m)^\s*title="([^"]+)""#)
            .expect("legacy title regex")
            .captures(&content)
            .map(|capture| capture[1].to_string());
        Ok(LegacyVerbInfo {
            path,
            id,
            category: parse_category(&declaration[2]),
            title,
            size_bytes: metadata.len(),
            warning: "Legacy .verb files are shell programs and run with your user permissions. Only continue for code you have reviewed and trust.".into(),
        })
    }

    pub async fn run(
        &self,
        path: &Path,
        prefix: &WinePrefix,
        options: &OperationOptions,
        trusted: bool,
    ) -> Result<()> {
        let info = self.inspect(path)?;
        if !trusted {
            return Err(BettertricksError::Security(info.warning));
        }
        let binary = self.binary_path()?.ok_or_else(|| {
            BettertricksError::Unsupported(
                "running custom .verb files requires the optional Winetricks compatibility host"
                    .into(),
            )
        })?;
        let mut command = Command::new(&binary);
        apply_options(&mut command, options);
        command
            .arg(&info.path)
            .env("WINEPREFIX", &prefix.path)
            .env("WINETRICKS_OPT_SHAREDPREFIX", "1");
        if let Some(runtime) = &prefix.runtime {
            command.env("WINE", runtime);
        }
        let status = command.status().await?;
        if !status.success() {
            return Err(BettertricksError::CommandFailed {
                program: binary.display().to_string(),
                code: status.code(),
            });
        }
        Ok(())
    }

    fn binary_for_baseline(&self, expected_tag: &str) -> Result<Option<PathBuf>> {
        if let Some(managed) = &self.managed
            && managed.tag == expected_tag
            && managed.path.exists()
        {
            verify_managed_host(managed)?;
            return Ok(Some(managed.path.clone()));
        }
        Ok(self.system.clone())
    }
}

pub async fn install_managed_compatibility_host(
    paths: &AppPaths,
    expected_tag: &str,
) -> Result<PathBuf> {
    if expected_tag != MANAGED_WINETRICKS_TAG {
        return Err(BettertricksError::Unsupported(format!(
            "Bettertricks does not ship a verified compatibility host for catalog baseline {expected_tag}"
        )));
    }
    let destination = managed_host_path(paths, expected_tag);
    let managed = ManagedHost {
        path: destination.clone(),
        tag: expected_tag.into(),
        sha256: MANAGED_WINETRICKS_SHA256.into(),
    };
    if destination.exists() && verify_managed_host(&managed).is_ok() {
        LegacyVerbHost {
            managed: Some(managed),
            system: None,
        }
        .require_baseline(expected_tag)
        .await?;
        return Ok(destination);
    }

    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(concat!("Bettertricks/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get(MANAGED_WINETRICKS_URL)
        .send()
        .await?
        .error_for_status()?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MANAGED_HOST_BYTES)
    {
        return Err(BettertricksError::Security(
            "Winetricks compatibility host exceeds the download limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_MANAGED_HOST_BYTES as usize {
            return Err(BettertricksError::Security(
                "Winetricks compatibility host exceeds the download limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    publish_managed_host(&destination, MANAGED_WINETRICKS_SHA256, &bytes).await?;
    LegacyVerbHost {
        managed: Some(managed),
        system: None,
    }
    .require_baseline(expected_tag)
    .await?;
    Ok(destination)
}

fn managed_host_path(paths: &AppPaths, tag: &str) -> PathBuf {
    paths.compatibility_hosts.join(format!("winetricks-{tag}"))
}

async fn publish_managed_host(destination: &Path, expected: &str, bytes: &[u8]) -> Result<()> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual != expected {
        return Err(BettertricksError::ChecksumMismatch {
            file: destination.display().to_string(),
            expected: expected.into(),
            actual,
        });
    }
    let parent = destination.parent().ok_or_else(|| {
        BettertricksError::Security("compatibility host has no parent directory".into())
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let staging = parent.join(format!(".winetricks-{}.partial", Uuid::new_v4()));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staging)
            .await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        // Tokio may finish closing its blocking file handle on the next scheduler turn. Ensure the
        // writable handle is gone before the script can be renamed and executed on Linux.
        tokio::task::yield_now().await;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))?;
        tokio::fs::rename(&staging, destination).await?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&staging).await;
    }
    result
}

fn verify_managed_host(host: &ManagedHost) -> Result<()> {
    let metadata = std::fs::symlink_metadata(&host.path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(BettertricksError::Security(
            "managed Winetricks host must be a regular file".into(),
        ));
    }
    if metadata.len() > MAX_MANAGED_HOST_BYTES || metadata.permissions().mode() & 0o111 == 0 {
        return Err(BettertricksError::Security(
            "managed Winetricks host has invalid size or permissions".into(),
        ));
    }
    let mut file = std::fs::File::open(&host.path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = hex::encode(digest.finalize());
    if actual != host.sha256 {
        return Err(BettertricksError::ChecksumMismatch {
            file: host.path.display().to_string(),
            expected: host.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

fn apply_options(command: &mut Command, options: &OperationOptions) {
    if options.force {
        command.arg("--force");
    }
    if options.unattended {
        command.arg("--unattended");
    }
    if options.no_clean {
        command.arg("--no-clean");
    }
    if options.isolate {
        command.arg("--isolate");
    }
    if options.torify {
        command.arg("--torify");
    }
    if options.verify {
        command.arg("--verify");
    }
    if let Some(country) = &options.country {
        command.arg(format!("--country={country}"));
    }
}

fn canonical_verb(path: &Path) -> Result<PathBuf> {
    if path.extension().and_then(|value| value.to_str()) != Some("verb") {
        return Err(BettertricksError::Security(
            "legacy recipe files must use the .verb extension".into(),
        ));
    }
    let path = path.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(BettertricksError::Security(
            "legacy verb must be a regular file".into(),
        ));
    }
    Ok(path)
}

fn parse_category(value: &str) -> VerbCategory {
    match value {
        "apps" => VerbCategory::Apps,
        "benchmarks" => VerbCategory::Benchmarks,
        "dlls" => VerbCategory::Dlls,
        "fonts" => VerbCategory::Fonts,
        _ => VerbCategory::Settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn inspects_a_single_matching_metadata_declaration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("custom.verb");
        std::fs::write(
            &path,
            "w_metadata custom dlls \\\n+    title=\"Custom component\"\nload_custom() { :; }\n",
        )
        .unwrap();
        let info = LegacyVerbHost::discover().inspect(&path).unwrap();
        assert_eq!(info.id, "custom");
        assert_eq!(info.title.as_deref(), None);
    }

    #[test]
    fn rejects_a_filename_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("other.verb");
        std::fs::write(&path, "w_metadata custom dlls title=\"Custom\"\n").unwrap();
        assert!(LegacyVerbHost::discover().inspect(&path).is_err());
    }

    #[tokio::test]
    async fn accepts_only_the_catalogs_exact_winetricks_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("winetricks");
        std::fs::write(&binary, "#!/bin/sh\necho '20260125 - test build'\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let host = LegacyVerbHost::with_binary(binary);

        host.require_baseline("20260125").await.unwrap();
        assert!(host.require_baseline("20270101").await.is_err());
    }

    #[tokio::test]
    async fn builds_named_recipe_commands_without_shell_interpolation() {
        let host = LegacyVerbHost::with_binary(PathBuf::from("/bin/echo"));
        let prefix = WinePrefix {
            id: uuid::Uuid::new_v4(),
            name: "Test".into(),
            path: PathBuf::from("/tmp/test-prefix"),
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Wow64,
            runtime: Some(PathBuf::from("/usr/bin/wine")),
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        };
        let options = OperationOptions {
            force: true,
            unattended: true,
            isolate: true,
            ..OperationOptions::default()
        };
        let output = host
            .recipe_command("vcrun2022", "20260125", &prefix, &options)
            .unwrap()
            .output()
            .await
            .unwrap();

        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            "--force --unattended --isolate vcrun2022"
        );
        assert!(
            host.recipe_command("--unsafe", "20260125", &prefix, &options)
                .is_err()
        );
    }

    #[tokio::test]
    async fn publishes_and_revalidates_a_managed_host() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("winetricks-test");
        let bytes = b"#!/bin/sh\necho test\n";
        let checksum = hex::encode(Sha256::digest(bytes));
        publish_managed_host(&destination, &checksum, bytes)
            .await
            .unwrap();
        let managed = ManagedHost {
            path: destination.clone(),
            tag: "test".into(),
            sha256: checksum,
        };
        let host = LegacyVerbHost {
            managed: Some(managed.clone()),
            system: None,
        };

        assert_eq!(
            host.binary_for_baseline("test").unwrap(),
            Some(destination.clone())
        );
        host.require_baseline("test").await.unwrap();
        assert_ne!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );

        std::fs::write(&destination, "#!/bin/sh\necho tampered\n").unwrap();
        assert!(verify_managed_host(&managed).is_err());
    }

    #[tokio::test]
    async fn rejects_managed_host_checksum_mismatches_before_publication() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("winetricks-test");

        assert!(
            publish_managed_host(&destination, &"0".repeat(64), b"unexpected")
                .await
                .is_err()
        );
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn rejects_unpinned_managed_host_baselines_without_network_access() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();

        assert!(
            install_managed_compatibility_host(&paths, "20990101")
                .await
                .is_err()
        );
    }
}
