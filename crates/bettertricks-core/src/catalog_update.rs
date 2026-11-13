use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::{
    AppPaths, BettertricksError, Catalog, CatalogSource, CatalogVersionRecord, Result, Store,
};

const INDEX_SCHEMA: u32 = 1;
const MAX_CATALOG_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogIndex {
    pub schema: u32,
    pub releases: Vec<CatalogRelease>,
}

impl CatalogIndex {
    pub fn latest_after(&self, current: &str) -> Option<&CatalogRelease> {
        let current = CatalogVersion::parse(current);
        self.releases
            .iter()
            .filter(|release| CatalogVersion::parse(&release.version) > current)
            .max_by_key(|release| CatalogVersion::parse(&release.version))
    }
}

/// A catalog version: `winetricks-<upstream tag>` with an optional `-r<revision>` suffix, so
/// recipe fixes for the same upstream tag can ship as newer catalogs. Ordered by tag, then
/// revision (numerically: `r10` is newer than `r9`). Unparseable versions sort first.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CatalogVersion {
    parsed: bool,
    tag_length: usize,
    tag: String,
    revision: u64,
}

impl CatalogVersion {
    pub fn parse(version: &str) -> Self {
        let unparsed = Self {
            parsed: false,
            tag_length: 0,
            tag: version.to_owned(),
            revision: 0,
        };
        let Some(rest) = version.strip_prefix("winetricks-") else {
            return unparsed;
        };
        let (tag, revision) = match rest.split_once("-r") {
            Some((tag, revision)) => match revision.parse::<u64>() {
                Ok(revision) => (tag, revision),
                Err(_) => return unparsed,
            },
            None => (rest, 0),
        };
        if tag.is_empty()
            || !tag
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
        {
            return unparsed;
        }
        Self {
            parsed: true,
            tag_length: tag.len(),
            tag: tag.to_owned(),
            revision,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.parsed
    }

    pub fn upstream_tag(&self) -> &str {
        &self.tag
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRelease {
    pub version: String,
    pub upstream_tag: String,
    pub url: Url,
    pub sha256: String,
    pub signature: String,
    pub recipe_count: usize,
}

impl CatalogRelease {
    pub fn signing_payload(&self) -> Vec<u8> {
        format!(
            "bettertricks-catalog-v1\n{}\n{}\n{}\n{}\n{}\n",
            self.version, self.upstream_tag, self.sha256, self.url, self.recipe_count
        )
        .into_bytes()
    }
}

#[derive(Clone)]
pub struct CatalogUpdater {
    paths: AppPaths,
    catalog: Catalog,
    store: Arc<Store>,
    verifying_key: VerifyingKey,
    client: reqwest::Client,
}

impl CatalogUpdater {
    pub fn new(
        paths: AppPaths,
        catalog: Catalog,
        store: Arc<Store>,
        public_key: [u8; 32],
    ) -> Result<Self> {
        let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|error| {
            BettertricksError::Security(format!("invalid catalog key: {error}"))
        })?;
        let client = reqwest::Client::builder()
            .user_agent(format!(
                "Bettertricks/{} catalog-updater",
                env!("CARGO_PKG_VERSION")
            ))
            .https_only(true)
            .build()?;
        Ok(Self {
            paths,
            catalog,
            store,
            verifying_key,
            client,
        })
    }

    pub fn from_hex_key(
        paths: AppPaths,
        catalog: Catalog,
        store: Arc<Store>,
        public_key: &str,
    ) -> Result<Self> {
        let bytes = hex::decode(public_key).map_err(|error| {
            BettertricksError::Security(format!("invalid catalog key: {error}"))
        })?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| {
            BettertricksError::Security("catalog public key must contain 32 bytes".into())
        })?;
        Self::new(paths, catalog, store, key)
    }

    pub async fn fetch_index(&self, url: &Url) -> Result<CatalogIndex> {
        if url.scheme() != "https" {
            return Err(BettertricksError::Security(
                "catalog indexes must use HTTPS".into(),
            ));
        }
        let index = self
            .client
            .get(url.clone())
            .send()
            .await?
            .error_for_status()?
            .json::<CatalogIndex>()
            .await?;
        if index.schema != INDEX_SCHEMA {
            return Err(BettertricksError::Catalog(format!(
                "unsupported catalog index schema {}",
                index.schema
            )));
        }
        for release in &index.releases {
            self.verify_release(release)?;
        }
        Ok(index)
    }

    pub fn verify_release(&self, release: &CatalogRelease) -> Result<()> {
        validate_release(release)?;
        let signature_bytes = hex::decode(&release.signature).map_err(|error| {
            BettertricksError::Security(format!("invalid catalog signature encoding: {error}"))
        })?;
        let signature = Signature::try_from(signature_bytes.as_slice()).map_err(|error| {
            BettertricksError::Security(format!("invalid catalog signature: {error}"))
        })?;
        self.verifying_key
            .verify(&release.signing_payload(), &signature)
            .map_err(|_| {
                BettertricksError::Security("catalog signature verification failed".into())
            })
    }

    /// Installs a signed release that is newer than the active catalog. Older or equal
    /// releases are refused so a replayed, superseded catalog cannot be re-activated; going
    /// back is only possible through [`rollback_catalog`].
    pub async fn install(&self, release: &CatalogRelease) -> Result<CatalogVersionRecord> {
        self.verify_release(release)?;
        let current = self.catalog.summary().version;
        if CatalogVersion::parse(&release.version) <= CatalogVersion::parse(&current) {
            return Err(BettertricksError::Security(format!(
                "catalog {} is not newer than the active catalog {current}",
                release.version
            )));
        }
        let response = self
            .client
            .get(release.url.clone())
            .send()
            .await?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_CATALOG_BYTES as u64)
        {
            return Err(BettertricksError::Security(
                "catalog bundle exceeds the size limit".into(),
            ));
        }

        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_CATALOG_BYTES {
                return Err(BettertricksError::Security(
                    "catalog bundle exceeds the size limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        self.install_bytes(release, bytes).await
    }

    pub async fn install_bytes(
        &self,
        release: &CatalogRelease,
        bytes: Vec<u8>,
    ) -> Result<CatalogVersionRecord> {
        self.verify_release(release)?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(&release.sha256) {
            return Err(BettertricksError::ChecksumMismatch {
                file: release.url.to_string(),
                expected: release.sha256.clone(),
                actual,
            });
        }

        let paths = self.paths.clone();
        let catalog = self.catalog.clone();
        let store = self.store.clone();
        let release = release.clone();
        tokio::task::spawn_blocking(move || {
            install_verified_bundle(&paths, &catalog, &store, &release, &bytes)
        })
        .await
        .map_err(|error| BettertricksError::Io(std::io::Error::other(error)))?
    }

    pub fn rollback(&self) -> Result<CatalogVersionRecord> {
        rollback_catalog(&self.catalog, &self.store)
    }
}

/// The catalog that was active before the current one, if its files still exist.
pub fn rollback_target(store: &Store) -> Result<Option<CatalogVersionRecord>> {
    let versions = store.catalog_versions()?;
    let Some(previous) = versions
        .iter()
        .find(|version| version.active)
        .and_then(|active| active.previous.clone())
    else {
        return Ok(None);
    };
    Ok(versions
        .into_iter()
        .find(|version| version.version == previous && version.path.is_dir()))
}

pub fn rollback_catalog(catalog: &Catalog, store: &Store) -> Result<CatalogVersionRecord> {
    let previous = rollback_target(store)?
        .ok_or_else(|| BettertricksError::Catalog("no previous catalog is available".into()))?;
    activate_existing(catalog, store, &previous)?;
    // The user chose this catalog: keep it across launches even if the bundled one is newer,
    // until an explicit update activates another.
    store.pin_active_catalog()?;
    Ok(CatalogVersionRecord {
        active: true,
        ..previous
    })
}

fn validate_release(release: &CatalogRelease) -> Result<()> {
    if release.version.is_empty()
        || !release.version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        return Err(BettertricksError::Security(
            "catalog version contains unsafe characters".into(),
        ));
    }
    let version = CatalogVersion::parse(&release.version);
    if !version.is_valid() || version.upstream_tag() != release.upstream_tag {
        return Err(BettertricksError::Security(format!(
            "catalog version {} must be winetricks-{}[-rN]",
            release.version, release.upstream_tag
        )));
    }
    if release.upstream_tag.trim().is_empty() || release.url.scheme() != "https" {
        return Err(BettertricksError::Security(
            "catalog releases require an upstream tag and HTTPS URL".into(),
        ));
    }
    if release.sha256.len() != 64
        || !release
            .sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(BettertricksError::Security(
            "catalog release has an invalid SHA-256".into(),
        ));
    }
    if release.recipe_count == 0 {
        return Err(BettertricksError::Security(
            "catalog release cannot be empty".into(),
        ));
    }
    Ok(())
}

fn install_verified_bundle(
    paths: &AppPaths,
    catalog: &Catalog,
    store: &Store,
    release: &CatalogRelease,
    bytes: &[u8],
) -> Result<CatalogVersionRecord> {
    let destination = paths.catalogs.join(&release.version);
    let staging = paths.catalogs.join(format!(".staging-{}", Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    let mut cleanup = StagingCleanup(Some(staging.clone()));
    unpack_catalog(bytes, &staging)?;
    let root = if staging.join("catalog").is_dir() {
        staging.join("catalog")
    } else {
        staging.clone()
    };
    validate_manifest(&root, release)?;
    let candidate = Catalog::load(
        CatalogSource {
            path: root.clone(),
            version: release.version.clone(),
            upstream_tag: release.upstream_tag.clone(),
        },
        paths.winetricks_cache.clone(),
    )?;
    let summary = candidate.summary();
    if summary.recipe_count != release.recipe_count {
        return Err(BettertricksError::Security(format!(
            "catalog recipe count mismatch: signed {}, found {}",
            release.recipe_count, summary.recipe_count
        )));
    }

    if destination.is_dir() {
        // Never activate stale files under a reused version name: the verified bytes must be
        // exactly what is already installed.
        if !same_tree(&root, &destination)? {
            return Err(BettertricksError::Security(format!(
                "a different catalog is already installed as {}; publish a new revision instead",
                release.version
            )));
        }
    } else if root == staging {
        std::fs::rename(&staging, &destination)?;
        cleanup.0 = None;
    } else {
        std::fs::rename(&root, &destination)?;
    }
    let record = record_for(release, destination);
    activate_existing(catalog, store, &record)?;
    Ok(CatalogVersionRecord {
        active: true,
        ..record
    })
}

fn activate_existing(
    catalog: &Catalog,
    store: &Store,
    record: &CatalogVersionRecord,
) -> Result<()> {
    let candidate = Catalog::load(
        CatalogSource {
            path: record.path.clone(),
            version: record.version.clone(),
            upstream_tag: record.upstream_tag.clone(),
        },
        record.path.join(".cache-probe"),
    )?;
    if candidate.summary().recipe_count == 0 {
        return Err(BettertricksError::Catalog(
            "refusing to activate an empty catalog".into(),
        ));
    }
    catalog.reload(CatalogSource {
        path: record.path.clone(),
        version: record.version.clone(),
        upstream_tag: record.upstream_tag.clone(),
    })?;
    store.activate_catalog_version(
        &record.version,
        &record.upstream_tag,
        &record.path,
        record.signature.as_deref(),
    )
}

fn same_tree(left: &Path, right: &Path) -> Result<bool> {
    let listing = |root: &Path| -> Result<Vec<(PathBuf, Vec<u8>)>> {
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry.map_err(|error| BettertricksError::Catalog(error.to_string()))?;
            if entry.file_type().is_file() {
                let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
                entries.push((relative.to_path_buf(), std::fs::read(entry.path())?));
            }
        }
        Ok(entries)
    };
    Ok(listing(left)? == listing(right)?)
}

fn record_for(release: &CatalogRelease, path: PathBuf) -> CatalogVersionRecord {
    CatalogVersionRecord {
        version: release.version.clone(),
        upstream_tag: release.upstream_tag.clone(),
        path,
        signature: Some(release.signature.clone()),
        active: false,
        installed_at: chrono::Utc::now(),
        previous: None,
    }
}

fn unpack_catalog(bytes: &[u8], destination: &Path) -> Result<()> {
    let decoder = zstd::stream::read::Decoder::new(Cursor::new(bytes))?;
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(BettertricksError::Security(format!(
                "catalog archive contains an unsafe path: {}",
                path.display()
            )));
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(BettertricksError::Security(format!(
                "catalog archive contains a link or special file: {}",
                path.display()
            )));
        }
        if !entry.unpack_in(destination)? {
            return Err(BettertricksError::Security(format!(
                "catalog archive escaped its staging directory: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_manifest(root: &Path, release: &CatalogRelease) -> Result<()> {
    let path = root.join("manifest.json");
    if !path.is_file() {
        return Err(BettertricksError::Catalog(
            "catalog bundle is missing manifest.json".into(),
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let version = value.get("version").and_then(serde_json::Value::as_str);
    let upstream = value.get("upstreamTag").and_then(serde_json::Value::as_str);
    if version != Some(release.version.as_str()) || upstream != Some(release.upstream_tag.as_str())
    {
        return Err(BettertricksError::Security(
            "catalog manifest does not match its signed release descriptor".into(),
        ));
    }
    Ok(())
}

struct StagingCleanup(Option<PathBuf>);

impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    fn release(signing_key: &SigningKey) -> CatalogRelease {
        let mut release = CatalogRelease {
            version: "winetricks-20990101".into(),
            upstream_tag: "20990101".into(),
            url: Url::parse("https://updates.example.test/catalog.tar.zst").unwrap(),
            sha256: "00".repeat(32),
            signature: String::new(),
            recipe_count: 1,
        };
        release.signature = hex::encode(signing_key.sign(&release.signing_payload()).to_bytes());
        release
    }

    fn signed_release(signing_key: &SigningKey, bytes: &[u8]) -> CatalogRelease {
        let mut release = release(signing_key);
        release.sha256 = hex::encode(Sha256::digest(bytes));
        release.signature = hex::encode(signing_key.sign(&release.signing_payload()).to_bytes());
        release
    }

    fn catalog_bundle() -> Vec<u8> {
        catalog_bundle_with_title("Good")
    }

    fn catalog_bundle_with_title(title: &str) -> Vec<u8> {
        let good = format!(
            "schema=1\nid=\"good\"\ncategory=\"settings\"\ntitle=\"{title}\"\n[source]\nupstream_tag=\"20990101\"\nupstream_verb=\"good\"\n"
        );
        let encoder = zstd::Encoder::new(Vec::new(), 1).unwrap();
        let mut archive = tar::Builder::new(encoder);
        for (path, content) in [
            (
                "catalog/manifest.json",
                r#"{"version":"winetricks-20990101","upstreamTag":"20990101","generatedRecipes":1,"nativeRecipes":0}"#,
            ),
            ("catalog/good.toml", good.as_str()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, path, content.as_bytes())
                .unwrap();
        }
        archive.finish().unwrap();
        let encoder = archive.into_inner().unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn verifies_the_entire_release_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        std::fs::write(
            temp.path().join("good.toml"),
            "schema=1\nid=\"good\"\ncategory=\"settings\"\ntitle=\"Good\"\n[source]\nupstream_tag=\"test\"\nupstream_verb=\"good\"\n",
        )
        .unwrap();
        let catalog = Catalog::load(
            CatalogSource {
                path: temp.path().into(),
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            paths.winetricks_cache.clone(),
        )
        .unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let updater = CatalogUpdater::new(
            paths,
            catalog,
            store,
            signing_key.verifying_key().to_bytes(),
        )
        .unwrap();
        let signed = release(&signing_key);
        updater.verify_release(&signed).unwrap();

        let mut tampered = signed;
        tampered.recipe_count = 2;
        assert!(updater.verify_release(&tampered).is_err());
    }

    #[test]
    fn orders_revisions_numerically_after_the_upstream_tag() {
        let parse = CatalogVersion::parse;
        assert!(parse("winetricks-20260125-r10") > parse("winetricks-20260125-r9"));
        assert!(parse("winetricks-20260125-r1") > parse("winetricks-20260125"));
        assert!(parse("winetricks-20260601") > parse("winetricks-20260125-r10"));
        assert!(!parse("../escape").is_valid());
        let index = CatalogIndex {
            schema: 1,
            releases: ["winetricks-20260125-r9", "winetricks-20260125-r10"]
                .into_iter()
                .map(|version| CatalogRelease {
                    version: version.into(),
                    upstream_tag: "20260125".into(),
                    url: Url::parse("https://updates.example.test/c.tar.zst").unwrap(),
                    sha256: "00".repeat(32),
                    signature: String::new(),
                    recipe_count: 1,
                })
                .collect(),
        };
        assert_eq!(
            index
                .latest_after("winetricks-20260125-r2")
                .unwrap()
                .version,
            "winetricks-20260125-r10"
        );
        assert!(index.latest_after("winetricks-20260125-r10").is_none());
    }

    #[test]
    fn rejects_path_like_versions() {
        let signing_key = SigningKey::from_bytes(&[9; 32]);
        let mut release = release(&signing_key);
        release.version = "../escape".into();
        assert!(validate_release(&release).is_err());
    }

    #[tokio::test]
    async fn verifies_activates_and_rolls_back_a_signed_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let initial = temp.path().join("initial");
        std::fs::create_dir(&initial).unwrap();
        std::fs::write(
            initial.join("initial.toml"),
            "schema=1\nid=\"initial\"\ncategory=\"settings\"\ntitle=\"Initial\"\n[source]\nupstream_tag=\"20260125\"\nupstream_verb=\"initial\"\n",
        )
        .unwrap();
        let catalog = Catalog::load(
            CatalogSource {
                path: initial.clone(),
                version: "winetricks-20260125".into(),
                upstream_tag: "20260125".into(),
            },
            paths.winetricks_cache.clone(),
        )
        .unwrap();
        let store = Arc::new(Store::open(&paths).unwrap());
        store
            .activate_catalog_version("winetricks-20260125", "20260125", &initial, None)
            .unwrap();
        let signing_key = SigningKey::from_bytes(&[11; 32]);
        let updater = CatalogUpdater::new(
            paths.clone(),
            catalog.clone(),
            store.clone(),
            signing_key.verifying_key().to_bytes(),
        )
        .unwrap();
        let bytes = catalog_bundle();
        let release = signed_release(&signing_key, &bytes);

        let installed = updater.install_bytes(&release, bytes).await.unwrap();

        assert!(installed.active);
        assert!(installed.path.starts_with(&paths.catalogs));
        assert_eq!(catalog.summary().version, "winetricks-20990101");
        assert_eq!(catalog.get("good").unwrap().title, "Good");
        assert_eq!(
            store.active_catalog_version().unwrap().unwrap().version,
            installed.version
        );

        // A replayed (not newer) release is refused.
        assert!(matches!(
            updater.install(&release).await,
            Err(BettertricksError::Security(_))
        ));

        let rolled_back = updater.rollback().unwrap();
        assert!(store.active_catalog_pinned().unwrap());
        assert_eq!(rolled_back.version, "winetricks-20260125");
        assert_eq!(catalog.summary().version, "winetricks-20260125");
        assert_eq!(catalog.get("initial").unwrap().title, "Initial");
        assert_eq!(
            store.active_catalog_version().unwrap().unwrap().version,
            "winetricks-20260125"
        );

        // Re-installing the same version name with different bytes must not activate the
        // stale directory.
        let mut tampered = catalog_bundle_with_title("Changed");
        let changed = signed_release(&signing_key, &tampered);
        assert!(matches!(
            updater
                .install_bytes(&changed, std::mem::take(&mut tampered))
                .await,
            Err(BettertricksError::Security(_))
        ));
        assert_eq!(catalog.summary().version, "winetricks-20260125");
        // Identical bytes re-activate the existing directory.
        let bytes = catalog_bundle();
        let same = signed_release(&signing_key, &bytes);
        updater.install_bytes(&same, bytes).await.unwrap();
        assert_eq!(catalog.get("good").unwrap().title, "Good");
    }
}
