use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use walkdir::WalkDir;

use crate::{
    BettertricksError, CatalogQuery, CatalogSummary, PrefixArchitecture, Recipe, RecipeListItem,
    RecipeMaturity, Result, WinePrefix,
};

#[derive(Debug, Clone)]
pub struct CatalogSource {
    pub path: PathBuf,
    pub version: String,
    pub upstream_tag: String,
}

#[derive(Clone)]
pub struct Catalog {
    inner: Arc<RwLock<CatalogInner>>,
    cache_directory: PathBuf,
}

struct CatalogInner {
    source: CatalogSource,
    recipes: HashMap<String, Recipe>,
}

const MAX_MANUAL_FILE_BYTES: u64 = 16 * 1024 * 1024 * 1024;

impl Catalog {
    pub fn load(source: CatalogSource, cache_directory: PathBuf) -> Result<Self> {
        let recipes = load_recipes(&source.path)?;
        if recipes.is_empty() {
            return Err(BettertricksError::Catalog(format!(
                "no recipes found in {}",
                source.path.display()
            )));
        }
        validate_catalog_recipes(&recipes, &source)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(CatalogInner { source, recipes })),
            cache_directory,
        })
    }

    pub fn reload(&self, source: CatalogSource) -> Result<()> {
        let recipes = load_recipes(&source.path)?;
        if recipes.is_empty() {
            return Err(BettertricksError::Catalog(
                "refusing to activate an empty catalog".into(),
            ));
        }
        validate_catalog_recipes(&recipes, &source)?;
        *self.inner.write() = CatalogInner { source, recipes };
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Recipe> {
        self.inner
            .read()
            .recipes
            .get(id)
            .cloned()
            .ok_or_else(|| BettertricksError::RecipeNotFound(id.into()))
    }

    pub fn contains(&self, id: &str) -> bool {
        self.inner.read().recipes.contains_key(id)
    }

    pub fn all(&self) -> Vec<Recipe> {
        let mut recipes: Vec<_> = self.inner.read().recipes.values().cloned().collect();
        recipes.sort_by(|left, right| left.id.cmp(&right.id));
        recipes
    }

    pub fn summary(&self) -> CatalogSummary {
        let inner = self.inner.read();
        let mut categories = BTreeMap::new();
        let mut native_count = 0;
        let mut metadata_only_count = 0;
        for recipe in inner.recipes.values() {
            *categories
                .entry(recipe.category.as_str().to_string())
                .or_insert(0) += 1;
            match recipe.maturity {
                RecipeMaturity::Native => native_count += 1,
                RecipeMaturity::MetadataOnly => metadata_only_count += 1,
                RecipeMaturity::BrokenUpstream => {}
            }
        }
        CatalogSummary {
            version: inner.source.version.clone(),
            upstream_tag: inner.source.upstream_tag.clone(),
            recipe_count: inner.recipes.len(),
            native_count,
            metadata_only_count,
            categories,
        }
    }

    pub fn search(&self, query: &CatalogQuery, prefix: Option<&WinePrefix>) -> Vec<RecipeListItem> {
        let search = query
            .search
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let installed = prefix
            .map(|prefix| prefix.installed_verbs.as_slice())
            .unwrap_or_default();
        let inner = self.inner.read();

        let mut results: Vec<_> = inner
            .recipes
            .values()
            .filter(|recipe| {
                query
                    .category
                    .is_none_or(|category| recipe.category == category)
            })
            .filter(|recipe| query.media.is_none_or(|media| recipe.media == media))
            .filter(|recipe| {
                search.is_empty()
                    || recipe.id.contains(&search)
                    || recipe.title.to_ascii_lowercase().contains(&search)
                    || recipe
                        .publisher
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&search)
                    || recipe
                        .tags
                        .iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(&search))
            })
            .filter_map(|recipe| {
                let is_installed = installed.iter().any(|item| item == &recipe.id);
                let cached = self.recipe_cached(recipe);
                let (compatible, reason) = compatibility(recipe, prefix);
                if query.installed_only && !is_installed
                    || query.cached_only && !cached
                    || query.compatible_only && !compatible
                {
                    return None;
                }
                Some(RecipeListItem {
                    id: recipe.id.clone(),
                    category: recipe.category,
                    title: recipe.title.clone(),
                    publisher: recipe.publisher.clone(),
                    year: recipe.year.clone(),
                    description: recipe.description.clone(),
                    media: recipe.media,
                    maturity: recipe.maturity,
                    tags: recipe.tags.clone(),
                    installed: is_installed,
                    cached,
                    compatible,
                    compatibility_reason: reason,
                })
            })
            .collect();

        results.sort_by(|left, right| {
            left.category
                .as_str()
                .cmp(right.category.as_str())
                .then_with(|| {
                    left.title
                        .to_ascii_lowercase()
                        .cmp(&right.title.to_ascii_lowercase())
                })
        });
        results
    }

    pub fn recipe_cached(&self, recipe: &Recipe) -> bool {
        recipe
            .files
            .iter()
            .all(|file| self.cache_path(&recipe.id, file).is_file())
    }

    pub fn cache_path(&self, recipe_id: &str, file: &crate::RecipeFile) -> PathBuf {
        file.cache_path
            .as_deref()
            .map(|path| self.cache_directory.join(path))
            .unwrap_or_else(|| self.cache_directory.join(recipe_id).join(&file.filename))
    }

    pub async fn import_manual_file(
        &self,
        recipe_id: &str,
        file_id: &str,
        source: &Path,
    ) -> Result<PathBuf> {
        let recipe = self.get(recipe_id)?;
        if recipe.maturity != RecipeMaturity::Native {
            return Err(BettertricksError::Unsupported(format!(
                "{} is not an executable native recipe",
                recipe.id
            )));
        }
        let file = recipe
            .files
            .iter()
            .find(|file| file.id == file_id)
            .ok_or_else(|| BettertricksError::Recipe(format!("missing file {file_id}")))?;
        if !file.manual {
            return Err(BettertricksError::Security(format!(
                "{} is not declared as a manual download",
                file.filename
            )));
        }
        let expected = file.sha256.as_deref().ok_or_else(|| {
            BettertricksError::Recipe(format!("{} has no expected checksum", file.filename))
        })?;
        let source = tokio::fs::canonicalize(source).await?;
        let metadata = tokio::fs::metadata(&source).await?;
        if !metadata.is_file() {
            return Err(BettertricksError::Security(
                "the selected manual download is not a regular file".into(),
            ));
        }
        if metadata.len() > MAX_MANUAL_FILE_BYTES {
            return Err(BettertricksError::Security(format!(
                "{} exceeds the 16 GiB recipe-file limit",
                source.display()
            )));
        }

        let destination = self.cache_path(recipe_id, file);
        if source == destination {
            verify_file_sha256(&source, expected).await?;
            return Ok(destination);
        }
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let partial = destination.with_file_name(format!(
            ".{}.bettertricks-import-{}.partial",
            file.filename,
            uuid::Uuid::new_v4()
        ));
        let result = async {
            let mut input = tokio::fs::File::open(&source).await?;
            let mut output = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&partial)
                .await?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; 1024 * 1024];
            let mut copied = 0_u64;
            loop {
                let count = input.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                copied = copied.saturating_add(count as u64);
                if copied > MAX_MANUAL_FILE_BYTES {
                    return Err(BettertricksError::Security(
                        "manual download exceeded the 16 GiB recipe-file limit".into(),
                    ));
                }
                digest.update(&buffer[..count]);
                output.write_all(&buffer[..count]).await?;
            }
            output.flush().await?;
            output.sync_all().await?;
            drop(output);
            let actual = hex::encode(digest.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(BettertricksError::ChecksumMismatch {
                    file: source.display().to_string(),
                    expected: expected.into(),
                    actual,
                });
            }
            tokio::fs::rename(&partial, &destination).await?;
            Ok(destination.clone())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&partial).await;
        }
        result
    }
}

async fn verify_file_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut input = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = hex::encode(digest.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(BettertricksError::ChecksumMismatch {
            file: path.display().to_string(),
            expected: expected.into(),
            actual,
        })
    }
}

fn load_recipes(root: &Path) -> Result<HashMap<String, Recipe>> {
    let mut recipes = HashMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| BettertricksError::Catalog(error.to_string()))?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("toml")
        {
            continue;
        }
        let content = std::fs::read_to_string(entry.path())?;
        let recipe: Recipe = toml::from_str(&content).map_err(|error| {
            BettertricksError::Recipe(format!("{}: {error}", entry.path().display()))
        })?;
        recipe.validate().map_err(BettertricksError::Recipe)?;
        let id = recipe.id.clone();
        if entry.path().file_stem().and_then(|value| value.to_str()) != Some(id.as_str()) {
            return Err(BettertricksError::Recipe(format!(
                "{} does not match recipe id {id}",
                entry.path().display()
            )));
        }
        if recipes.insert(id.clone(), recipe).is_some() {
            return Err(BettertricksError::Recipe(format!("duplicate recipe {id}")));
        }
    }
    Ok(recipes)
}

fn validate_catalog_recipes(
    recipes: &HashMap<String, Recipe>,
    source: &CatalogSource,
) -> Result<()> {
    let mut shared_cache_files: HashMap<String, (String, Option<String>, bool)> = HashMap::new();
    for recipe in recipes.values() {
        if recipe.source.upstream_tag != source.upstream_tag {
            return Err(BettertricksError::Recipe(format!(
                "recipe {} targets upstream {}, catalog targets {}",
                recipe.id, recipe.source.upstream_tag, source.upstream_tag
            )));
        }
        for dependency in recipe
            .dependencies
            .iter()
            .map(String::as_str)
            .chain(called_recipes(&recipe.steps))
        {
            if dependency == recipe.id {
                return Err(BettertricksError::Recipe(format!(
                    "recipe {} depends on itself",
                    recipe.id
                )));
            }
            if !recipes.contains_key(dependency) {
                return Err(BettertricksError::Recipe(format!(
                    "recipe {} depends on missing recipe {dependency}",
                    recipe.id
                )));
            }
        }
        for conflict in &recipe.conflicts {
            if !recipes.contains_key(conflict) {
                return Err(BettertricksError::Recipe(format!(
                    "recipe {} conflicts with missing recipe {conflict}",
                    recipe.id
                )));
            }
        }
        for file in &recipe.files {
            let Some(cache_path) = &file.cache_path else {
                continue;
            };
            let definition = (file.filename.clone(), file.sha256.clone(), file.manual);
            if let Some(existing) =
                shared_cache_files.insert(cache_path.clone(), definition.clone())
                && existing != definition
            {
                return Err(BettertricksError::Recipe(format!(
                    "shared cache path {cache_path} has conflicting file definitions"
                )));
            }
        }
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    for id in recipes.keys() {
        validate_dependency_tree(id, recipes, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn validate_dependency_tree(
    id: &str,
    recipes: &HashMap<String, Recipe>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_owned()) {
        return Err(BettertricksError::Recipe(format!(
            "dependency cycle includes {id}"
        )));
    }
    let recipe = &recipes[id];
    for dependency in recipe
        .dependencies
        .iter()
        .map(String::as_str)
        .chain(called_recipes(&recipe.steps))
    {
        validate_dependency_tree(dependency, recipes, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    Ok(())
}

fn called_recipes(steps: &[crate::RecipeStep]) -> Vec<&str> {
    steps
        .iter()
        .flat_map(|step| match step {
            crate::RecipeStep::Call { recipe } => vec![recipe.as_str()],
            crate::RecipeStep::On64BitPrefix { steps } => called_recipes(steps),
            _ => Vec::new(),
        })
        .collect()
}

fn compatibility(recipe: &Recipe, prefix: Option<&WinePrefix>) -> (bool, Option<String>) {
    if let Some(reason) = &recipe.constraints.broken_reason {
        return (false, Some(reason.clone()));
    }
    let Some(prefix) = prefix else {
        return (true, None);
    };
    if !recipe.constraints.architectures.is_empty()
        && prefix.architecture != PrefixArchitecture::Unknown
        && !recipe
            .constraints
            .architectures
            .contains(&prefix.architecture)
    {
        return (
            false,
            Some(format!(
                "Requires {:?} architecture",
                recipe.constraints.architectures
            )),
        );
    }
    (true, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &Path) -> CatalogSource {
        CatalogSource {
            path: path.to_path_buf(),
            version: "test".into(),
            upstream_tag: "test".into(),
        }
    }

    #[test]
    fn rejects_empty_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let result = Catalog::load(source(temp.path()), temp.path().join("cache"));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn imports_manual_files_only_after_checksum_verification() {
        let temp = tempfile::tempdir().unwrap();
        let recipes = temp.path().join("recipes");
        let cache = temp.path().join("cache");
        std::fs::create_dir(&recipes).unwrap();
        std::fs::write(
            recipes.join("manual.toml"),
            r#"
schema = 1
id = "manual"
category = "apps"
title = "Manual"
media = "manual_download"
maturity = "native"

[[files]]
id = "installer"
filename = "installer.exe"
sha256 = "93413c6ce1e4603c4adaa057e4ad0ce316e72589ba485ecc573e4a1331706139"
manual = true

[[steps]]
type = "download"
file = "installer"

[source]
upstream_tag = "test"
upstream_verb = "manual"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(source(&recipes), cache.clone()).unwrap();
        let selected = temp.path().join("selected.exe");
        std::fs::write(&selected, b"wrong").unwrap();
        assert!(
            catalog
                .import_manual_file("manual", "installer", &selected)
                .await
                .is_err()
        );
        assert!(!cache.join("manual/installer.exe").exists());

        std::fs::write(&selected, b"bettertricks").unwrap();
        let imported = catalog
            .import_manual_file("manual", "installer", &selected)
            .await
            .unwrap();
        assert_eq!(imported, cache.join("manual/installer.exe"));
        assert_eq!(std::fs::read(imported).unwrap(), b"bettertricks");
        assert!(
            std::fs::read_dir(cache.join("manual"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("partial"))
        );
    }

    #[test]
    fn rejects_missing_dependencies_at_load_time() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("top.toml"),
            r#"
schema = 1
id = "top"
category = "dlls"
title = "Top"
dependencies = ["missing"]

[source]
upstream_tag = "test"
upstream_verb = "top"
"#,
        )
        .unwrap();

        assert!(Catalog::load(source(temp.path()), temp.path().join("cache")).is_err());
    }

    #[test]
    fn rejects_missing_dependencies_inside_architecture_groups() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("top.toml"),
            r#"
schema = 1
id = "top"
category = "dlls"
title = "Top"
maturity = "native"

[[steps]]
type = "on_64_bit_prefix"

[[steps.steps]]
type = "call"
recipe = "missing"

[source]
upstream_tag = "test"
upstream_verb = "top"
"#,
        )
        .unwrap();

        assert!(Catalog::load(source(temp.path()), temp.path().join("cache")).is_err());
    }

    #[test]
    fn rejects_dependency_cycles_at_load_time() {
        let temp = tempfile::tempdir().unwrap();
        for (id, dependency) in [("first", "second"), ("second", "first")] {
            std::fs::write(
                temp.path().join(format!("{id}.toml")),
                format!(
                    "schema=1\nid=\"{id}\"\ncategory=\"dlls\"\ntitle=\"{id}\"\ndependencies=[\"{dependency}\"]\n[source]\nupstream_tag=\"test\"\nupstream_verb=\"{id}\"\n"
                ),
            )
            .unwrap();
        }

        assert!(Catalog::load(source(temp.path()), temp.path().join("cache")).is_err());
    }

    #[test]
    fn shared_cache_paths_drive_cache_status_and_must_agree() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(cache.join("shared")).unwrap();
        std::fs::write(cache.join("shared/archive.exe"), b"cached").unwrap();
        for (id, hash) in [("first", "11".repeat(32)), ("second", "11".repeat(32))] {
            std::fs::write(
                temp.path().join(format!("{id}.toml")),
                format!(
                    r#"schema=1
id="{id}"
category="fonts"
title="{id}"
[[files]]
id="archive"
filename="archive.exe"
cache_path="shared/archive.exe"
sha256="{hash}"
manual=false
[source]
upstream_tag="test"
upstream_verb="{id}"
"#,
                ),
            )
            .unwrap();
        }

        let catalog = Catalog::load(source(temp.path()), cache).unwrap();
        assert!(catalog.recipe_cached(&catalog.get("first").unwrap()));

        let second = temp.path().join("second.toml");
        let changed = std::fs::read_to_string(&second)
            .unwrap()
            .replace(&"11".repeat(32), &"22".repeat(32));
        std::fs::write(second, changed).unwrap();
        assert!(Catalog::load(source(temp.path()), temp.path().join("other-cache")).is_err());
    }
}
