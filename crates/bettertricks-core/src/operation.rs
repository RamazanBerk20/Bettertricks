use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use futures_util::StreamExt;
use parking_lot::Mutex;
use regex::Regex;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::planner::{plan_recipe_map, recipe_dependency_ids, step_required_tools};
use crate::{
    AppPaths, BettertricksError, Catalog, DllOverrideMode, LegacyVerbHost, NativeAction,
    OperationEvent, OperationPlan, OperationPrompt, OperationRecord, OperationState, PromptChoice,
    PromptLevel, PromptResponse, Recipe, RecipeFailure, RecipeFailureKind, RecipeFile, RecipeFont,
    RecipeMaturity, RecipeStep, RecoveryManager, RegistryArchitecture, Result, Store, WinePrefix,
};

type PromptSenders = HashMap<(Uuid, Uuid), oneshot::Sender<String>>;
const MAX_RECIPE_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_LIVE_PROCESS_LOG_LINES: usize = 200;
const MAX_PROCESS_LOG_LINE_CHARS: usize = 1_200;
const PROCESS_FAILURE_TAIL_LINES: usize = 8;
const MAX_PROCESS_FAILURE_DETAIL_CHARS: usize = 4_000;

pub trait OperationEventSink: Send + Sync + 'static {
    fn emit(&self, event: OperationEvent);
}

impl<F> OperationEventSink for F
where
    F: Fn(OperationEvent) + Send + Sync + 'static,
{
    fn emit(&self, event: OperationEvent) {
        self(event)
    }
}

#[derive(Clone)]
pub struct OperationEngine {
    catalog: Catalog,
    paths: AppPaths,
    store: Arc<Store>,
    recovery: RecoveryManager,
    legacy_host: LegacyVerbHost,
    cancellations: Arc<Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,
    prefix_locks: Arc<Mutex<HashMap<Uuid, Arc<AsyncMutex<()>>>>>,
    cache_locks: Arc<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>>,
    prompts: Arc<Mutex<PromptSenders>>,
}

impl OperationEngine {
    pub fn new(catalog: Catalog, paths: AppPaths, store: Arc<Store>) -> Self {
        Self {
            recovery: RecoveryManager::new(paths.clone(), store.clone()),
            legacy_host: LegacyVerbHost::discover_with_paths(&paths),
            catalog,
            paths,
            store,
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            prefix_locks: Arc::new(Mutex::new(HashMap::new())),
            cache_locks: Arc::new(Mutex::new(HashMap::new())),
            prompts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self, plan: OperationPlan, sink: Arc<dyn OperationEventSink>) -> Result<Uuid> {
        let id = plan.id;
        let cancellation = Arc::new(AtomicBool::new(false));
        self.cancellations.lock().insert(id, cancellation.clone());

        let record = OperationRecord {
            id,
            prefix_id: plan.prefix.id,
            prefix_name: plan.prefix.name.clone(),
            recipes: plan.resolved_recipes.clone(),
            state: OperationState::Planned,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            current_step: 0,
            total_steps: plan.steps.len(),
            message: Some("Waiting to start".into()),
            failures: Vec::new(),
        };
        self.store.upsert_operation(&record)?;

        let engine = self.clone();
        tokio::spawn(async move {
            let result = engine.run_plan(plan, cancellation, sink.clone()).await;
            if let Err(error) = result {
                tracing::error!(operation_id = %id, %error, "operation failed");
            }
            engine.cancellations.lock().remove(&id);
        });
        Ok(id)
    }

    pub async fn run(&self, plan: OperationPlan, sink: Arc<dyn OperationEventSink>) -> Result<()> {
        let cancellation = Arc::new(AtomicBool::new(false));
        self.cancellations
            .lock()
            .insert(plan.id, cancellation.clone());
        let result = self.run_plan(plan.clone(), cancellation, sink).await;
        self.cancellations.lock().remove(&plan.id);
        result
    }

    pub fn cancel(&self, operation_id: Uuid) -> Result<()> {
        let cancellations = self.cancellations.lock();
        let cancellation = cancellations
            .get(&operation_id)
            .ok_or_else(|| BettertricksError::OperationNotFound(operation_id.to_string()))?;
        cancellation.store(true, Ordering::SeqCst);
        drop(cancellations);

        let prompt_ids = self
            .prompts
            .lock()
            .keys()
            .filter_map(|(candidate_operation_id, prompt_id)| {
                (*candidate_operation_id == operation_id).then_some(*prompt_id)
            })
            .collect::<Vec<_>>();
        for prompt_id in prompt_ids {
            if let Some(sender) = self.prompts.lock().remove(&(operation_id, prompt_id)) {
                let _ = sender.send("cancel".into());
            }
        }
        Ok(())
    }

    pub fn respond(&self, response: PromptResponse) -> Result<()> {
        let sender = self
            .prompts
            .lock()
            .remove(&(response.operation_id, response.prompt_id))
            .ok_or_else(|| BettertricksError::OperationNotFound(response.prompt_id.to_string()))?;
        sender
            .send(response.choice_id)
            .map_err(|_| BettertricksError::Cancelled)
    }

    async fn run_plan(
        &self,
        plan: OperationPlan,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<()> {
        let prefix_lock = {
            let mut locks = self.prefix_locks.lock();
            locks
                .entry(plan.prefix.id)
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let _guard = prefix_lock.lock().await;
        let mut context =
            ExecutionContext::new(plan.clone(), cancellation, sink, self.paths.clone());
        let recipes = match plan_recipe_map(&self.catalog, &plan) {
            Ok(recipes) => recipes,
            Err(error) => return self.finish_error(&mut context, error).await,
        };
        if let Err(error) = validate_plan_inputs(&recipes, &plan) {
            return self.finish_error(&mut context, error).await;
        }

        self.update_record(
            &context,
            OperationState::Preflight,
            Some("Running preflight checks"),
            false,
        )?;
        context.emit(
            OperationState::Preflight,
            "Checking operation safety",
            None,
            None,
            None,
        );

        if !plan.conflicts.is_empty() && !plan.options.force {
            return self
                .finish_error(
                    &mut context,
                    BettertricksError::Conflict("resolve conflicts or enable force mode".into()),
                )
                .await;
        }
        for recipe in recipes.values() {
            if recipe.maturity == RecipeMaturity::BrokenUpstream {
                return self
                    .finish_error(
                        &mut context,
                        BettertricksError::Unsupported(format!(
                            "{} is marked broken upstream and cannot be executed",
                            recipe.id
                        )),
                    )
                    .await;
            }
        }

        if let Some(recipe) = recipes
            .values()
            .find(|recipe| recipe.maturity == RecipeMaturity::MetadataOnly)
            && let Err(error) = self
                .legacy_host
                .require_baseline(&recipe.source.upstream_tag)
                .await
        {
            return self.finish_error(&mut context, error).await;
        }

        let mut required_tools = recipes
            .values()
            .flat_map(|recipe| recipe.steps.iter().chain(&recipe.verify))
            .flat_map(step_required_tools)
            .collect::<HashSet<_>>();
        if plan.options.create_restore_point {
            required_tools.extend(["tar", "zstd"]);
        }
        let mut missing_tools = required_tools
            .into_iter()
            .filter(|tool| crate::system::find_command(tool).is_none())
            .collect::<Vec<_>>();
        missing_tools.sort_unstable();
        if !missing_tools.is_empty() {
            return self
                .finish_error(
                    &mut context,
                    BettertricksError::Unsupported(format!(
                        "missing required tool(s): {}",
                        missing_tools.join(", ")
                    )),
                )
                .await;
        }

        if !plan.prefix.exists
            && let Err(error) = initialize_prefix(&plan.prefix, &context).await
        {
            return self.finish_error(&mut context, error).await;
        }

        if plan.options.create_restore_point && plan.prefix.exists {
            context.emit(
                OperationState::Preflight,
                "Creating a restore point",
                Some("The operation starts after recovery data is safe.".into()),
                None,
                None,
            );
            if let Err(error) = self.recovery.create(&plan.prefix, Some(plan.id)).await {
                return self.finish_error(&mut context, error).await;
            }
        }

        context.started_at = Some(Utc::now());
        self.update_record(
            &context,
            OperationState::Running,
            Some("Operation running"),
            false,
        )?;
        context.emit(
            OperationState::Running,
            "Operation started",
            None,
            Some(0.0),
            None,
        );

        let mut unsuccessful_recipes = HashSet::new();
        let mut successful_recipes = 0usize;
        for recipe_id in &plan.resolved_recipes {
            let Some(recipe) = recipes.get(recipe_id) else {
                return self
                    .finish_error(
                        &mut context,
                        BettertricksError::RecipeNotFound(recipe_id.clone()),
                    )
                    .await;
            };
            context.recipe_id = Some(recipe.id.clone());
            let recipe_start_step = context.step;
            let recipe_step_count = plan
                .steps
                .iter()
                .filter(|step| step.recipe_id == recipe.id)
                .count();

            if let Some(blocked_by) = recipe_dependency_ids(recipe)
                .into_iter()
                .find(|dependency| unsuccessful_recipes.contains(*dependency))
            {
                context.step = recipe_start_step
                    .saturating_add(recipe_step_count)
                    .min(plan.steps.len());
                let dependency_title = recipes
                    .get(blocked_by)
                    .map(|dependency| dependency.title.as_str())
                    .unwrap_or(blocked_by);
                let failure = RecipeFailure {
                    recipe_id: recipe.id.clone(),
                    recipe_title: recipe.title.clone(),
                    kind: RecipeFailureKind::SkippedDependency,
                    message: format!(
                        "Skipped because dependency {dependency_title} ({blocked_by}) did not complete successfully."
                    ),
                };
                context.log(format!("[{}] SKIPPED: {}", recipe.id, failure.message));
                context.emit_failure(failure);
                unsuccessful_recipes.insert(recipe.id.clone());
                self.update_record(
                    &context,
                    OperationState::Running,
                    Some("Continuing after a dependency failure"),
                    false,
                )?;
                continue;
            }

            match self.execute_recipe(recipe, &mut context).await {
                Ok(()) => successful_recipes += 1,
                Err(error) if matches!(&error, BettertricksError::Cancelled) => {
                    return self.finish_cancelled(&mut context).await;
                }
                Err(error) if operation_execution_error_is_fatal(&error) => {
                    return self.finish_error(&mut context, error).await;
                }
                Err(error) => {
                    context.step = recipe_start_step
                        .saturating_add(recipe_step_count)
                        .min(plan.steps.len());
                    let failure = RecipeFailure {
                        recipe_id: recipe.id.clone(),
                        recipe_title: recipe.title.clone(),
                        kind: RecipeFailureKind::Failed,
                        message: error.to_string(),
                    };
                    context.log(format!("[{}] FAILED: {}", recipe.id, failure.message));
                    context.emit_failure(failure);
                    unsuccessful_recipes.insert(recipe.id.clone());
                    self.update_record(
                        &context,
                        OperationState::Running,
                        Some("Continuing after a component failure"),
                        false,
                    )?;
                }
            }
        }

        context.recipe_id = None;
        context.finished_at = Some(Utc::now());
        if context.failures.is_empty() {
            context.emit(
                OperationState::Succeeded,
                "Operation complete",
                Some(format!("Applied {successful_recipes} recipe(s)")),
                Some(1.0),
                None,
            );
            self.update_record(
                &context,
                OperationState::Succeeded,
                Some("Operation complete"),
                true,
            )?;
            Ok(())
        } else {
            let failed = context
                .failures
                .iter()
                .filter(|failure| failure.kind == RecipeFailureKind::Failed)
                .count();
            let skipped = context.failures.len() - failed;
            let summary = format_operation_failure_summary(successful_recipes, failed, skipped);
            context.log(summary.clone());
            context.emit(
                OperationState::Failed,
                "Completed with failures",
                Some(summary.clone()),
                Some(1.0),
                None,
            );
            self.update_record(&context, OperationState::Failed, Some(&summary), true)?;
            Err(BettertricksError::Recipe(summary))
        }
    }

    async fn execute_recipe(&self, recipe: &Recipe, context: &mut ExecutionContext) -> Result<()> {
        if recipe.maturity == RecipeMaturity::MetadataOnly {
            if context.cancelled() {
                return Err(BettertricksError::Cancelled);
            }
            context.step += 1;
            let title = format!("Run {} through Winetricks", recipe.id);
            context.emit(
                OperationState::Running,
                &title,
                Some(recipe.title.clone()),
                Some(context.fraction()),
                None,
            );
            self.update_record(context, OperationState::Running, Some(&title), false)?;
            self.run_legacy_recipe(recipe, context).await?;
        }

        for step in &recipe.steps {
            if context.cancelled() {
                return Err(BettertricksError::Cancelled);
            }
            context.step += 1;
            let title = crate::planner::step_label(step);
            context.emit(
                OperationState::Running,
                &title,
                Some(recipe.title.clone()),
                Some(context.fraction()),
                None,
            );
            self.update_record(context, OperationState::Running, Some(&title), false)?;
            self.execute_step(recipe, step, context).await?;
        }

        if context.plan.options.verify && recipe.maturity == RecipeMaturity::Native {
            for step in &recipe.verify {
                context.log(format!(
                    "[{}] Verifying: {}",
                    recipe.id,
                    crate::planner::step_label(step)
                ));
                self.execute_step(recipe, step, context).await?;
            }
        }
        append_installed_log(&context.plan.prefix.path, &recipe.id).await
    }

    async fn run_legacy_recipe(
        &self,
        recipe: &Recipe,
        context: &mut ExecutionContext,
    ) -> Result<()> {
        context.log(format!(
            "Using Winetricks {} compatibility host for {}",
            recipe.source.upstream_tag, recipe.id
        ));
        let mut command = self.legacy_host.recipe_command(
            &recipe.id,
            &recipe.source.upstream_tag,
            &context.plan.prefix,
            &context.plan.options,
        )?;
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let (sender, mut receiver) = mpsc::channel(128);
        let stdout_task = child
            .stdout
            .take()
            .map(|stdout| tokio::spawn(forward_process_output(stdout, "stdout", sender.clone())));
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(forward_process_output(stderr, "stderr", sender.clone())));
        drop(sender);

        let mut tail = VecDeque::with_capacity(PROCESS_FAILURE_TAIL_LINES);
        let mut emitted_lines = 0usize;
        let mut suppressed_lines = 0usize;
        let status = loop {
            while let Ok(line) = receiver.try_recv() {
                record_process_output(
                    context,
                    recipe,
                    line,
                    &mut tail,
                    &mut emitted_lines,
                    &mut suppressed_lines,
                );
            }
            if context.cancelled() {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(BettertricksError::Cancelled);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            tokio::select! {
                Some(line) = receiver.recv() => {
                    record_process_output(
                        context,
                        recipe,
                        line,
                        &mut tail,
                        &mut emitted_lines,
                        &mut suppressed_lines,
                    );
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        };

        let drain_timed_out = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while let Some(line) = receiver.recv().await {
                record_process_output(
                    context,
                    recipe,
                    line,
                    &mut tail,
                    &mut emitted_lines,
                    &mut suppressed_lines,
                );
            }
        })
        .await
        .is_err();
        if drain_timed_out {
            if let Some(task) = &stdout_task {
                task.abort();
            }
            if let Some(task) = &stderr_task {
                task.abort();
            }
            context.log(format!(
                "[{}] Stopped collecting output one second after Winetricks exited because a child process kept the output pipe open.",
                recipe.id
            ));
        }
        for task in [stdout_task, stderr_task].into_iter().flatten() {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => context.log(format!(
                    "[{}] Could not read all process output: {error}",
                    recipe.id
                )),
                Err(error) if error.is_cancelled() => {}
                Err(error) => context.log(format!(
                    "[{}] Process output reader stopped unexpectedly: {error}",
                    recipe.id
                )),
            }
        }
        if suppressed_lines > 0 {
            context.log(format!(
                "[{}] {suppressed_lines} additional process output line(s) were omitted to keep the activity view responsive.",
                recipe.id
            ));
        }
        if status.success() {
            return Ok(());
        }
        let detail = if tail.is_empty() {
            "Winetricks produced no diagnostic output.".into()
        } else {
            truncate_text(
                &format!(
                    "Last output: {}",
                    tail.into_iter().collect::<Vec<_>>().join(" | ")
                ),
                MAX_PROCESS_FAILURE_DETAIL_CHARS,
            )
        };
        Err(BettertricksError::CommandFailedWithOutput {
            program: format!("winetricks ({})", recipe.id),
            status: status
                .code()
                .map(|code| format!("exited with code {code}"))
                .unwrap_or_else(|| "was terminated by a signal".into()),
            detail,
        })
    }

    async fn execute_step(
        &self,
        recipe: &Recipe,
        step: &RecipeStep,
        context: &mut ExecutionContext,
    ) -> Result<()> {
        match step {
            RecipeStep::Download { file } => {
                let definition = recipe
                    .files
                    .iter()
                    .find(|candidate| &candidate.id == file)
                    .ok_or_else(|| BettertricksError::Recipe(format!("missing file {file}")))?;
                self.download(recipe, definition, context).await
            }
            RecipeStep::EnsureDirectory { path } => {
                let path = context.expand_path(recipe, path)?;
                ensure_mutation_in_prefix(
                    &path,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                tokio::fs::create_dir_all(path).await?;
                Ok(())
            }
            RecipeStep::EnsureFile { path } => {
                let path = context.expand_path(recipe, path)?;
                ensure_mutation_in_prefix(
                    &path,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                ensure_file(&path).await
            }
            RecipeStep::VerifyPath { path, kind } => {
                let path = context.expand_path(recipe, path)?;
                let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
                    BettertricksError::Recipe(format!(
                        "verification failed for {}: {error}",
                        path.display()
                    ))
                })?;
                let matches = match kind {
                    crate::DetectKind::File => metadata.is_file(),
                    crate::DetectKind::Directory => metadata.is_dir(),
                };
                if matches {
                    Ok(())
                } else {
                    Err(BettertricksError::Recipe(format!(
                        "verification found the wrong file type at {}",
                        path.display()
                    )))
                }
            }
            RecipeStep::Copy { from, to } => {
                let from = context.expand_path(recipe, from)?;
                let from = ensure_read_source_in_prefix_or_cache(
                    &from,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                let to = context.expand_path(recipe, to)?;
                ensure_mutation_in_prefix(
                    &to,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                copy_path(&from, &to).await
            }
            RecipeStep::Move { from, to } => {
                let from = context.expand_path(recipe, from)?;
                let to = context.expand_path(recipe, to)?;
                ensure_mutation_in_prefix(
                    &from,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                ensure_mutation_in_prefix(
                    &to,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                if let Some(parent) = to.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::rename(from, to).await?;
                Ok(())
            }
            RecipeStep::Remove { path, recursive } => {
                let path = context.expand_path(recipe, path)?;
                ensure_mutation_in_prefix(&path, &context.plan.prefix.path, &context.paths.cache)?;
                if path.is_dir() && *recursive {
                    tokio::fs::remove_dir_all(path).await?;
                } else if path.exists() {
                    tokio::fs::remove_file(path).await?;
                }
                Ok(())
            }
            RecipeStep::RemoveSymlink { path } => {
                let path = context.expand_path(recipe, path)?;
                remove_symlink_in_prefix(&path, &context.plan.prefix.path).await
            }
            RecipeStep::Extract {
                file,
                destination,
                format,
                include,
            } => {
                let definition = recipe
                    .files
                    .iter()
                    .find(|candidate| &candidate.id == file)
                    .ok_or_else(|| BettertricksError::Recipe(format!("missing file {file}")))?;
                let source = self.catalog.cache_path(&recipe.id, definition);
                let source = ensure_read_source_in_prefix_or_cache(
                    &source,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                let destination = context.expand_path(recipe, destination)?;
                ensure_mutation_in_prefix(
                    &destination,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                tokio::fs::create_dir_all(&destination).await?;
                extract_archive(&source, &destination, *format, include, context).await
            }
            RecipeStep::ExtractPath {
                source,
                destination,
                format,
                include,
            } => {
                let source = context.expand_path(recipe, source)?;
                let source = ensure_read_source_in_prefix_or_cache(
                    &source,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                let destination = context.expand_path(recipe, destination)?;
                ensure_mutation_in_prefix(
                    &destination,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
                tokio::fs::create_dir_all(&destination).await?;
                extract_archive(&source, &destination, *format, include, context).await
            }
            RecipeStep::InstallFonts { source, fonts } => {
                let source = context.expand_path(recipe, source)?;
                install_fonts(&source, fonts, context).await
            }
            RecipeStep::FontReplacements { replacements } => {
                let values = replacements
                    .iter()
                    .map(|replacement| {
                        Ok(format!(
                            "\"{}\"=\"{}\"",
                            registry_string(&replacement.alias)?,
                            registry_string(&replacement.replacement)?
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join("\n");
                let registry =
                    format!("[HKEY_CURRENT_USER\\Software\\Wine\\Fonts\\Replacements]\n{values}");
                import_registry(&registry, RegistryArchitecture::Prefix, context).await
            }
            RecipeStep::On64BitPrefix { steps } => {
                if is_64_bit_prefix(&context.plan.prefix) {
                    for step in steps {
                        Box::pin(self.execute_step(recipe, step, context)).await?;
                    }
                } else {
                    context.log("Skipping steps that only apply to 64-bit prefixes".into());
                }
                Ok(())
            }
            RecipeStep::Wine {
                program,
                arguments,
                unattended_arguments,
                environment,
            } => {
                let mut arguments = arguments.clone();
                if context.plan.options.unattended {
                    arguments.extend(unattended_arguments.clone());
                }
                let prefix = context.plan.prefix.clone();
                run_wine(&prefix, program, &arguments, environment, context).await
            }
            RecipeStep::Registry {
                content,
                architecture,
            } => import_registry(content, *architecture, context).await,
            RecipeStep::DllOverride {
                mode,
                libraries,
                application,
            } => import_dll_overrides(*mode, libraries, application.as_deref(), context).await,
            RecipeStep::WindowsVersion {
                version,
                application,
            } => {
                if application.is_some() {
                    let key = format!(
                        "[HKEY_CURRENT_USER\\Software\\Wine\\AppDefaults\\{}]\n\"Version\"=\"{}\"",
                        application.as_deref().unwrap_or_default(),
                        version
                    );
                    import_registry(&key, RegistryArchitecture::Prefix, context).await
                } else {
                    let prefix = context.plan.prefix.clone();
                    run_wine(
                        &prefix,
                        "winecfg",
                        &["-v".into(), version.clone()],
                        &BTreeMap::new(),
                        context,
                    )
                    .await
                }
            }
            RecipeStep::Call { recipe } => {
                context.log(format!("Dependency {recipe} is resolved by the planner."));
                Ok(())
            }
            RecipeStep::Notice {
                level: _,
                title,
                message,
            } => {
                context.log(format!("{title}: {message}"));
                context.emit(
                    OperationState::Running,
                    title,
                    Some(message.clone()),
                    Some(context.fraction()),
                    None,
                );
                Ok(())
            }
            RecipeStep::Prompt {
                level,
                title,
                message,
            } => self.prompt(*level, title, message, context).await,
            RecipeStep::NativeAction { action, parameters } => {
                self.native_action(action, parameters, context).await
            }
        }
    }

    async fn download(
        &self,
        recipe: &Recipe,
        file: &RecipeFile,
        context: &mut ExecutionContext,
    ) -> Result<()> {
        let destination = self.catalog.cache_path(&recipe.id, file);
        let cache_lock = {
            let mut locks = self.cache_locks.lock();
            locks
                .entry(destination.clone())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let _cache_guard = cache_lock.lock().await;
        ensure_mutation_in_prefix(
            &destination,
            &context.plan.prefix.path,
            &context.paths.winetricks_cache,
        )?;
        if destination.is_file() {
            verify_checksum(&destination, file.sha256.as_deref()).await?;
            context.log(format!("Using cached {}", destination.display()));
            return Ok(());
        }
        if file.manual {
            return Err(BettertricksError::Unsupported(format!(
                "Manual download required: place {} in {}",
                file.filename,
                destination.parent().unwrap_or(Path::new(".")).display()
            )));
        }
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let partial = destination.with_file_name(format!(
            "{}.{}.part",
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            context.plan.id
        ));

        let mut builder = reqwest::Client::builder()
            .user_agent(format!("Bettertricks/{}", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(30))
            .read_timeout(std::time::Duration::from_secs(90));
        if context.plan.options.torify {
            builder = builder.proxy(reqwest::Proxy::all("socks5h://127.0.0.1:9050")?);
        }
        let client = builder.build()?;
        let mut last_error = None;
        for url in &file.urls {
            if context.cancelled() {
                return Err(BettertricksError::Cancelled);
            }
            match download_url(&client, url, &partial, context).await {
                Ok(()) => {
                    verify_checksum(&partial, file.sha256.as_deref()).await?;
                    tokio::fs::rename(&partial, &destination).await?;
                    return Ok(());
                }
                Err(error) => {
                    if matches!(error, BettertricksError::Cancelled) {
                        let _ = tokio::fs::remove_file(&partial).await;
                        return Err(error);
                    }
                    context.log(format!("Download source failed: {url}: {error}"));
                    last_error = Some(error);
                    let _ = tokio::fs::remove_file(&partial).await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            BettertricksError::Recipe(format!("{} has no download URL", file.id))
        }))
    }

    async fn prompt(
        &self,
        level: PromptLevel,
        title: &str,
        message: &str,
        context: &mut ExecutionContext,
    ) -> Result<()> {
        if context.plan.options.unattended {
            if matches!(level, PromptLevel::Confirmation) {
                return Err(BettertricksError::Unsupported(format!(
                    "{title} requires confirmation in attended mode"
                )));
            }
            context.log(message.to_string());
            return Ok(());
        }
        let prompt_id = Uuid::new_v4();
        let prompt = OperationPrompt {
            id: prompt_id,
            level,
            title: title.into(),
            message: message.into(),
            choices: vec![
                PromptChoice {
                    id: "continue".into(),
                    label: "Continue".into(),
                    destructive: false,
                },
                PromptChoice {
                    id: "cancel".into(),
                    label: "Cancel".into(),
                    destructive: false,
                },
            ],
        };
        let (sender, receiver) = oneshot::channel();
        self.prompts
            .lock()
            .insert((context.plan.id, prompt_id), sender);
        self.update_record(context, OperationState::WaitingForUser, Some(title), false)?;
        context.emit(
            OperationState::WaitingForUser,
            title,
            Some(message.into()),
            Some(context.fraction()),
            Some(prompt),
        );
        let choice = receiver.await.map_err(|_| BettertricksError::Cancelled)?;
        if choice == "continue" {
            Ok(())
        } else {
            Err(BettertricksError::Cancelled)
        }
    }

    async fn native_action(
        &self,
        action: &NativeAction,
        parameters: &BTreeMap<String, String>,
        context: &mut ExecutionContext,
    ) -> Result<()> {
        match action {
            NativeAction::Noop => Ok(()),
            NativeAction::FontSmoothing => {
                let smoothing = parameters
                    .get("smoothing")
                    .map(String::as_str)
                    .unwrap_or("2");
                let orientation = parameters
                    .get("orientation")
                    .map(String::as_str)
                    .unwrap_or("1");
                let smoothing_type = parameters
                    .get("smoothing_type")
                    .map(String::as_str)
                    .unwrap_or("2");
                let registry = format!(
                    r#"[HKEY_CURRENT_USER\Control Panel\Desktop]
"FontSmoothing"="{smoothing}"
"FontSmoothingGamma"=dword:00000578
"FontSmoothingOrientation"=dword:{orientation:0>8}
"FontSmoothingType"=dword:{smoothing_type:0>8}
"#
                );
                import_registry(&registry, RegistryArchitecture::Prefix, context).await
            }
            NativeAction::WinebootUpdate => {
                let prefix = context.plan.prefix.clone();
                run_wine(
                    &prefix,
                    "wineboot",
                    &["-u".into()],
                    &BTreeMap::new(),
                    context,
                )
                .await
            }
            NativeAction::FontfixCheck => match Command::new("xlsfonts").output().await {
                Ok(output) if has_problematic_samyak_oriya_font(&output.stdout) => {
                    Err(BettertricksError::Recipe(
                        "A Samyak/Oriya X11 font was detected. Remove that host font, then log out and back in before running affected .NET applications.".into(),
                    ))
                }
                Ok(output) => {
                    if !output.status.success() {
                        context.log("xlsfonts could not enumerate host fonts; the Samyak/Oriya compatibility check was skipped.".into());
                    } else {
                        context.log("No problematic Samyak/Oriya X11 font was detected.".into());
                    }
                    Ok(())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    context.log("xlsfonts is not installed; the optional Samyak/Oriya compatibility check was skipped.".into());
                    Ok(())
                }
                Err(error) => Err(error.into()),
            },
            NativeAction::IsolateHome => {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .ok_or_else(|| BettertricksError::Unsupported("HOME is not set".into()))?;
                let user = std::env::var("USER")
                    .map_err(|_| BettertricksError::Unsupported("USER is not set".into()))?;
                let changed = isolate_home_links(&context.plan.prefix.path, &home, &user)?;
                context.log(format!(
                    "Replaced {changed} Wine user link(s) into HOME with local directories."
                ));
                Ok(())
            }
            NativeAction::NativeMdac => {
                let wine = context
                    .plan
                    .prefix
                    .runtime
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("wine"));
                let output = Command::new(&wine).arg("--version").output().await?;
                if !output.status.success() {
                    return Err(BettertricksError::CommandFailed {
                        program: wine.to_string_lossy().into_owned(),
                        code: output.status.code(),
                    });
                }
                let version_output = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                let include_msdasql = wine_version_at_most(&version_output, &[6, 21])
                    .ok_or_else(|| {
                        BettertricksError::Unsupported(format!(
                            "could not determine Wine version from {version_output:?}"
                        ))
                    })?;
                let mut libraries = vec!["msado15", "odbccp32"];
                if include_msdasql {
                    libraries.push("msdasql");
                }
                libraries.extend(["mtxdm", "odbc32", "oledb32"]);
                context.log(format!(
                    "Applying native,builtin MDAC overrides for Wine {}.",
                    version_output.trim()
                ));
                let libraries = libraries.into_iter().map(str::to_owned).collect::<Vec<_>>();
                import_dll_overrides(
                    DllOverrideMode::NativeBuiltin,
                    &libraries,
                    None,
                    context,
                )
                .await
            }
            NativeAction::RemoveMono => remove_mono(context).await,
            NativeAction::SetMidiDevice => {
                let device = context.recipe_input("device")?.to_owned();
                let device = registry_string(&device)?;
                let registry = format!(
                    "[HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Multimedia\\MIDIMap]\n\"CurrentInstrument\"=\"{device}\""
                );
                import_registry(&registry, RegistryArchitecture::Prefix, context).await
            }
            NativeAction::SetUserPath => {
                let paths = context.recipe_input("paths")?.to_owned();
                let winepath = wine_tool(&context.plan.prefix, "winepath");
                let output = Command::new(&winepath)
                    .args(["-w", &paths])
                    .env("WINEPREFIX", &context.plan.prefix.path)
                    .output()
                    .await?;
                if !output.status.success() {
                    return Err(BettertricksError::CommandFailed {
                        program: winepath.to_string_lossy().into_owned(),
                        code: output.status.code(),
                    });
                }
                let converted = String::from_utf8_lossy(&output.stdout)
                    .trim_end_matches(['\r', '\n'])
                    .to_owned();
                if converted.is_empty() {
                    return Err(BettertricksError::Unsupported(
                        "winepath returned an empty Windows PATH".into(),
                    ));
                }
                let converted = registry_string(&converted)?;
                let registry = format!(
                    "[HKEY_CURRENT_USER\\Environment]\n\"PATH\"=\"{converted}\""
                );
                import_registry(&registry, RegistryArchitecture::Prefix, context).await
            }
            NativeAction::IntentionalFailure => Err(BettertricksError::Recipe(
                parameters
                    .get("message")
                    .cloned()
                    .unwrap_or_else(|| "Intentional compatibility-test failure".into()),
            )),
        }
    }

    async fn finish_error(
        &self,
        context: &mut ExecutionContext,
        error: BettertricksError,
    ) -> Result<()> {
        if matches!(error, BettertricksError::Cancelled) {
            return self.finish_cancelled(context).await;
        }
        let detail = error.to_string();
        let title = if context.started_at.is_some() {
            "Operation stopped"
        } else {
            "Preflight failed"
        };
        context.log(format!("{title}: {detail}"));
        context.finished_at = Some(Utc::now());
        context.emit(
            OperationState::Failed,
            title,
            Some(detail.clone()),
            Some(context.fraction()),
            None,
        );
        self.update_record(context, OperationState::Failed, Some(&detail), true)?;
        Err(error)
    }

    async fn finish_cancelled(&self, context: &mut ExecutionContext) -> Result<()> {
        context.finished_at = Some(Utc::now());
        context.emit(
            OperationState::Cancelled,
            "Operation cancelled",
            Some("The prefix may contain partial changes. Review the log before retrying.".into()),
            Some(context.fraction()),
            None,
        );
        self.update_record(
            context,
            OperationState::Cancelled,
            Some("Operation cancelled"),
            true,
        )?;
        Err(BettertricksError::Cancelled)
    }

    fn update_record(
        &self,
        context: &ExecutionContext,
        state: OperationState,
        message: Option<&str>,
        finished: bool,
    ) -> Result<()> {
        self.store.upsert_operation(&OperationRecord {
            id: context.plan.id,
            prefix_id: context.plan.prefix.id,
            prefix_name: context.plan.prefix.name.clone(),
            recipes: context.plan.resolved_recipes.clone(),
            state,
            created_at: context.created_at,
            started_at: context.started_at,
            finished_at: finished.then_some(context.finished_at.unwrap_or_else(Utc::now)),
            current_step: context.step,
            total_steps: context.plan.steps.len(),
            message: message.map(str::to_owned),
            failures: context.failures.clone(),
        })
    }
}

#[derive(Debug)]
struct ProcessOutputLine {
    stream: &'static str,
    text: String,
}

async fn forward_process_output<R>(
    reader: R,
    stream: &'static str,
    sender: mpsc::Sender<ProcessOutputLine>,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    while let Some(text) = lines.next_line().await? {
        if sender
            .send(ProcessOutputLine { stream, text })
            .await
            .is_err()
        {
            break;
        }
    }
    Ok(())
}

fn record_process_output(
    context: &mut ExecutionContext,
    recipe: &Recipe,
    output: ProcessOutputLine,
    tail: &mut VecDeque<String>,
    emitted_lines: &mut usize,
    suppressed_lines: &mut usize,
) {
    let text = truncate_text(output.text.trim(), MAX_PROCESS_LOG_LINE_CHARS);
    if text.is_empty() {
        return;
    }
    let diagnostic = format!("{}: {text}", output.stream);
    if tail.len() == PROCESS_FAILURE_TAIL_LINES {
        tail.pop_front();
    }
    tail.push_back(diagnostic);
    if *emitted_lines < MAX_LIVE_PROCESS_LOG_LINES {
        context.log(format!("[{} {}] {text}", recipe.id, output.stream));
        *emitted_lines += 1;
    } else {
        *suppressed_lines += 1;
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let result = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        format!("{result}…")
    } else {
        result
    }
}

fn format_operation_failure_summary(succeeded: usize, failed: usize, skipped: usize) -> String {
    let succeeded_label = if succeeded == 1 {
        "component"
    } else {
        "components"
    };
    let failed_label = if failed == 1 {
        "component"
    } else {
        "components"
    };
    let mut summary = format!(
        "Finished the remaining jobs: {succeeded} {succeeded_label} succeeded and {failed} {failed_label} failed"
    );
    if skipped > 0 {
        let skipped_label = if skipped == 1 {
            "dependent component was"
        } else {
            "dependent components were"
        };
        summary.push_str(&format!("; {skipped} {skipped_label} skipped"));
    }
    summary.push('.');
    summary
}

fn operation_execution_error_is_fatal(error: &BettertricksError) -> bool {
    matches!(
        error,
        BettertricksError::Security(_)
            | BettertricksError::Database(_)
            | BettertricksError::Serialization(_)
            | BettertricksError::Catalog(_)
            | BettertricksError::PrefixNotFound(_)
            | BettertricksError::RecipeNotFound(_)
            | BettertricksError::OperationNotFound(_)
            | BettertricksError::Conflict(_)
    )
}

fn has_problematic_samyak_oriya_font(output: &[u8]) -> bool {
    String::from_utf8_lossy(output).lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.find("samyak")
            .is_some_and(|position| line[position + "samyak".len()..].contains("oriya"))
    })
}

fn validate_plan_inputs(recipes: &HashMap<String, Recipe>, plan: &OperationPlan) -> Result<()> {
    let mut supplied = HashMap::new();
    for input in &plan.inputs {
        if supplied.insert(input.key.as_str(), input).is_some() {
            return Err(BettertricksError::Recipe(format!(
                "operation input {} appears more than once",
                input.key
            )));
        }
    }
    let mut expected = HashSet::new();
    for recipe_id in &plan.resolved_recipes {
        let recipe = recipes.get(recipe_id).ok_or_else(|| {
            BettertricksError::Recipe(format!("planned recipe {recipe_id} is unavailable"))
        })?;
        for definition in &recipe.inputs {
            let key = format!("{}.{}", recipe.id, definition.id);
            expected.insert(key.clone());
            let input = supplied.get(key.as_str()).ok_or_else(|| {
                BettertricksError::Recipe(format!("operation plan is missing input {key}"))
            })?;
            if input.recipe_id != recipe.id || input.id != definition.id {
                return Err(BettertricksError::Recipe(format!(
                    "operation input {key} does not match its recipe definition"
                )));
            }
            let value = input.value.as_deref().unwrap_or_default();
            if definition.required && value.trim().is_empty() {
                return Err(BettertricksError::Recipe(format!(
                    "operation input {key} is required"
                )));
            }
            if value.len() > 8192
                || value
                    .chars()
                    .any(|value| matches!(value, '\0' | '\r' | '\n'))
            {
                return Err(BettertricksError::Recipe(format!(
                    "operation input {key} contains unsupported data"
                )));
            }
        }
    }
    if let Some(unknown) = plan
        .inputs
        .iter()
        .find(|input| !expected.contains(&input.key))
    {
        return Err(BettertricksError::Recipe(format!(
            "operation plan contains unknown input {}",
            unknown.key
        )));
    }
    Ok(())
}

fn registry_string(value: &str) -> Result<String> {
    if value
        .chars()
        .any(|value| matches!(value, '\0' | '\r' | '\n'))
    {
        return Err(BettertricksError::Security(
            "registry string contains a control character".into(),
        ));
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn wine_tool(prefix: &WinePrefix, tool: &str) -> PathBuf {
    let Some(runtime) = &prefix.runtime else {
        return PathBuf::from(tool);
    };
    let Some(parent) = runtime
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return PathBuf::from(tool);
    };
    let candidate = parent.join(tool);
    if candidate.is_file() {
        candidate
    } else {
        PathBuf::from(tool)
    }
}

fn isolate_home_links(prefix: &Path, home: &Path, user: &str) -> Result<usize> {
    if user.is_empty() || user == "." || user == ".." || user.contains('/') || user.contains('\\') {
        return Err(BettertricksError::Security(
            "USER is not a safe Wine user directory name".into(),
        ));
    }
    let prefix = prefix.canonicalize()?;
    let home = home.canonicalize()?;
    if home == Path::new("/") {
        return Err(BettertricksError::Security(
            "refusing to isolate HOME when HOME resolves to /".into(),
        ));
    }
    let user_directory = prefix.join("drive_c/users").join(user);
    let mut changed = 0;
    if user_directory.is_dir() {
        for entry in WalkDir::new(&user_directory).follow_links(false) {
            let entry = entry
                .map_err(|error| BettertricksError::Io(std::io::Error::other(error.to_string())))?;
            if !entry.file_type().is_symlink() {
                continue;
            }
            let link = entry.path();
            let raw_target = std::fs::read_link(link)?;
            let target = if raw_target.is_absolute() {
                raw_target
            } else {
                link.parent().unwrap_or(&user_directory).join(raw_target)
            };
            let target_lexical = absolute_lexical(&target);
            let resolved_target = target.canonicalize().ok();
            let points_inside_prefix = resolved_target
                .as_deref()
                .is_some_and(|target| target.starts_with(&prefix))
                || target_lexical.starts_with(&prefix);
            if points_inside_prefix {
                continue;
            }
            if std::fs::metadata(&target).is_ok_and(|metadata| metadata.is_file()) {
                continue;
            }
            let points_into_home = resolved_target
                .as_deref()
                .is_some_and(|target| target.starts_with(&home))
                || target_lexical.starts_with(&home);
            if !points_into_home {
                continue;
            }

            let metadata = std::fs::symlink_metadata(link)?;
            if !metadata.file_type().is_symlink() {
                return Err(BettertricksError::Security(format!(
                    "refusing to replace {} because it is no longer a symlink",
                    link.display()
                )));
            }
            std::fs::remove_file(link)?;
            std::fs::create_dir(link)?;
            changed += 1;
        }
    }
    std::fs::write(prefix.join(".update-timestamp"), b"disable\n")?;
    Ok(changed)
}

fn wine_version_at_most(output: &str, maximum: &[u64]) -> Option<bool> {
    let expression = Regex::new(r"(?i)\bwine[- ]?(\d+(?:\.\d+)*)").expect("valid expression");
    let version = expression.captures(output)?.get(1)?.as_str();
    let components = version
        .split('.')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    let length = components.len().max(maximum.len());
    for index in 0..length {
        match components
            .get(index)
            .copied()
            .unwrap_or_default()
            .cmp(&maximum.get(index).copied().unwrap_or_default())
        {
            std::cmp::Ordering::Less => return Some(true),
            std::cmp::Ordering::Greater => return Some(false),
            std::cmp::Ordering::Equal => {}
        }
    }
    Some(true)
}

fn mono_uninstaller_ids(output: &str) -> Vec<String> {
    let descriptions = [
        "Wine Mono Windows Support",
        "Wine Mono Runtime",
        "Wine Mono",
    ];
    let mut ids = Vec::new();
    for description in descriptions {
        for line in output.lines().filter(|line| line.contains(description)) {
            let id = line.split('|').next().unwrap_or_default().trim();
            if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
                ids.push(id.to_owned());
            }
        }
    }
    ids
}

async fn remove_mono(context: &mut ExecutionContext) -> Result<()> {
    let prefix = context.plan.prefix.clone();
    let wine = prefix
        .runtime
        .clone()
        .unwrap_or_else(|| PathBuf::from("wine"));
    let output = Command::new(&wine)
        .args(["uninstaller", "--list"])
        .current_dir(prefix.path.join("drive_c"))
        .env("WINEPREFIX", &prefix.path)
        .env("WINEDEBUG", "-all")
        .output()
        .await?;
    let listing = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let ids = mono_uninstaller_ids(&listing);
    if ids.is_empty() {
        context.log("Wine Mono does not appear to be installed.".into());
        return Ok(());
    }

    for id in &ids {
        run_wine(
            &prefix,
            "uninstaller",
            &["--remove".into(), id.clone()],
            &BTreeMap::new(),
            context,
        )
        .await?;
    }
    for registry_key in [
        r"HKLM\Software\Microsoft\NET Framework Setup\NDP\v3.5",
        r"HKLM\Software\Microsoft\NET Framework Setup\NDP\v4",
    ] {
        if let Err(error) = run_wine(
            &prefix,
            "reg",
            &["delete".into(), registry_key.into(), "/f".into()],
            &BTreeMap::new(),
            context,
        )
        .await
        {
            context.log(format!(
                "Ignoring optional Mono registry cleanup failure: {error}"
            ));
        }
    }

    for path in [
        prefix.path.join("drive_c/windows/system32/mscoree.dll"),
        prefix.path.join("drive_c/windows/syswow64/mscoree.dll"),
    ] {
        let Ok(content) = tokio::fs::read(&path).await else {
            continue;
        };
        if content
            .windows(b"WINE_MONO_OVERRIDES".len())
            .any(|window| window == b"WINE_MONO_OVERRIDES")
        {
            ensure_mutation_in_prefix(&path, &prefix.path, &context.paths.cache)?;
            tokio::fs::remove_file(path).await?;
        }
    }
    context.log(format!("Removed {} Wine Mono installer(s).", ids.len()));
    Ok(())
}

struct ExecutionContext {
    plan: OperationPlan,
    cancellation: Arc<AtomicBool>,
    sink: Arc<dyn OperationEventSink>,
    paths: AppPaths,
    sequence: u64,
    step: usize,
    recipe_id: Option<String>,
    created_at: chrono::DateTime<Utc>,
    started_at: Option<chrono::DateTime<Utc>>,
    finished_at: Option<chrono::DateTime<Utc>>,
    failures: Vec<RecipeFailure>,
}

impl ExecutionContext {
    fn new(
        plan: OperationPlan,
        cancellation: Arc<AtomicBool>,
        sink: Arc<dyn OperationEventSink>,
        paths: AppPaths,
    ) -> Self {
        Self {
            plan,
            cancellation,
            sink,
            paths,
            sequence: 0,
            step: 0,
            recipe_id: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            failures: Vec::new(),
        }
    }

    fn cancelled(&self) -> bool {
        self.cancellation.load(Ordering::SeqCst)
    }

    fn fraction(&self) -> f64 {
        if self.plan.steps.is_empty() {
            0.0
        } else {
            self.step as f64 / self.plan.steps.len() as f64
        }
    }

    fn emit(
        &mut self,
        state: OperationState,
        title: &str,
        detail: Option<String>,
        progress: Option<f64>,
        prompt: Option<OperationPrompt>,
    ) {
        self.sequence += 1;
        self.sink.emit(OperationEvent {
            operation_id: self.plan.id,
            sequence: self.sequence,
            state,
            step: self.step,
            total_steps: self.plan.steps.len(),
            recipe_id: self.recipe_id.clone(),
            title: title.into(),
            detail,
            progress,
            prompt,
            log_line: None,
            failure: None,
            timestamp: Utc::now(),
        });
    }

    fn emit_failure(&mut self, failure: RecipeFailure) {
        let title = match failure.kind {
            RecipeFailureKind::Failed => format!("{} failed", failure.recipe_title),
            RecipeFailureKind::SkippedDependency => {
                format!("{} skipped", failure.recipe_title)
            }
        };
        self.failures.push(failure.clone());
        self.sequence += 1;
        self.sink.emit(OperationEvent {
            operation_id: self.plan.id,
            sequence: self.sequence,
            state: OperationState::Running,
            step: self.step,
            total_steps: self.plan.steps.len(),
            recipe_id: Some(failure.recipe_id.clone()),
            title,
            detail: Some(failure.message.clone()),
            progress: Some(self.fraction()),
            prompt: None,
            log_line: None,
            failure: Some(failure),
            timestamp: Utc::now(),
        });
    }

    fn log(&mut self, line: String) {
        let line = sanitize_log(line);
        self.sequence += 1;
        self.sink.emit(OperationEvent {
            operation_id: self.plan.id,
            sequence: self.sequence,
            state: OperationState::Running,
            step: self.step,
            total_steps: self.plan.steps.len(),
            recipe_id: self.recipe_id.clone(),
            title: "Log".into(),
            detail: None,
            progress: Some(self.fraction()),
            prompt: None,
            log_line: Some(line),
            failure: None,
            timestamp: Utc::now(),
        });
    }

    fn recipe_input(&self, id: &str) -> Result<&str> {
        let recipe_id = self.recipe_id.as_deref().ok_or_else(|| {
            BettertricksError::Recipe("operation input requested outside a recipe step".into())
        })?;
        let key = format!("{recipe_id}.{id}");
        self.plan
            .inputs
            .iter()
            .find(|input| input.key == key)
            .and_then(|input| input.value.as_deref())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| BettertricksError::Recipe(format!("operation input {key} is required")))
    }

    fn expand_path(&self, recipe: &Recipe, template: &str) -> Result<PathBuf> {
        let mut value = template.to_string();
        let prefix = &self.plan.prefix.path;
        let replacements = [
            ("${prefix}", prefix.clone()),
            ("${drive_c}", prefix.join("drive_c")),
            ("${windows}", prefix.join("drive_c/windows")),
            ("${system32}", prefix.join("drive_c/windows/system32")),
            ("${syswow64}", prefix.join("drive_c/windows/syswow64")),
            ("${temp}", prefix.join("drive_c/windows/temp/bettertricks")),
            ("${cache}", self.paths.winetricks_cache.clone()),
        ];
        for (marker, replacement) in replacements {
            value = value.replace(marker, replacement.to_string_lossy().as_ref());
        }
        if value.contains("${system32_32}") {
            value = value.replace(
                "${system32_32}",
                system_directory_for_32_bit(&self.plan.prefix)?
                    .to_string_lossy()
                    .as_ref(),
            );
        }
        if value.contains("${system32_64}") {
            value = value.replace(
                "${system32_64}",
                system_directory_for_64_bit(&self.plan.prefix)?
                    .to_string_lossy()
                    .as_ref(),
            );
        }
        for file in &recipe.files {
            let cache_path = file
                .cache_path
                .as_deref()
                .map(|path| self.paths.winetricks_cache.join(path))
                .unwrap_or_else(|| {
                    self.paths
                        .winetricks_cache
                        .join(&recipe.id)
                        .join(&file.filename)
                });
            value = value.replace(
                &format!("${{file:{}}}", file.id),
                cache_path.to_string_lossy().as_ref(),
            );
        }
        if value.contains("${") {
            return Err(BettertricksError::Recipe(format!(
                "unresolved path template {value}"
            )));
        }
        Ok(PathBuf::from(value))
    }
}

fn system_directory_for_32_bit(prefix: &WinePrefix) -> Result<PathBuf> {
    let windows = prefix.path.join("drive_c/windows");
    let syswow64 = windows.join("syswow64");
    if syswow64.is_dir() || matches!(prefix.architecture, crate::PrefixArchitecture::Wow64) {
        return Ok(syswow64);
    }
    if prefix.architecture == crate::PrefixArchitecture::Win64 {
        return Err(BettertricksError::Unsupported(
            "This prefix has no 32-bit Windows subsystem".into(),
        ));
    }
    Ok(windows.join("system32"))
}

fn system_directory_for_64_bit(prefix: &WinePrefix) -> Result<PathBuf> {
    if prefix.architecture == crate::PrefixArchitecture::Win32 {
        return Err(BettertricksError::Unsupported(
            "This recipe step requires a 64-bit Wine prefix".into(),
        ));
    }
    Ok(prefix.path.join("drive_c/windows/system32"))
}

fn is_64_bit_prefix(prefix: &WinePrefix) -> bool {
    matches!(
        prefix.architecture,
        crate::PrefixArchitecture::Win64 | crate::PrefixArchitecture::Wow64
    ) || prefix.path.join("drive_c/windows/syswow64").is_dir()
}

async fn initialize_prefix(prefix: &WinePrefix, context: &ExecutionContext) -> Result<()> {
    tokio::fs::create_dir_all(&prefix.path).await?;
    let wine = prefix
        .runtime
        .clone()
        .unwrap_or_else(|| PathBuf::from("wine"));
    let status = Command::new(wine)
        .arg("wineboot")
        .arg("-u")
        .env("WINEPREFIX", &prefix.path)
        .env(
            "WINEARCH",
            match prefix.architecture {
                crate::PrefixArchitecture::Win32 => "win32",
                _ => "win64",
            },
        )
        .status()
        .await?;
    if !status.success() || context.cancelled() {
        return Err(BettertricksError::CommandFailed {
            program: "wineboot".into(),
            code: status.code(),
        });
    }
    Ok(())
}

async fn download_url(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    context: &mut ExecutionContext,
) -> Result<()> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total = response.content_length();
    if total.is_some_and(|total| total > MAX_RECIPE_DOWNLOAD_BYTES) {
        return Err(BettertricksError::Security(
            "recipe download exceeds the 16 GiB safety limit".into(),
        ));
    }
    let mut stream = response.bytes_stream();
    let mut output = tokio::fs::File::create(destination).await?;
    let mut downloaded = 0_u64;
    while let Some(chunk) = stream.next().await {
        if context.cancelled() {
            return Err(BettertricksError::Cancelled);
        }
        let chunk = chunk?;
        if downloaded.saturating_add(chunk.len() as u64) > MAX_RECIPE_DOWNLOAD_BYTES {
            return Err(BettertricksError::Security(
                "recipe download exceeds the 16 GiB safety limit".into(),
            ));
        }
        output.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(total) = total.filter(|total| *total > 0) {
            context.emit(
                OperationState::Running,
                "Downloading",
                Some(format!(
                    "{} of {}",
                    format_bytes(downloaded),
                    format_bytes(total)
                )),
                Some(downloaded as f64 / total as f64),
                None,
            );
        }
    }
    output.flush().await?;
    Ok(())
}

async fn verify_checksum(path: &Path, expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = hex::encode(digest.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(BettertricksError::ChecksumMismatch {
            file: path.display().to_string(),
            expected: expected.into(),
            actual,
        });
    }
    Ok(())
}

async fn copy_path(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if from.is_dir() {
        let status = Command::new("cp")
            .arg("-a")
            .arg(from)
            .arg(to)
            .status()
            .await?;
        if !status.success() {
            return Err(BettertricksError::CommandFailed {
                program: "cp".into(),
                code: status.code(),
            });
        }
    } else {
        tokio::fs::copy(from, to).await?;
    }
    Ok(())
}

async fn install_fonts(
    source_directory: &Path,
    fonts: &[RecipeFont],
    context: &mut ExecutionContext,
) -> Result<()> {
    let source_directory = source_directory.canonicalize()?;
    let prefix = context.plan.prefix.path.canonicalize()?;
    let cache = resolve_existing_path(&context.paths.winetricks_cache)?;
    if !source_directory.starts_with(&prefix) && !source_directory.starts_with(&cache) {
        return Err(BettertricksError::Security(format!(
            "refusing to install fonts from {} outside the prefix or cache",
            source_directory.display()
        )));
    }
    let destination_directory = prefix.join("drive_c/windows/Fonts");
    tokio::fs::create_dir_all(&destination_directory).await?;

    let mut copied_destinations = HashSet::new();
    for font in fonts {
        if !copied_destinations.insert(font.filename.to_ascii_lowercase()) {
            continue;
        }
        let mut source_entries = tokio::fs::read_dir(&source_directory).await?;
        let mut source = None;
        while let Some(entry) = source_entries.next_entry().await? {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&font.source)
            {
                if source.is_some() {
                    return Err(BettertricksError::Recipe(format!(
                        "more than one extracted file matches {}",
                        font.source
                    )));
                }
                source = Some(entry.path());
            }
        }
        let source = source
            .ok_or_else(|| {
                BettertricksError::Recipe(format!("extracted font {} is missing", font.source))
            })?
            .canonicalize()?;
        if !source.starts_with(&source_directory) || !source.is_file() {
            return Err(BettertricksError::Security(format!(
                "font source is not a regular extracted file: {}",
                source.display()
            )));
        }
        let destination = destination_directory.join(&font.filename);
        ensure_mutation_in_prefix(&destination, &prefix, &context.paths.winetricks_cache)?;
        let mut entries = tokio::fs::read_dir(&destination_directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&font.filename)
            {
                let existing = entry.path();
                let metadata = tokio::fs::symlink_metadata(&existing).await?;
                if metadata.is_file() || metadata.file_type().is_symlink() {
                    tokio::fs::remove_file(existing).await?;
                }
            }
        }
        tokio::fs::copy(source, destination).await?;
    }

    wait_for_wineserver(context).await?;
    let values = fonts
        .iter()
        .map(|font| {
            let suffix = if font.filename.to_ascii_lowercase().ends_with(".ttf")
                || font.filename.to_ascii_lowercase().ends_with(".ttc")
            {
                " (TrueType)"
            } else {
                ""
            };
            Ok(format!(
                "\"{}{}\"=\"{}\"",
                registry_string(&font.display_name)?,
                suffix,
                registry_string(&font.filename)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    let registry = format!(
        "[HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts]\n{values}\n\n[HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows\\CurrentVersion\\Fonts]\n{values}"
    );
    import_registry(&registry, RegistryArchitecture::Prefix, context).await
}

async fn wait_for_wineserver(context: &mut ExecutionContext) -> Result<()> {
    let tool = wine_tool(&context.plan.prefix, "wineserver");
    context.log(format!("{} -w", tool.display()));
    let mut child = Command::new(&tool)
        .arg("-w")
        .current_dir(context.plan.prefix.path.join("drive_c"))
        .env("WINEPREFIX", &context.plan.prefix.path)
        .spawn()?;
    loop {
        if context.cancelled() {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(BettertricksError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(BettertricksError::CommandFailed {
                    program: tool.to_string_lossy().into_owned(),
                    code: status.code(),
                })
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn extract_archive(
    source: &Path,
    destination: &Path,
    format: crate::ArchiveFormat,
    include: &[String],
    context: &mut ExecutionContext,
) -> Result<()> {
    let mut command = match format {
        crate::ArchiveFormat::Zip => {
            let mut command = Command::new("unzip");
            command.arg("-o").arg(source);
            for pattern in include {
                command.arg(pattern);
            }
            command.arg("-d").arg(destination);
            command
        }
        crate::ArchiveFormat::SevenZip => {
            let mut command = Command::new("7z");
            command
                .arg("x")
                .arg("-y")
                .arg(format!("-o{}", destination.display()))
                .arg(source);
            for pattern in include {
                command.arg(pattern);
            }
            command
        }
        crate::ArchiveFormat::Cabinet => {
            let mut command = Command::new("cabextract");
            command.arg("-q").arg("-L").arg("-d").arg(destination);
            for pattern in include {
                command.arg("-F").arg(pattern);
            }
            command.arg(source);
            command
        }
        crate::ArchiveFormat::Tar | crate::ArchiveFormat::TarGz | crate::ArchiveFormat::TarXz => {
            let mut command = Command::new("tar");
            command
                .arg("--no-same-owner")
                .arg("--no-same-permissions")
                .arg("-xf")
                .arg(source)
                .arg("-C")
                .arg(destination);
            for pattern in include {
                command.arg(pattern);
            }
            command
        }
    };
    let program = command.as_std().get_program().to_string_lossy().to_string();
    context.log(format!("Extracting {} with {program}", source.display()));
    let mut child = command.spawn()?;
    loop {
        if context.cancelled() {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(BettertricksError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(BettertricksError::CommandFailed {
                    program,
                    code: status.code(),
                })
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn run_wine(
    prefix: &WinePrefix,
    program: &str,
    arguments: &[String],
    environment: &BTreeMap<String, String>,
    context: &mut ExecutionContext,
) -> Result<()> {
    let wine = prefix
        .runtime
        .clone()
        .unwrap_or_else(|| PathBuf::from("wine"));
    let mut command = Command::new(wine);
    command
        .arg(program)
        .args(arguments)
        .current_dir(prefix.path.join("drive_c"))
        .env("WINEPREFIX", &prefix.path);
    if context.plan.options.unattended {
        command.env("WINETRICKS_OPT_UNATTENDED", "1");
    }
    for (key, value) in environment {
        command.env(key, value);
    }
    context.log(format!("wine {program} {}", shell_words::join(arguments)));
    let mut child = command.spawn()?;
    loop {
        if context.cancelled() {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(BettertricksError::Cancelled);
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            return Err(BettertricksError::CommandFailed {
                program: program.into(),
                code: status.code(),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn import_registry(
    content: &str,
    architecture: RegistryArchitecture,
    context: &mut ExecutionContext,
) -> Result<()> {
    let directory = context
        .plan
        .prefix
        .path
        .join("drive_c/windows/temp/bettertricks");
    tokio::fs::create_dir_all(&directory).await?;
    let path = directory.join(format!("{}.reg", Uuid::new_v4()));
    tokio::fs::write(&path, encode_registry_file(content)).await?;
    let wine_path = format!(
        "C:\\windows\\temp\\bettertricks\\{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    let prefix = context.plan.prefix.clone();
    for (runtime, program) in registry_invocations(&prefix, architecture) {
        let mut invocation_prefix = prefix.clone();
        invocation_prefix.runtime = Some(runtime);
        run_wine(
            &invocation_prefix,
            program,
            &["/S".into(), wine_path.clone()],
            &BTreeMap::new(),
            context,
        )
        .await?;
    }
    if !context.plan.options.no_clean {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(())
}

fn registry_invocations(
    prefix: &WinePrefix,
    architecture: RegistryArchitecture,
) -> Vec<(PathBuf, &'static str)> {
    let wine = prefix
        .runtime
        .clone()
        .unwrap_or_else(|| PathBuf::from("wine"));
    let wine64 = wine64_runtime(prefix, &wine);
    let is_64_bit_prefix = matches!(
        prefix.architecture,
        crate::PrefixArchitecture::Win64 | crate::PrefixArchitecture::Wow64
    );
    match architecture {
        RegistryArchitecture::Prefix if is_64_bit_prefix => vec![
            (wine, r"C:\windows\syswow64\regedit.exe"),
            (wine64, r"C:\windows\regedit.exe"),
        ],
        RegistryArchitecture::Win32 if is_64_bit_prefix => {
            vec![(wine, r"C:\windows\syswow64\regedit.exe")]
        }
        RegistryArchitecture::Win64 => vec![(wine64, r"C:\windows\regedit.exe")],
        RegistryArchitecture::Prefix | RegistryArchitecture::Win32 => {
            vec![(wine, r"C:\windows\regedit.exe")]
        }
    }
}

fn wine64_runtime(prefix: &WinePrefix, wine: &Path) -> PathBuf {
    if wine
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains("wine64"))
    {
        return wine.to_path_buf();
    }
    if let (Some(parent), Some(name)) = (
        wine.parent(),
        wine.file_name().and_then(|name| name.to_str()),
    ) && !parent.as_os_str().is_empty()
        && let Some(position) = name.find("wine")
    {
        let mut sibling_name = name.to_owned();
        sibling_name.replace_range(position..position + "wine".len(), "wine64");
        let sibling = parent.join(sibling_name);
        if sibling.is_file() {
            return sibling;
        }
    }
    if prefix.runtime.is_none()
        && let Some(found) = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|directory| directory.join("wine64"))
            .find(|candidate| candidate.is_file())
    {
        return found;
    }
    wine.to_path_buf()
}

async fn import_dll_overrides(
    mode: DllOverrideMode,
    libraries: &[String],
    application: Option<&str>,
    context: &mut ExecutionContext,
) -> Result<()> {
    import_registry(
        &dll_override_registry(mode, libraries, application),
        RegistryArchitecture::Prefix,
        context,
    )
    .await
}

fn dll_override_registry(
    mode: DllOverrideMode,
    libraries: &[String],
    application: Option<&str>,
) -> String {
    let value = match mode {
        DllOverrideMode::Native => Some("native"),
        DllOverrideMode::Builtin => Some("builtin"),
        DllOverrideMode::NativeBuiltin => Some("native,builtin"),
        DllOverrideMode::BuiltinNative => Some("builtin,native"),
        DllOverrideMode::Disabled => Some(""),
        DllOverrideMode::Default => None,
    };
    let key = if let Some(application) = application {
        format!(r#"HKEY_CURRENT_USER\Software\Wine\AppDefaults\{application}\DllOverrides"#)
    } else {
        r#"HKEY_CURRENT_USER\Software\Wine\DllOverrides"#.into()
    };
    let values = libraries
        .iter()
        .map(|library| match value {
            // Winetricks deliberately prefixes DLL names with `*` so the
            // override also applies when an application uses an absolute path.
            Some(value) => format!("\"*{library}\"=\"{value}\""),
            None => format!("\"*{library}\"=-"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("[{key}]\n{values}")
}

async fn ensure_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    Ok(())
}

async fn remove_symlink_in_prefix(path: &Path, prefix: &Path) -> Result<()> {
    let path = absolute_lexical(path);
    let prefix = resolve_existing_path(&absolute_lexical(prefix))?;
    let parent = path.parent().ok_or_else(|| {
        BettertricksError::Security(format!("symlink path has no parent: {}", path.display()))
    })?;
    let parent = resolve_existing_path(parent)?;
    if !parent.starts_with(&prefix) {
        return Err(BettertricksError::Security(format!(
            "refusing to remove symlink {} outside the prefix",
            path.display()
        )));
    }
    let metadata = match tokio::fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_symlink() {
        return Err(BettertricksError::Security(format!(
            "refusing to remove non-symlink {}",
            path.display()
        )));
    }
    tokio::fs::remove_file(path).await?;
    Ok(())
}

async fn append_installed_log(prefix: &Path, recipe_id: &str) -> Result<()> {
    let path = prefix.join("winetricks.log");
    let existing = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    if existing.split_whitespace().any(|value| value == recipe_id) {
        return Ok(());
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(format!("{recipe_id}\n").as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

fn ensure_mutation_in_prefix(path: &Path, prefix: &Path, cache: &Path) -> Result<()> {
    let path = absolute_lexical(path);
    let prefix = absolute_lexical(prefix);
    let cache = absolute_lexical(cache);
    let prefix_resolved = resolve_existing_path(&prefix)?;
    let cache_resolved = resolve_existing_path(&cache)?;
    let path_resolved = resolve_existing_path(&path)?;
    if !path_resolved.starts_with(&prefix_resolved) && !path_resolved.starts_with(&cache_resolved) {
        return Err(BettertricksError::Security(format!(
            "refusing to modify {} outside the prefix or cache",
            path.display()
        )));
    }
    if path_resolved == prefix_resolved || path_resolved == cache_resolved {
        return Err(BettertricksError::Security(
            "recipe cannot remove a storage root".into(),
        ));
    }
    Ok(())
}

fn ensure_read_source_in_prefix_or_cache(
    path: &Path,
    prefix: &Path,
    cache: &Path,
) -> Result<PathBuf> {
    let path = path.canonicalize()?;
    let prefix = resolve_existing_path(&absolute_lexical(prefix))?;
    let cache = resolve_existing_path(&absolute_lexical(cache))?;
    if !path.starts_with(&prefix) && !path.starts_with(&cache) {
        return Err(BettertricksError::Security(format!(
            "refusing to read recipe source {} outside the prefix or cache",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(BettertricksError::Security(format!(
            "recipe source is not a regular file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn encode_registry_file(content: &str) -> Vec<u8> {
    let has_header = content.starts_with("REGEDIT4")
        || content.starts_with("Windows Registry Editor Version 5.00");
    if content.is_ascii() {
        return if has_header {
            content.as_bytes().to_vec()
        } else {
            format!("REGEDIT4\n\n{content}\n").into_bytes()
        };
    }

    let body = content
        .strip_prefix("REGEDIT4")
        .or_else(|| content.strip_prefix("Windows Registry Editor Version 5.00"))
        .unwrap_or(content)
        .trim_start_matches(['\r', '\n']);
    let document = format!("Windows Registry Editor Version 5.00\r\n\r\n{body}\r\n");
    let mut encoded = vec![0xff, 0xfe];
    encoded.extend(document.encode_utf16().flat_map(u16::to_le_bytes));
    encoded
}

fn absolute_lexical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn resolve_existing_path(path: &Path) -> Result<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing.file_name().ok_or_else(|| {
            BettertricksError::Security(format!(
                "cannot resolve a safe ancestor for {}",
                path.display()
            ))
        })?;
        missing.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            BettertricksError::Security(format!(
                "cannot resolve a safe ancestor for {}",
                path.display()
            ))
        })?;
    }
    let mut resolved = existing.canonicalize()?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn sanitize_log(mut line: String) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        line = line.replace(home.to_string_lossy().as_ref(), "$HOME");
    }
    line
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn runs_metadata_recipes_through_the_exact_compatibility_host() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        std::fs::write(
            catalog_root.join("hosted.toml"),
            r#"
schema = 1
id = "hosted"
category = "dlls"
title = "Hosted recipe"
maturity = "metadata_only"

[source]
upstream_tag = "20260125"
upstream_verb = "hosted"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(
            crate::CatalogSource {
                path: catalog_root,
                version: "test".into(),
                upstream_tag: "20260125".into(),
            },
            paths.winetricks_cache.clone(),
        )
        .unwrap();
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: prefix_path.clone(),
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Wow64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        };
        let plan = crate::Planner::new(catalog.clone())
            .plan(
                crate::OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["hosted".into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap();
        assert_eq!(plan.steps.len(), 1);
        assert!(plan.restore_recommended);
        assert_eq!(plan.steps[0].label, "Run hosted through Winetricks");

        let binary = temp.path().join("winetricks");
        std::fs::write(
            &binary,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 20260125; else printf '%s' \"$*\" > \"$WINEPREFIX/host.args\"; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut engine = OperationEngine::new(catalog, paths, store);
        engine.legacy_host = LegacyVerbHost::with_binary(binary);
        let sink: Arc<dyn OperationEventSink> = Arc::new(|_event: OperationEvent| {});

        engine.run(plan, sink).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(prefix_path.join("host.args")).unwrap(),
            "hosted"
        );
        assert!(
            std::fs::read_to_string(prefix_path.join("winetricks.log"))
                .unwrap()
                .lines()
                .any(|line| line == "hosted")
        );
    }

    #[tokio::test]
    async fn continues_independent_recipes_and_skips_failed_dependants() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        std::fs::write(
            catalog_root.join("broken_component.toml"),
            r#"
schema = 1
id = "broken_component"
category = "settings"
title = "Broken component"
maturity = "native"

[[steps]]
type = "native_action"
action = "intentional_failure"
parameters = { message = "The simulated installer rejected its payload." }

[source]
upstream_tag = "test"
upstream_verb = "broken_component"
"#,
        )
        .unwrap();
        std::fs::write(
            catalog_root.join("dependent_component.toml"),
            r#"
schema = 1
id = "dependent_component"
category = "settings"
title = "Dependent component"
maturity = "native"
dependencies = ["broken_component"]

[[steps]]
type = "native_action"
action = "noop"

[source]
upstream_tag = "test"
upstream_verb = "dependent_component"
"#,
        )
        .unwrap();
        std::fs::write(
            catalog_root.join("independent_component.toml"),
            r#"
schema = 1
id = "independent_component"
category = "settings"
title = "Independent component"
maturity = "native"

[[steps]]
type = "native_action"
action = "noop"

[source]
upstream_tag = "test"
upstream_verb = "independent_component"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(
            crate::CatalogSource {
                path: catalog_root,
                version: "test".into(),
                upstream_tag: "test".into(),
            },
            paths.winetricks_cache.clone(),
        )
        .unwrap();
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Continuation test".into(),
            path: prefix_path.clone(),
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Wow64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        };
        let plan = crate::Planner::new(catalog.clone())
            .plan(
                crate::OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["dependent_component".into(), "independent_component".into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap();
        let operation_id = plan.id;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let engine = OperationEngine::new(catalog, paths, store.clone());
        let events = Arc::new(Mutex::new(Vec::<OperationEvent>::new()));
        let captured = events.clone();
        let sink: Arc<dyn OperationEventSink> = Arc::new(move |event: OperationEvent| {
            captured.lock().push(event);
        });

        let result = engine.run(plan, sink).await;

        assert!(result.is_err());
        let installed = std::fs::read_to_string(prefix_path.join("winetricks.log")).unwrap();
        assert_eq!(
            installed.trim(),
            "independent_component",
            "result: {result:?}; events: {:?}",
            events.lock()
        );
        let events = events.lock();
        let failures = events
            .iter()
            .filter_map(|event| event.failure.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].recipe_id, "broken_component");
        assert_eq!(failures[0].kind, RecipeFailureKind::Failed);
        assert!(failures[0].message.contains("simulated installer rejected"));
        assert_eq!(failures[1].recipe_id, "dependent_component");
        assert_eq!(failures[1].kind, RecipeFailureKind::SkippedDependency);
        assert!(failures[1].message.contains("broken_component"));
        let final_event = events.last().unwrap();
        assert_eq!(final_event.state, OperationState::Failed);
        assert_eq!(final_event.title, "Completed with failures");
        assert_eq!(final_event.progress, Some(1.0));
        drop(events);

        let record = store
            .operations(10)
            .unwrap()
            .into_iter()
            .find(|record| record.id == operation_id)
            .unwrap();
        assert_eq!(record.state, OperationState::Failed);
        assert_eq!(record.failures.len(), 2);
        assert_eq!(record.current_step, record.total_steps);
    }

    #[tokio::test]
    async fn captures_winetricks_failure_output_in_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        std::fs::write(
            catalog_root.join("hosted.toml"),
            r#"
schema = 1
id = "hosted"
category = "dlls"
title = "Hosted recipe"
maturity = "metadata_only"

[source]
upstream_tag = "20260125"
upstream_verb = "hosted"
"#,
        )
        .unwrap();
        let catalog = Catalog::load(
            crate::CatalogSource {
                path: catalog_root,
                version: "test".into(),
                upstream_tag: "20260125".into(),
            },
            paths.winetricks_cache.clone(),
        )
        .unwrap();
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Diagnostic test".into(),
            path: prefix_path,
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Wow64,
            runtime: None,
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        };
        let plan = crate::Planner::new(catalog.clone())
            .plan(
                crate::OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec!["hosted".into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap();
        let binary = temp.path().join("winetricks");
        std::fs::write(
            &binary,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 20260125; else echo 'server rejected checksum payload' >&2; exit 42; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut engine = OperationEngine::new(catalog, paths, store);
        engine.legacy_host = LegacyVerbHost::with_binary(binary);
        let events = Arc::new(Mutex::new(Vec::<OperationEvent>::new()));
        let captured = events.clone();
        let sink: Arc<dyn OperationEventSink> = Arc::new(move |event: OperationEvent| {
            captured.lock().push(event);
        });

        assert!(engine.run(plan, sink).await.is_err());

        let events = events.lock();
        assert!(events.iter().any(|event| {
            event
                .log_line
                .as_deref()
                .is_some_and(|line| line.contains("stderr") && line.contains("rejected checksum"))
        }));
        let failure = events
            .iter()
            .find_map(|event| event.failure.as_ref())
            .unwrap();
        assert!(failure.message.contains("exited with code 42"));
        assert!(failure.message.contains("server rejected checksum payload"));
    }

    #[test]
    fn mutation_guard_rejects_lexical_escape() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::create_dir_all(&cache).unwrap();

        assert!(ensure_mutation_in_prefix(&prefix.join("drive_c"), &prefix, &cache).is_ok());
        assert!(ensure_mutation_in_prefix(&prefix.join("../outside"), &prefix, &cache).is_err());
        assert!(ensure_mutation_in_prefix(&prefix, &prefix, &cache).is_err());
    }

    #[test]
    fn extraction_source_guard_rejects_files_outside_storage() {
        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside.cab");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("inside.cab"), b"cab").unwrap();
        std::fs::write(&outside, b"cab").unwrap();

        assert!(
            ensure_read_source_in_prefix_or_cache(&cache.join("inside.cab"), &prefix, &cache)
                .is_ok()
        );
        assert!(ensure_read_source_in_prefix_or_cache(&outside, &prefix, &cache).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn mutation_guard_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, prefix.join("escape")).unwrap();

        assert!(ensure_mutation_in_prefix(&prefix.join("escape/file"), &prefix, &cache).is_err());
    }

    #[tokio::test]
    async fn ensure_file_creates_without_truncating() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join("nested/hosts");
        tokio::fs::create_dir_all(existing.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&existing, "127.0.0.1 localhost\n")
            .await
            .unwrap();

        ensure_file(&existing).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&existing).await.unwrap(),
            "127.0.0.1 localhost\n"
        );

        let created = temp.path().join("other/services");
        ensure_file(&created).await.unwrap();
        assert_eq!(tokio::fs::metadata(created).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn verifies_checksums_without_loading_the_whole_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("archive.bin");
        tokio::fs::write(&path, b"bettertricks").await.unwrap();
        assert!(
            verify_checksum(
                &path,
                Some("93413c6ce1e4603c4adaa057e4ad0ce316e72589ba485ecc573e4a1331706139")
            )
            .await
            .is_ok()
        );
        assert!(
            verify_checksum(&path, Some(&"00".repeat(32)))
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn isolates_only_home_directory_links_and_preserves_targets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let prefix = home.join(".wine");
        let user_directory = prefix.join("drive_c/users/alice");
        let documents = home.join("Documents");
        let home_file = home.join("notes.txt");
        let inside = prefix.join("drive_c/private");
        let outside = temp.path().join("shared");
        for directory in [&user_directory, &documents, &inside, &outside] {
            std::fs::create_dir_all(directory).unwrap();
        }
        std::fs::write(&home_file, "keep").unwrap();
        symlink(&documents, user_directory.join("Documents")).unwrap();
        symlink(&home_file, user_directory.join("Notes")).unwrap();
        symlink(&inside, user_directory.join("Inside")).unwrap();
        symlink(&outside, user_directory.join("Shared")).unwrap();

        assert_eq!(isolate_home_links(&prefix, &home, "alice").unwrap(), 1);
        assert!(user_directory.join("Documents").is_dir());
        assert!(!user_directory.join("Documents").is_symlink());
        assert!(documents.is_dir());
        assert!(user_directory.join("Notes").is_symlink());
        assert!(home_file.is_file());
        assert!(user_directory.join("Inside").is_symlink());
        assert!(user_directory.join("Shared").is_symlink());
        assert_eq!(
            std::fs::read_to_string(prefix.join(".update-timestamp")).unwrap(),
            "disable\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_removal_never_follows_the_target() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let prefix = temp.path().join("prefix");
        let links = prefix.join("dosdevices");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&links).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let link = links.join("z:");
        symlink(&outside, &link).unwrap();

        remove_symlink_in_prefix(&link, &prefix).await.unwrap();
        assert!(!link.exists());
        assert!(outside.is_dir());

        let regular = links.join("c:");
        std::fs::write(&regular, "do not remove").unwrap();
        assert!(remove_symlink_in_prefix(&regular, &prefix).await.is_err());
        assert!(regular.is_file());
    }

    #[test]
    fn detects_only_samyak_oriya_font_names() {
        assert!(has_problematic_samyak_oriya_font(
            b"-misc-samyak-medium-r-normal--0-0-0-0-p-0-oriya-0\n"
        ));
        assert!(has_problematic_samyak_oriya_font(b"SAMYAK something ORIYA"));
        assert!(!has_problematic_samyak_oriya_font(
            b"samyak devanagari\noriya sans"
        ));
    }

    #[test]
    fn compares_wine_versions_without_confusing_proton_labels() {
        assert_eq!(wine_version_at_most("wine-6.21", &[6, 21]), Some(true));
        assert_eq!(
            wine_version_at_most("wine-6.21.1 (Staging)", &[6, 21]),
            Some(false)
        );
        assert_eq!(wine_version_at_most("wine-11.13", &[6, 21]), Some(false));
        assert_eq!(
            wine_version_at_most("GE-Proton9-27; wine-9.0", &[6, 21]),
            Some(false)
        );
        assert_eq!(wine_version_at_most("Proton Experimental", &[6, 21]), None);
    }

    #[test]
    fn finds_each_installed_wine_mono_variant_once() {
        let listing =
            "{old}|Wine Mono\n{runtime}|Wine Mono Runtime\n{support}|Wine Mono Windows Support\n";
        assert_eq!(
            mono_uninstaller_ids(listing),
            vec!["{support}", "{runtime}", "{old}"]
        );
    }

    #[test]
    fn escapes_dynamic_registry_strings() {
        assert_eq!(
            registry_string(r#"C:\Program Files\"Quoted\""#).unwrap(),
            r#"C:\\Program Files\\\"Quoted\\\""#
        );
        assert!(registry_string("value\n[injected]").is_err());
    }

    #[test]
    fn registry_files_use_utf16_for_non_ascii_values() {
        let encoded = encode_registry_file(
            "[HKEY_CURRENT_USER\\Software\\Wine\\Fonts\\Replacements]\n\"メイリオ\"=\"Source Han Sans\"",
        );
        assert_eq!(&encoded[..2], &[0xff, 0xfe]);
        let decoded = String::from_utf16(
            &encoded[2..]
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(decoded.starts_with("Windows Registry Editor Version 5.00\r\n"));
        assert!(decoded.contains("\"メイリオ\"=\"Source Han Sans\""));

        assert!(
            encode_registry_file("[HKEY_CURRENT_USER\\Software\\Wine]")
                .starts_with(b"REGEDIT4\n\n")
        );
    }

    #[test]
    fn dll_overrides_match_winetricks_absolute_path_semantics() {
        let registry = dll_override_registry(
            DllOverrideMode::NativeBuiltin,
            &["oleaut32".into(), "xinput1_3".into()],
            None,
        );

        assert_eq!(
            registry,
            "[HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides]\n\"*oleaut32\"=\"native,builtin\"\n\"*xinput1_3\"=\"native,builtin\""
        );
        assert!(!registry.contains("\"oleaut32\""));
    }

    #[test]
    fn app_default_override_uses_starred_delete_value() {
        let registry = dll_override_registry(
            DllOverrideMode::Default,
            &["d3d11".into()],
            Some("game.exe"),
        );

        assert_eq!(
            registry,
            "[HKEY_CURRENT_USER\\Software\\Wine\\AppDefaults\\game.exe\\DllOverrides]\n\"*d3d11\"=-"
        );
    }

    #[test]
    fn prefix_registry_updates_cover_both_wow64_views() {
        let prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: PathBuf::from("/tmp/test-prefix"),
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Wow64,
            runtime: Some(PathBuf::from("wine")),
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        };
        let invocations = registry_invocations(&prefix, RegistryArchitecture::Prefix);
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].1, r"C:\windows\syswow64\regedit.exe");
        assert_eq!(invocations[1].1, r"C:\windows\regedit.exe");
    }

    #[test]
    fn native_system_directories_follow_windows_wow64_layout() {
        let temp = tempfile::tempdir().unwrap();
        let mut prefix = WinePrefix {
            id: Uuid::new_v4(),
            name: "Test".into(),
            path: temp.path().join("prefix"),
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Win32,
            runtime: Some(PathBuf::from("wine")),
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
        };
        assert_eq!(
            system_directory_for_32_bit(&prefix).unwrap(),
            prefix.path.join("drive_c/windows/system32")
        );
        assert!(system_directory_for_64_bit(&prefix).is_err());
        assert!(!is_64_bit_prefix(&prefix));

        prefix.architecture = crate::PrefixArchitecture::Wow64;
        assert_eq!(
            system_directory_for_32_bit(&prefix).unwrap(),
            prefix.path.join("drive_c/windows/syswow64")
        );
        assert_eq!(
            system_directory_for_64_bit(&prefix).unwrap(),
            prefix.path.join("drive_c/windows/system32")
        );
        assert!(is_64_bit_prefix(&prefix));

        prefix.architecture = crate::PrefixArchitecture::Win64;
        assert!(system_directory_for_32_bit(&prefix).is_err());
        assert!(is_64_bit_prefix(&prefix));
    }
}
