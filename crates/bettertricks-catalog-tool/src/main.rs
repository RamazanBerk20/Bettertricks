use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bettertricks_core::{Catalog, CatalogIndex, CatalogRelease, CatalogSource, CatalogSummary};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use url::Url;
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[command(
    name = "bettertricks-catalog",
    about = "Validate, package, and sign Bettertricks catalogs"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Validate {
        #[arg(default_value = "catalog")]
        catalog: PathBuf,
    },
    Bundle {
        #[arg(long, default_value = "catalog")]
        catalog: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        url: Url,
        #[arg(long)]
        signing_key: PathBuf,
    },
    Index {
        #[arg(long, required = true)]
        release: Vec<PathBuf>,
        #[arg(long)]
        output: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("bettertricks-catalog: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { catalog } => {
            let (catalog, manifest) = load_catalog(&catalog)?;
            let summary = catalog.summary();
            validate_manifest(&summary, &manifest)?;
            println!(
                "{}: {} recipes ({} native, {} tracked ports)",
                summary.version,
                summary.recipe_count,
                summary.native_count,
                summary.recipe_count - summary.native_count
            );
        }
        Command::Bundle {
            catalog,
            output,
            url,
            signing_key,
        } => bundle(&catalog, &output, url, &signing_key)?,
        Command::Index { release, output } => build_index(&release, &output)?,
    }
    Ok(())
}

fn bundle(catalog_path: &Path, output: &Path, url: Url, key_path: &Path) -> Result<()> {
    if url.scheme() != "https" {
        bail!("release URL must use HTTPS");
    }
    let (catalog, manifest) = load_catalog(catalog_path)?;
    let summary = catalog.summary();
    validate_manifest(&summary, &manifest)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    let partial = output.with_file_name(format!(
        "{}.part",
        output.file_name().unwrap_or_default().to_string_lossy()
    ));
    write_deterministic_bundle(catalog_path, &partial)?;
    std::fs::rename(&partial, output)
        .with_context(|| format!("activate bundle {}", output.display()))?;

    let mut archive = Vec::new();
    File::open(output)?.read_to_end(&mut archive)?;
    let mut release = CatalogRelease {
        version: summary.version,
        upstream_tag: summary.upstream_tag,
        url,
        sha256: hex::encode(Sha256::digest(&archive)),
        signature: String::new(),
        recipe_count: summary.recipe_count,
    };
    let signing_key = read_signing_key(key_path)?;
    release.signature = hex::encode(signing_key.sign(&release.signing_payload()).to_bytes());
    let descriptor_path = output.with_extension("release.json");
    std::fs::write(&descriptor_path, serde_json::to_vec_pretty(&release)?)?;
    println!("Wrote {}", output.display());
    println!("Wrote {}", descriptor_path.display());
    Ok(())
}

fn build_index(release_paths: &[PathBuf], output: &Path) -> Result<()> {
    let mut releases = Vec::new();
    for path in release_paths {
        let release: CatalogRelease = serde_json::from_slice(&std::fs::read(path)?)
            .with_context(|| format!("parse {}", path.display()))?;
        releases.push(release);
    }
    releases.sort_by(|left, right| right.version.cmp(&left.version));
    let index = CatalogIndex {
        schema: 1,
        releases,
    };
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, serde_json::to_vec_pretty(&index)?)?;
    println!("Wrote {}", output.display());
    Ok(())
}

fn load_catalog(path: &Path) -> Result<(Catalog, serde_json::Value)> {
    let manifest_path = path.join("manifest.json");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?,
    )?;
    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .context("manifest has no version")?
        .to_string();
    let upstream_tag = manifest
        .get("upstreamTag")
        .and_then(serde_json::Value::as_str)
        .context("manifest has no upstreamTag")?
        .to_string();
    let catalog = Catalog::load(
        CatalogSource {
            path: path.to_path_buf(),
            version,
            upstream_tag,
        },
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".bettertricks-catalog-cache-probe"),
    )?;
    Ok((catalog, manifest))
}

fn validate_manifest(summary: &CatalogSummary, manifest: &serde_json::Value) -> Result<()> {
    let schema = manifest
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .context("manifest has no numeric schema")?;
    if schema != 1 {
        bail!("manifest schema must be 1, found {schema}");
    }
    if summary.version != format!("winetricks-{}", summary.upstream_tag) {
        bail!(
            "manifest version {} does not match upstream tag {}",
            summary.version,
            summary.upstream_tag
        );
    }

    let native = manifest
        .get("nativeRecipes")
        .and_then(serde_json::Value::as_u64)
        .context("manifest has no numeric nativeRecipes")?;
    let generated = manifest
        .get("generatedRecipes")
        .and_then(serde_json::Value::as_u64)
        .context("manifest has no numeric generatedRecipes")?;
    if native != summary.native_count as u64 {
        bail!(
            "manifest declares {native} native recipes, but validation found {}",
            summary.native_count
        );
    }
    let tracked = summary.recipe_count - summary.native_count;
    if generated != tracked as u64 {
        bail!("manifest declares {generated} tracked recipes, but validation found {tracked}");
    }

    let categories = manifest
        .get("categories")
        .and_then(serde_json::Value::as_object)
        .context("manifest has no categories object")?;
    if categories.len() != summary.categories.len() {
        bail!("manifest category set does not match the catalog");
    }
    for (category, count) in &summary.categories {
        let declared = categories
            .get(category)
            .and_then(serde_json::Value::as_u64)
            .with_context(|| format!("manifest has no numeric category count for {category}"))?;
        if declared != *count as u64 {
            bail!("manifest declares {declared} {category} recipes, but validation found {count}");
        }
    }
    Ok(())
}

fn write_deterministic_bundle(catalog: &Path, output: &Path) -> Result<()> {
    let file = BufWriter::new(File::create(output)?);
    let encoder = zstd::Encoder::new(file, 19)?;
    let mut archive = tar::Builder::new(encoder.auto_finish());
    let mut files = WalkDir::new(catalog)
        .follow_links(false)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    files.sort_by(|left, right| left.path().cmp(right.path()));
    for entry in files {
        if entry.file_type().is_symlink() {
            bail!(
                "catalog cannot contain symlinks: {}",
                entry.path().display()
            );
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(catalog)?;
        let name = Path::new("catalog").join(relative);
        let mut input = BufReader::new(File::open(entry.path())?);
        let size = input.get_ref().metadata()?.len();
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        archive.append_data(&mut header, name, &mut input)?;
    }
    archive.finish()?;
    Ok(())
}

fn read_signing_key(path: &Path) -> Result<SigningKey> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let decoded = match std::str::from_utf8(&bytes) {
        Ok(value) if value.trim().len() == 64 => hex::decode(value.trim())?,
        _ => bytes,
    };
    let key: [u8; 32] = decoded.try_into().map_err(|_| {
        anyhow::anyhow!("signing key must be 32 raw bytes or 64 hexadecimal characters")
    })?;
    Ok(SigningKey::from_bytes(&key))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn summary() -> CatalogSummary {
        CatalogSummary {
            version: "winetricks-20260125".into(),
            upstream_tag: "20260125".into(),
            recipe_count: 3,
            native_count: 2,
            metadata_only_count: 1,
            categories: BTreeMap::from([("settings".into(), 3)]),
        }
    }

    fn manifest() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "version": "winetricks-20260125",
            "upstreamTag": "20260125",
            "generatedRecipes": 1,
            "nativeRecipes": 2,
            "categories": { "settings": 3 }
        })
    }

    #[test]
    fn accepts_matching_manifest_counts() {
        validate_manifest(&summary(), &manifest()).unwrap();
    }

    #[test]
    fn rejects_stale_manifest_counts() {
        let mut manifest = manifest();
        manifest["nativeRecipes"] = serde_json::json!(1);
        assert!(validate_manifest(&summary(), &manifest).is_err());
    }
}
