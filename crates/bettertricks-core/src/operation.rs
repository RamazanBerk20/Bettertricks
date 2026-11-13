use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
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

use crate::lock::{LockProbe, PrefixLock, PrefixLocks};
use crate::planner::{plan_recipe_map, recipe_dependency_ids, step_required_tools};
use crate::process::{WineScope, spawn_supervised, stop_process_group, wait_supervised};
use crate::{
    AppPaths, BettertricksError, Catalog, DllOverrideMode, LegacyVerbHost, LegacyVerbInfo,
    LiveOperation, NativeAction, OperationEvent, OperationOptions, OperationPlan, OperationPrompt,
    OperationRecord, OperationState, PrefixArchitecture, PrefixSource, PromptChoice, PromptLevel,
    PromptResponse, Recipe, RecipeFailure, RecipeFailureKind, RecipeFile, RecipeFont,
    RecipeMaturity, RecipeStep, RecoveryManager, RegistryArchitecture, RestorePoint, Result, Store,
    WinePrefix,
};

const MAX_RECIPE_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_LIVE_PROCESS_LOG_LINES: usize = 200;
const MAX_PROCESS_LOG_LINE_CHARS: usize = 1_200;
const PROCESS_FAILURE_TAIL_LINES: usize = 8;
const MAX_PROCESS_FAILURE_DETAIL_CHARS: usize = 4_000;
/// Download progress is emitted at most this often per operation.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// A `.part` file untouched for this long belongs to a download that died (reads time out
/// after 90 s), so it is safe to delete even if another process uses the same cache.
const STALE_PARTIAL_AGE: Duration = Duration::from_secs(10 * 60);
const CREATE_PREFIX_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const INTERRUPTED_MESSAGE: &str =
    "Interrupted: Bettertricks exited while this operation was running";

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

/// Fetches one URL into `destination`, reporting progress through the context.
#[async_trait]
pub(crate) trait Fetcher: Send + Sync {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        context: &mut ExecutionContext,
    ) -> Result<()>;
}

struct HttpFetcher;

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn fetch(
        &self,
        url: &str,
        destination: &Path,
        context: &mut ExecutionContext,
    ) -> Result<()> {
        let mut builder = reqwest::Client::builder()
            .user_agent(format!("Bettertricks/{}", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(90));
        if context.plan.options.torify {
            builder = builder.proxy(reqwest::Proxy::all("socks5h://127.0.0.1:9050")?);
        }
        download_url(&builder.build()?, url, destination, context).await
    }
}

struct LiveEntry {
    plan: OperationPlan,
    cancel: Arc<AtomicBool>,
    sequence: Arc<AtomicU64>,
    sink: Arc<dyn OperationEventSink>,
    latest: Option<OperationEvent>,
    prompt: Option<(OperationPrompt, oneshot::Sender<String>)>,
}

type LiveOperations = Arc<Mutex<HashMap<Uuid, LiveEntry>>>;

/// Records the latest lifecycle event of each live operation so a reloaded UI can re-attach.
struct TrackingSink {
    id: Uuid,
    live: LiveOperations,
    inner: Arc<dyn OperationEventSink>,
}

impl OperationEventSink for TrackingSink {
    fn emit(&self, event: OperationEvent) {
        if event.log_line.is_none()
            && let Some(entry) = self.live.lock().get_mut(&self.id)
        {
            entry.latest = Some(event.clone());
        }
        self.inner.emit(event);
    }
}

#[derive(Clone)]
pub struct OperationEngine {
    catalog: Catalog,
    paths: AppPaths,
    store: Arc<Store>,
    recovery: RecoveryManager,
    legacy_host: LegacyVerbHost,
    locks: PrefixLocks,
    live: LiveOperations,
    cache_locks: Arc<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>>,
    fetcher: Arc<dyn Fetcher>,
    /// `program trash <path>`; `gio` in production, a fake in tests so no test touches the
    /// user's real Trash.
    trash_program: PathBuf,
}

impl OperationEngine {
    pub fn new(catalog: Catalog, paths: AppPaths, store: Arc<Store>) -> Self {
        Self {
            recovery: RecoveryManager::new(paths.clone(), store.clone()),
            legacy_host: LegacyVerbHost::for_catalog(&paths, &catalog),
            locks: PrefixLocks::new(&paths),
            catalog,
            paths,
            store,
            live: Arc::new(Mutex::new(HashMap::new())),
            cache_locks: Arc::new(Mutex::new(HashMap::new())),
            fetcher: Arc::new(HttpFetcher),
            trash_program: PathBuf::from("gio"),
        }
    }

    pub fn locks(&self) -> &PrefixLocks {
        &self.locks
    }

    pub fn recovery(&self) -> &RecoveryManager {
        &self.recovery
    }

    /// Resolves rows left non-terminal by a crash or quit: if no process holds the prefix
    /// lock, nobody can still be running the operation, so it becomes `Interrupted` and its
    /// restore point is released for restore or deletion. Safe to call at any time.
    ///
    /// Fails closed: only a lock that is positively free counts. A held lock, a missing lock
    /// file or a probe error leaves the row alone, and the store refuses to overwrite a row
    /// that finished meanwhile. Never waits.
    pub fn reconcile_interrupted(&self) -> Result<usize> {
        let mut resolved = 0;
        for record in self.store.unfinished_operations()? {
            if self.live.lock().contains_key(&record.id)
                || self.locks.probe(record.prefix_id) != LockProbe::Free
            {
                continue;
            }
            if self
                .store
                .mark_interrupted(record.id, INTERRUPTED_MESSAGE)?
            {
                resolved += 1;
            }
        }
        Ok(resolved)
    }

    async fn acquire(&self, prefix: &WinePrefix, holder: &str) -> Result<PrefixLock> {
        self.lock_prefix(prefix.id, holder).await
    }

    /// Takes a prefix lock on a blocking thread. Refuses when an unfinished operation is
    /// recorded for the prefix but its lock cannot be probed (lock file removed, probe error):
    /// that operation might still be running, so a second one must not start.
    async fn lock_prefix(&self, prefix_id: Uuid, holder: &str) -> Result<PrefixLock> {
        if self.locks.probe(prefix_id) == LockProbe::Unknown
            && self.store.unfinished_operations()?.iter().any(|record| {
                record.prefix_id == prefix_id && !self.live.lock().contains_key(&record.id)
            })
        {
            return Err(BettertricksError::Busy(
                "an earlier operation on this prefix is still recorded as running and its lock cannot be checked; wait for it, or restart Bettertricks to resolve it".into(),
            ));
        }
        let locks = self.locks.clone();
        let holder = holder.to_owned();
        tokio::task::spawn_blocking(move || locks.acquire(prefix_id, &holder))
            .await
            .map_err(|error| BettertricksError::Io(std::io::Error::other(error)))?
    }

    fn register_live(
        &self,
        plan: &OperationPlan,
        sink: Arc<dyn OperationEventSink>,
    ) -> (Arc<AtomicBool>, Arc<AtomicU64>, Arc<dyn OperationEventSink>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let sequence = Arc::new(AtomicU64::new(0));
        let tracking: Arc<dyn OperationEventSink> = Arc::new(TrackingSink {
            id: plan.id,
            live: self.live.clone(),
            inner: sink,
        });
        self.live.lock().insert(
            plan.id,
            LiveEntry {
                plan: plan.clone(),
                cancel: cancel.clone(),
                sequence: sequence.clone(),
                sink: tracking.clone(),
                latest: None,
                prompt: None,
            },
        );
        (cancel, sequence, tracking)
    }

    /// Takes the prefix lock (failing fast with `Busy`), records the operation and runs it in
    /// the background. Desktop entry point.
    pub async fn start(
        &self,
        plan: OperationPlan,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<Uuid> {
        let id = plan.id;
        let lock = self
            .acquire(&plan.prefix, &format!("operation {id}"))
            .await?;
        self.store.upsert_operation(&planned_record(&plan))?;
        let (cancel, sequence, sink) = self.register_live(&plan, sink);
        let engine = self.clone();
        tokio::spawn(async move {
            let result = engine.run_plan(plan, cancel, sequence, sink, lock).await;
            if let Err(error) = result {
                tracing::error!(operation_id = %id, %error, "operation failed");
            }
            engine.live.lock().remove(&id);
        });
        Ok(id)
    }

    /// Same lifecycle as [`Self::start`], awaited in the caller. CLI entry point.
    pub async fn run(&self, plan: OperationPlan, sink: Arc<dyn OperationEventSink>) -> Result<()> {
        let id = plan.id;
        let lock = self
            .acquire(&plan.prefix, &format!("operation {id}"))
            .await?;
        self.store.upsert_operation(&planned_record(&plan))?;
        let (cancel, sequence, sink) = self.register_live(&plan, sink);
        let result = self.run_plan(plan, cancel, sequence, sink, lock).await;
        self.live.lock().remove(&id);
        result
    }

    pub fn live_operations(&self) -> Vec<LiveOperation> {
        let mut operations = self
            .live
            .lock()
            .values()
            .map(|entry| LiveOperation {
                plan: entry.plan.clone(),
                state: entry
                    .latest
                    .as_ref()
                    .map(|event| event.state)
                    .unwrap_or(OperationState::Planned),
                latest: entry.latest.clone(),
                prompt: entry.prompt.as_ref().map(|(prompt, _)| prompt.clone()),
            })
            .collect::<Vec<_>>();
        operations.sort_by_key(|operation| operation.plan.id);
        operations
    }

    pub fn cancel(&self, operation_id: Uuid) -> Result<()> {
        let (sink, event, prompt) = {
            let mut live = self.live.lock();
            let entry = live
                .get_mut(&operation_id)
                .ok_or_else(|| BettertricksError::OperationNotFound(operation_id.to_string()))?;
            if entry.cancel.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            let latest = entry.latest.as_ref();
            let event = OperationEvent {
                operation_id,
                sequence: entry.sequence.fetch_add(1, Ordering::SeqCst) + 1,
                state: OperationState::Cancelling,
                step: latest.map(|event| event.step).unwrap_or_default(),
                total_steps: entry.plan.steps.len(),
                recipe_id: latest.and_then(|event| event.recipe_id.clone()),
                title: "Cancelling".into(),
                detail: Some("Stopping Wine processes in this prefix".into()),
                progress: latest.and_then(|event| event.progress),
                prompt: None,
                log_line: None,
                failure: None,
                timestamp: Utc::now(),
            };
            (entry.sink.clone(), event, entry.prompt.take())
        };
        if let Err(error) = self.store.set_live_operation_state(
            operation_id,
            OperationState::Cancelling,
            "Cancelling",
        ) {
            tracing::warn!(%error, "could not record cancellation");
        }
        sink.emit(event);
        if let Some((_, sender)) = prompt {
            let _ = sender.send("cancel".into());
        }
        Ok(())
    }

    pub fn respond(&self, response: PromptResponse) -> Result<()> {
        let sender = {
            let mut live = self.live.lock();
            let entry = live.get_mut(&response.operation_id).ok_or_else(|| {
                BettertricksError::OperationNotFound(response.operation_id.to_string())
            })?;
            match &entry.prompt {
                Some((prompt, _)) if prompt.id == response.prompt_id => {}
                _ => {
                    return Err(BettertricksError::OperationNotFound(
                        response.prompt_id.to_string(),
                    ));
                }
            }
            entry.prompt.take().map(|(_, sender)| sender)
        };
        sender
            .ok_or(BettertricksError::Cancelled)?
            .send(response.choice_id)
            .map_err(|_| BettertricksError::Cancelled)
    }

    async fn run_plan(
        &self,
        plan: OperationPlan,
        cancellation: Arc<AtomicBool>,
        sequence: Arc<AtomicU64>,
        sink: Arc<dyn OperationEventSink>,
        _lock: PrefixLock,
    ) -> Result<()> {
        let mut context = ExecutionContext::new(
            plan.clone(),
            cancellation,
            sink,
            self.paths.clone(),
            self.store.clone(),
        );
        context.sequence = sequence;
        let recipes = match plan_recipe_map(&self.catalog, &plan) {
            Ok(recipes) => recipes,
            Err(error) => return self.finish_error(&mut context, error).await,
        };
        if let Err(error) = validate_plan_inputs(&recipes, &plan) {
            return self.finish_error(&mut context, error).await;
        }

        context.transition(OperationState::Preflight, Some("Running preflight checks"));
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
            if context.cancelled() {
                return self.finish_cancelled(&mut context).await;
            }
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
        if context.cancelled() {
            return self.finish_cancelled(&mut context).await;
        }

        context.started_at = Some(Utc::now());
        context.transition(OperationState::Running, Some("Operation running"));
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
                context.transition(
                    OperationState::Running,
                    Some("Continuing after a dependency failure"),
                );
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
                    context.transition(
                        OperationState::Running,
                        Some("Continuing after a component failure"),
                    );
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
            context.transition(OperationState::Succeeded, Some("Operation complete"));
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
            context.transition(OperationState::Failed, Some(&summary));
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
            context.transition(OperationState::Running, Some(&title));
            self.run_legacy_recipe(recipe, context).await?;
        }

        let scratch = context
            .plan
            .prefix
            .path
            .join("drive_c/windows/temp/bettertricks")
            .join(&recipe.id);
        if recipe.maturity == RecipeMaturity::Native {
            // Never let a previous run's extraction satisfy this run's steps.
            remove_scratch(&scratch, &context.plan.prefix.path).await?;
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
            context.transition(OperationState::Running, Some(&title));
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
        if !context.plan.options.no_clean {
            remove_scratch(&scratch, &context.plan.prefix.path).await?;
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
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = spawn_supervised(&mut command).await?;
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
                context.log(format!(
                    "[{}] Stopping Winetricks, its installers and the prefix's wineserver",
                    recipe.id
                ));
                stop_process_group(&mut child, Some(&wine_scope(&context.plan.prefix))).await;
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
                ensure_mutation_in_prefix(
                    &path,
                    &context.plan.prefix.path,
                    &context.paths.winetricks_cache,
                )?;
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

    /// Cache entry lifecycle: Present → verified, or quarantined and treated as Absent;
    /// Absent → `.part` per URL → verified and renamed; a mismatching mirror falls through to
    /// the next URL and never leaves its `.part` behind.
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
            match verify_checksum(&destination, file.sha256.as_deref()).await {
                Ok(()) => {
                    context.log(format!("Using cached {}", destination.display()));
                    return Ok(());
                }
                Err(BettertricksError::ChecksumMismatch { .. }) => {
                    // Keep only the latest rejected copy per entry so repeated corruption
                    // cannot grow the cache without bound.
                    remove_quarantined(&destination, &file.filename).await;
                    let quarantined = destination.with_file_name(format!(
                        "{}.bettertricks-invalid-{}",
                        file.filename,
                        Uuid::new_v4()
                    ));
                    tokio::fs::rename(&destination, &quarantined).await?;
                    context.log(format!(
                        "Cached {} failed its checksum; moved it to {} and downloading again",
                        destination.display(),
                        quarantined.display()
                    ));
                }
                Err(error) => return Err(error),
            }
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
        remove_stale_partials(&destination, &file.filename).await;
        let partial = destination.with_file_name(format!(
            "{}.{}.part",
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            context.plan.id
        ));

        let mut last_error = None;
        for url in &file.urls {
            if context.cancelled() {
                return Err(BettertricksError::Cancelled);
            }
            let result = match self.fetcher.fetch(url, &partial, context).await {
                Ok(()) => verify_checksum(&partial, file.sha256.as_deref()).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => {
                    tokio::fs::rename(&partial, &destination).await?;
                    return Ok(());
                }
                Err(error) => {
                    let _ = tokio::fs::remove_file(&partial).await;
                    if matches!(error, BettertricksError::Cancelled) {
                        return Err(error);
                    }
                    context.log(format!("Download source failed: {url}: {error}"));
                    last_error = Some(error);
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
        {
            // Registered under the same lock `cancel` takes, so a cancel can never miss it.
            let mut live = self.live.lock();
            if context.cancelled() {
                return Err(BettertricksError::Cancelled);
            }
            if let Some(entry) = live.get_mut(&context.plan.id) {
                entry.prompt = Some((prompt.clone(), sender));
            }
        }
        context.transition(OperationState::WaitingForUser, Some(title));
        context.emit(
            OperationState::WaitingForUser,
            title,
            Some(message.into()),
            Some(context.fraction()),
            Some(prompt),
        );
        let choice = receiver.await.map_err(|_| BettertricksError::Cancelled)?;
        if choice == "continue" && !context.cancelled() {
            context.transition(OperationState::Running, Some(title));
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
        context.transition(OperationState::Failed, Some(&detail));
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
        context.transition(OperationState::Cancelled, Some("Operation cancelled"));
        Err(BettertricksError::Cancelled)
    }

    /// Manual restore point, under the prefix lock.
    pub async fn create_restore_point(&self, prefix: &WinePrefix) -> Result<RestorePoint> {
        let _lock = self.acquire(prefix, "restore point").await?;
        self.recovery.create(prefix, None).await
    }

    /// Restores a snapshot under its prefix's lock. With `replace_existing`, a prefix that is
    /// still on disk is first moved to the desktop Trash (recoverable). A manually registered
    /// prefix is registered again so it reappears in the app.
    pub async fn restore(&self, point: &RestorePoint, replace_existing: bool) -> Result<()> {
        let _lock = self.lock_prefix(point.prefix_id, "restore").await?;
        if replace_existing && point.prefix_path.exists() {
            let path = crate::validate_existing_prefix_path(&point.prefix_path)?;
            move_to_trash(&self.trash_program, &path).await?;
        }
        self.recovery.restore(point).await?;
        if point.prefix_source == Some(PrefixSource::Manual) {
            self.store.register_prefix(
                &point.prefix_path,
                &point.prefix_name,
                PrefixSource::Manual,
                point.runtime.as_deref(),
            )?;
        }
        Ok(())
    }

    /// Moves a prefix to the desktop Trash under its lock and forgets a manual registration.
    pub async fn trash_prefix(&self, prefix: &WinePrefix) -> Result<()> {
        let _lock = self.acquire(prefix, "move to Trash").await?;
        let path = crate::validate_existing_prefix_path(&prefix.path)?;
        move_to_trash(&self.trash_program, &path).await?;
        let _ = self.store.unregister_prefix(&path);
        Ok(())
    }

    /// Runs a reviewed custom `.verb` under the prefix lock, after a restore point, in its own
    /// process group.
    pub async fn run_legacy_verb(
        &self,
        info: &LegacyVerbInfo,
        prefix: &WinePrefix,
        options: &OperationOptions,
    ) -> Result<()> {
        let _lock = self
            .acquire(prefix, &format!("custom verb {}", info.id))
            .await?;
        if prefix.exists {
            self.recovery.create(prefix, None).await?;
        }
        let mut command = self.legacy_host.verb_command(info, prefix, options)?;
        let mut child = spawn_supervised(&mut command).await?;
        let status = wait_supervised(&mut child, || false, None).await?;
        if !status.success() {
            return Err(BettertricksError::CommandFailed {
                program: "winetricks".into(),
                code: status.code(),
            });
        }
        Ok(())
    }

    /// Starts a Wine tool (winecfg, regedit, an installer…) in the prefix. The prefix stays
    /// locked until the tool exits, so no operation can change the prefix underneath it.
    /// The tool runs in its own process group, which [`crate::stop_all_groups`] kills when the
    /// app quits.
    pub async fn launch_tool(
        &self,
        prefix: &WinePrefix,
        program: &str,
        arguments: &[String],
    ) -> Result<()> {
        let lock = self.acquire(prefix, &format!("{program} window")).await?;
        let mut command = Command::new(wine_binary(prefix));
        command
            .arg(program)
            .args(arguments)
            .env("WINEPREFIX", &prefix.path)
            .stdin(Stdio::null());
        let mut child = crate::process::spawn_group(&mut command).await?;
        tokio::spawn(async move {
            let _ = child.wait().await;
            drop(lock);
        });
        Ok(())
    }

    /// Initializes a new prefix in a staging directory and publishes it atomically, under the
    /// lock of the prefix id it will have.
    pub async fn create_prefix(
        &self,
        path: &Path,
        name: &str,
        architecture: PrefixArchitecture,
        runtime: Option<&Path>,
    ) -> Result<()> {
        let path = crate::validate_new_prefix_path(path)?;
        let _lock = self
            .lock_prefix(crate::prefix::prefix_id_for_path(&path), "create prefix")
            .await?;
        if path.exists() && path.read_dir()?.next().is_some() {
            return Err(BettertricksError::Conflict(
                "The selected directory is not empty".into(),
            ));
        }
        let parent = path.parent().ok_or_else(|| {
            BettertricksError::Security("Prefix path has no parent directory".into())
        })?;
        tokio::fs::create_dir_all(parent).await?;
        let file_name = path.file_name().ok_or_else(|| {
            BettertricksError::Security("Prefix path has no directory name".into())
        })?;
        let staging = parent.join(format!(
            ".{}.bettertricks-create-{}.partial",
            file_name.to_string_lossy(),
            Uuid::new_v4()
        ));
        let staged = WinePrefix {
            id: Uuid::nil(),
            name: name.into(),
            path: staging.clone(),
            source: PrefixSource::Manual,
            architecture,
            runtime: runtime.map(Path::to_path_buf),
            runtime_label: None,
            managed: false,
            exists: false,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
            status: crate::PrefixStatus::Missing,
        };
        let scope = wine_scope(&staged);
        let result = async {
            let mut boot = wineboot_command(&staged);
            let mut child = spawn_supervised(&mut boot).await?;
            let deadline = Instant::now() + CREATE_PREFIX_TIMEOUT;
            let status = wait_supervised(&mut child, || Instant::now() > deadline, Some(&scope))
                .await
                .map_err(|error| match error {
                    BettertricksError::Cancelled => BettertricksError::Unsupported(
                        "wineboot did not finish within five minutes".into(),
                    ),
                    error => error,
                })?;
            if !status.success() {
                return Err(BettertricksError::CommandFailed {
                    program: "wineboot".into(),
                    code: status.code(),
                });
            }
            if !crate::process::wineserver_idle(
                &scope.wineserver,
                &staging,
                Duration::from_secs(60),
            )
            .await
            {
                return Err(BettertricksError::Unsupported(
                    "wineserver did not finish prefix initialization cleanly".into(),
                ));
            }
            if path.exists() {
                tokio::fs::remove_dir(&path).await.map_err(|error| {
                    BettertricksError::Conflict(format!(
                        "The target directory stopped being empty: {error}"
                    ))
                })?;
            }
            tokio::fs::rename(&staging, &path).await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            crate::process::kill_wineserver(&scope).await;
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return result;
        }
        self.store
            .register_prefix(&path, name, PrefixSource::Manual, runtime)
    }
}

fn planned_record(plan: &OperationPlan) -> OperationRecord {
    OperationRecord {
        id: plan.id,
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
    }
}

async fn move_to_trash(program: &Path, path: &Path) -> Result<()> {
    let mut command = Command::new(program);
    command.arg("trash").arg(path);
    let status = crate::process::spawn_retrying(&mut command)
        .await?
        .wait()
        .await?;
    if !status.success() {
        return Err(BettertricksError::CommandFailed {
            program: "gio trash".into(),
            code: status.code(),
        });
    }
    Ok(())
}

pub(crate) fn wine_binary(prefix: &WinePrefix) -> PathBuf {
    prefix
        .runtime
        .clone()
        .unwrap_or_else(|| PathBuf::from("wine"))
}

pub(crate) fn wine_scope(prefix: &WinePrefix) -> WineScope {
    WineScope {
        wineserver: wine_tool(prefix, "wineserver"),
        prefix: prefix.path.clone(),
    }
}

fn wineboot_command(prefix: &WinePrefix) -> Command {
    let mut command = Command::new(wine_binary(prefix));
    command
        .arg("wineboot")
        .arg("-u")
        .env("WINEPREFIX", &prefix.path)
        .env(
            "WINEARCH",
            match prefix.architecture {
                PrefixArchitecture::Win32 => "win32",
                _ => "win64",
            },
        );
    command
}

async fn remove_scratch(scratch: &Path, prefix: &Path) -> Result<()> {
    match tokio::fs::symlink_metadata(scratch).await {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            ensure_mutation_in_prefix(scratch, prefix, prefix)?;
            tokio::fs::remove_dir_all(scratch).await?;
        }
        Ok(_) => tokio::fs::remove_file(scratch).await?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

async fn remove_quarantined(destination: &Path, filename: &str) {
    let Some(parent) = destination.parent() else {
        return;
    };
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return;
    };
    let prefix = format!("{filename}.bettertricks-invalid-");
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// Deletes `.part` files of this cache entry that no live download is writing to.
async fn remove_stale_partials(destination: &Path, filename: &str) {
    let Some(parent) = destination.parent() else {
        return;
    };
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return;
    };
    let prefix = format!("{filename}.");
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(".part") {
            continue;
        }
        let stale = entry
            .metadata()
            .await
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_PARTIAL_AGE);
        if stale {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
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

pub(crate) fn wine_tool(prefix: &WinePrefix, tool: &str) -> PathBuf {
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
            ensure_mutation_in_prefix(&path, &prefix.path, &context.paths.winetricks_cache)?;
            tokio::fs::remove_file(path).await?;
        }
    }
    context.log(format!("Removed {} Wine Mono installer(s).", ids.len()));
    Ok(())
}

pub(crate) struct ExecutionContext {
    plan: OperationPlan,
    cancellation: Arc<AtomicBool>,
    sink: Arc<dyn OperationEventSink>,
    paths: AppPaths,
    store: Arc<Store>,
    state: OperationState,
    last_progress: Option<Instant>,
    sequence: Arc<AtomicU64>,
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
        store: Arc<Store>,
    ) -> Self {
        Self {
            plan,
            cancellation,
            sink,
            paths,
            store,
            state: OperationState::Planned,
            last_progress: None,
            sequence: Arc::new(AtomicU64::new(0)),
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

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// A live state requested while a cancel is pending is reported as `Cancelling`.
    fn effective(&self, state: OperationState) -> OperationState {
        if self.cancelled() && !state.is_terminal() {
            OperationState::Cancelling
        } else {
            state
        }
    }

    /// The one place the operation lifecycle advances: validates the transition and persists
    /// it. Non-terminal write failures are logged so a busy database cannot strand the
    /// operation; the terminal event has always been emitted before this is called.
    fn transition(&mut self, next: OperationState, message: Option<&str>) {
        let next = self.effective(next);
        if !self.state.can_transition_to(next) {
            tracing::error!(from = ?self.state, to = ?next, "rejected operation state transition");
            debug_assert!(false, "invalid transition {:?} -> {next:?}", self.state);
            return;
        }
        self.state = next;
        let record = OperationRecord {
            id: self.plan.id,
            prefix_id: self.plan.prefix.id,
            prefix_name: self.plan.prefix.name.clone(),
            recipes: self.plan.resolved_recipes.clone(),
            state: next,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: next
                .is_terminal()
                .then(|| self.finished_at.unwrap_or_else(Utc::now)),
            current_step: self.step,
            total_steps: self.plan.steps.len(),
            message: message.map(str::to_owned),
            failures: self.failures.clone(),
        };
        let attempts = if next.is_terminal() { 3 } else { 1 };
        for attempt in 1..=attempts {
            match self.store.upsert_operation(&record) {
                Ok(()) => break,
                Err(error) => tracing::warn!(%error, attempt, "could not persist operation state"),
            }
        }
    }

    /// Download progress, throttled per operation so a large file cannot flood the UI.
    fn download_progress(&mut self, downloaded: u64, total: Option<u64>) {
        let Some(total) = total.filter(|total| *total > 0) else {
            return;
        };
        let now = Instant::now();
        if downloaded < total
            && self
                .last_progress
                .is_some_and(|last| now.duration_since(last) < PROGRESS_INTERVAL)
        {
            return;
        }
        self.last_progress = Some(now);
        self.emit(
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
        let state = self.effective(state);
        self.sink.emit(OperationEvent {
            operation_id: self.plan.id,
            sequence: self.next_sequence(),
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
        self.sink.emit(OperationEvent {
            operation_id: self.plan.id,
            sequence: self.next_sequence(),
            state: self.effective(self.state),
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
        self.sink.emit(OperationEvent {
            operation_id: self.plan.id,
            sequence: self.next_sequence(),
            state: self.effective(self.state),
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
    // Never run wineboot over a directory that holds something other than a Wine prefix.
    if prefix.path.exists() && prefix.path.read_dir()?.next().is_some() {
        return Err(BettertricksError::Conflict(format!(
            "{} is not an initialized Wine prefix and is not empty",
            prefix.path.display()
        )));
    }
    tokio::fs::create_dir_all(&prefix.path).await?;
    let mut command = wineboot_command(prefix);
    let mut child = spawn_supervised(&mut command).await?;
    let status = wait_supervised(
        &mut child,
        || context.cancelled(),
        Some(&wine_scope(prefix)),
    )
    .await?;
    if !status.success() {
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
        context.download_progress(downloaded, total);
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
    let mut command = Command::new(&tool);
    command
        .arg("-w")
        .current_dir(context.plan.prefix.path.join("drive_c"))
        .env("WINEPREFIX", &context.plan.prefix.path);
    let mut child = spawn_supervised(&mut command).await?;
    let scope = wine_scope(&context.plan.prefix);
    let status = wait_supervised(&mut child, || context.cancelled(), Some(&scope)).await?;
    if status.success() {
        Ok(())
    } else {
        Err(BettertricksError::CommandFailed {
            program: tool.to_string_lossy().into_owned(),
            code: status.code(),
        })
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
    let mut child = spawn_supervised(&mut command).await?;
    let status = wait_supervised(&mut child, || context.cancelled(), None).await?;
    if status.success() {
        Ok(())
    } else {
        Err(BettertricksError::CommandFailed {
            program,
            code: status.code(),
        })
    }
}

async fn run_wine(
    prefix: &WinePrefix,
    program: &str,
    arguments: &[String],
    environment: &BTreeMap<String, String>,
    context: &mut ExecutionContext,
) -> Result<()> {
    let mut command = Command::new(wine_binary(prefix));
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
    let mut child = spawn_supervised(&mut command).await?;
    let status = wait_supervised(
        &mut child,
        || context.cancelled(),
        Some(&wine_scope(prefix)),
    )
    .await?;
    if status.success() {
        return Ok(());
    }
    Err(BettertricksError::CommandFailed {
        program: program.into(),
        code: status.code(),
    })
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
            status: Default::default(),
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
            status: Default::default(),
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
            status: Default::default(),
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

    /// A released lock may read held for a few ms (see `RELEASE_GRACE`); poll briefly.
    async fn wait_until_free(locks: &PrefixLocks, prefix: Uuid) -> bool {
        for _ in 0..100 {
            if locks.probe(prefix) == LockProbe::Free {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    fn test_prefix(path: PathBuf, runtime: Option<PathBuf>) -> WinePrefix {
        WinePrefix {
            id: crate::prefix_id_for_path(&path),
            name: "Test".into(),
            path,
            source: crate::PrefixSource::Manual,
            architecture: crate::PrefixArchitecture::Wow64,
            runtime,
            runtime_label: None,
            managed: false,
            exists: true,
            installed_verbs: Vec::new(),
            size_bytes: None,
            last_modified: None,
            status: Default::default(),
        }
    }

    fn write_recipe(root: &Path, id: &str, body: &str, upstream_tag: &str) {
        std::fs::write(
            root.join(format!("{id}.toml")),
            format!(
                "schema = 1\nid = \"{id}\"\ncategory = \"settings\"\ntitle = \"{id}\"\n{body}\n[source]\nupstream_tag = \"{upstream_tag}\"\nupstream_verb = \"{id}\"\n"
            ),
        )
        .unwrap();
    }

    fn load_catalog(root: PathBuf, paths: &AppPaths, tag: &str) -> Catalog {
        Catalog::load(
            crate::CatalogSource {
                path: root,
                version: "test".into(),
                upstream_tag: tag.into(),
            },
            paths.winetricks_cache.clone(),
        )
        .unwrap()
    }

    fn plan_for(catalog: &Catalog, recipe: &str, prefix: WinePrefix) -> OperationPlan {
        crate::Planner::new(catalog.clone())
            .plan(
                crate::OperationRequest {
                    prefix_id: prefix.id,
                    recipes: vec![recipe.into()],
                    input_values: Default::default(),
                    options: Default::default(),
                },
                prefix,
            )
            .unwrap()
    }

    fn recording_sink() -> (Arc<dyn OperationEventSink>, Arc<Mutex<Vec<OperationEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::<OperationEvent>::new()));
        let captured = events.clone();
        (
            Arc::new(move |event: OperationEvent| captured.lock().push(event)),
            events,
        )
    }

    /// A `gio`-compatible `trash` that moves into `<root>/trash`.
    fn fake_trash(root: &Path) -> PathBuf {
        let trash = root.join("trash");
        std::fs::create_dir_all(&trash).unwrap();
        let program = root.join("fake-gio");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\n[ \"$1\" = trash ] && mv \"$2\" '{}/'\n",
                trash.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    fn fake_wine(root: &Path) -> (PathBuf, PathBuf) {
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = root.join("wineserver.log");
        std::fs::write(
            bin.join("wineserver"),
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(
            bin.join("wineserver"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        (bin.join("wine"), log)
    }

    #[tokio::test]
    async fn cancel_stops_winetricks_descendants_and_wineserver_before_releasing_the_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        write_recipe(
            &catalog_root,
            "hosted",
            "maturity = \"metadata_only\"",
            "20260125",
        );
        let catalog = load_catalog(catalog_root, &paths, "20260125");
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let (wine, wineserver_log) = fake_wine(temp.path());
        let prefix = test_prefix(prefix_path.clone(), Some(wine));
        let plan = plan_for(&catalog, "hosted", prefix.clone());
        let binary = temp.path().join("winetricks");
        // An installer that outlives its Winetricks parent, like msiexec does.
        std::fs::write(
            &binary,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 20260125; exit 0; fi\nsleep 60 &\necho $! > \"$WINEPREFIX/installer.pid\"\nwait\n",
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let store = Arc::new(Store::open(&paths).unwrap());
        let mut engine = OperationEngine::new(catalog, paths.clone(), store.clone());
        engine.legacy_host = LegacyVerbHost::with_binary(binary);
        let (sink, events) = recording_sink();
        let operation_id = plan.id;
        let runner = engine.clone();
        let task = tokio::spawn(async move { runner.run(plan, sink).await });
        let pid_file = prefix_path.join("installer.pid");
        for _ in 0..500 {
            if pid_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(pid_file.exists(), "child never started");
        let installer: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();

        engine.cancel(operation_id).unwrap();
        let result = task.await.unwrap();

        assert!(matches!(result, Err(BettertricksError::Cancelled)));
        assert!(
            crate::process::process_gone(installer).await,
            "installer {installer} kept running after cancel"
        );
        assert!(
            std::fs::read_to_string(&wineserver_log)
                .unwrap()
                .contains("-k")
        );
        assert!(wait_until_free(&engine.locks, prefix.id).await);
        let events = events.lock();
        assert!(
            events
                .iter()
                .any(|event| event.state == OperationState::Cancelling)
        );
        assert_eq!(events.last().unwrap().state, OperationState::Cancelled);
        let record = store.operations(1).unwrap().remove(0);
        assert_eq!(record.state, OperationState::Cancelled);
    }

    #[tokio::test]
    async fn one_mutating_task_per_prefix_across_engines_and_prompts_survive_reattach() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        write_recipe(
            &catalog_root,
            "ask",
            "maturity = \"native\"\n[[steps]]\ntype = \"prompt\"\nlevel = \"confirmation\"\ntitle = \"Continue?\"\nmessage = \"Answer me\"",
            "test",
        );
        let catalog = load_catalog(catalog_root, &paths, "test");
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let prefix = test_prefix(prefix_path, None);
        let plan = plan_for(&catalog, "ask", prefix.clone());
        let operation_id = plan.id;
        // Desktop and CLI: separate engines (separate processes) sharing one state directory.
        let desktop = OperationEngine::new(
            catalog.clone(),
            paths.clone(),
            Arc::new(Store::open(&paths).unwrap()),
        );
        let mut cli = OperationEngine::new(
            catalog.clone(),
            paths.clone(),
            Arc::new(Store::open(&paths).unwrap()),
        );
        cli.trash_program = fake_trash(temp.path());
        let (sink, events) = recording_sink();
        desktop.start(plan, sink).await.unwrap();
        let prompt = loop {
            if let Some(prompt) = desktop
                .live_operations()
                .into_iter()
                .find_map(|operation| operation.prompt)
            {
                break prompt;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(
            desktop.live_operations()[0].state,
            OperationState::WaitingForUser
        );

        let second = plan_for(&catalog, "ask", prefix.clone());
        let (cli_sink, _) = recording_sink();
        assert!(matches!(
            cli.run(second, cli_sink).await,
            Err(BettertricksError::Busy(_))
        ));
        assert!(matches!(
            cli.create_restore_point(&prefix).await,
            Err(BettertricksError::Busy(_))
        ));
        assert!(matches!(
            cli.trash_prefix(&prefix).await,
            Err(BettertricksError::Busy(_))
        ));
        // The crashed-run reconciliation must not touch an operation another engine owns.
        assert_eq!(cli.reconcile_interrupted().unwrap(), 0);

        desktop
            .respond(PromptResponse {
                operation_id,
                prompt_id: prompt.id,
                choice_id: "continue".into(),
            })
            .unwrap();
        while !events
            .lock()
            .last()
            .is_some_and(|event| event.state.is_terminal())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            events.lock().last().unwrap().state,
            OperationState::Succeeded
        );
        assert!(wait_until_free(&desktop.locks, prefix.id).await);
        assert!(desktop.live_operations().is_empty());
    }

    #[tokio::test]
    async fn startup_reconciliation_interrupts_orphans_and_frees_their_restore_points() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        write_recipe(
            &catalog_root,
            "noop",
            "maturity = \"native\"\n[[steps]]\ntype = \"native_action\"\naction = \"noop\"",
            "test",
        );
        let catalog = load_catalog(catalog_root, &paths, "test");
        let store = Arc::new(Store::open(&paths).unwrap());
        let orphan = |prefix_id| OperationRecord {
            id: Uuid::new_v4(),
            prefix_id,
            prefix_name: "Game".into(),
            recipes: vec!["noop".into()],
            state: OperationState::Running,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            current_step: 1,
            total_steps: 2,
            message: None,
            failures: Vec::new(),
        };
        let crashed = orphan(Uuid::new_v4());
        let elsewhere = orphan(Uuid::new_v4());
        store.upsert_operation(&crashed).unwrap();
        store.upsert_operation(&elsewhere).unwrap();
        let snapshot = paths.backups.join("snapshot.tar.zst");
        std::fs::write(&snapshot, b"snapshot").unwrap();
        let point = RestorePoint {
            id: Uuid::new_v4(),
            prefix_id: crashed.prefix_id,
            prefix_name: "Game".into(),
            prefix_path: temp.path().join("game"),
            storage_path: snapshot,
            method: crate::RestoreMethod::Archive,
            created_at: Utc::now(),
            size_bytes: None,
            operation_id: Some(crashed.id),
            prefix_source: None,
            runtime: None,
        };
        store.add_restore_point(&point).unwrap();
        assert!(store.restore_point_is_protected(&point).unwrap());
        // The crashed process had taken (and lost, by dying) its lock; the lock file remains.
        drop(
            PrefixLocks::new(&paths)
                .acquire(crashed.prefix_id, "crashed")
                .unwrap(),
        );
        // A row whose lock file is missing cannot be judged and is left alone.
        let unknown = orphan(Uuid::new_v4());
        store.upsert_operation(&unknown).unwrap();
        // Another live process still owns the second operation's prefix.
        let other_process = PrefixLocks::new(&paths);
        let _held = other_process.acquire(elsewhere.prefix_id, "cli").unwrap();

        let engine = OperationEngine::new(catalog, paths, store.clone());
        assert!(wait_until_free(&engine.locks, crashed.prefix_id).await);
        assert_eq!(engine.reconcile_interrupted().unwrap(), 1);

        let records = store.operations(10).unwrap();
        let state = |id| records.iter().find(|record| record.id == id).unwrap().state;
        assert_eq!(state(crashed.id), OperationState::Interrupted);
        assert_eq!(state(elsewhere.id), OperationState::Running);
        assert_eq!(state(unknown.id), OperationState::Running);
        assert!(!store.restore_point_is_protected(&point).unwrap());
        engine.recovery.delete(point.id).await.unwrap();
        assert_eq!(store.clear_operation_history().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_running_row_with_an_unprobeable_lock_blocks_new_work() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        write_recipe(
            &catalog_root,
            "noop",
            "maturity = \"native\"\n[[steps]]\ntype = \"native_action\"\naction = \"noop\"",
            "test",
        );
        let catalog = load_catalog(catalog_root, &paths, "test");
        let store = Arc::new(Store::open(&paths).unwrap());
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let prefix = test_prefix(prefix_path, None);
        // Another process's operation, whose lock file was deleted by a cleanup tool.
        store
            .upsert_operation(&OperationRecord {
                id: Uuid::new_v4(),
                prefix_id: prefix.id,
                prefix_name: "Test".into(),
                recipes: Vec::new(),
                state: OperationState::Running,
                created_at: Utc::now(),
                started_at: None,
                finished_at: None,
                current_step: 0,
                total_steps: 0,
                message: None,
                failures: Vec::new(),
            })
            .unwrap();
        let engine = OperationEngine::new(catalog, paths, store.clone());

        assert_eq!(engine.reconcile_interrupted().unwrap(), 0);
        assert!(matches!(
            engine.create_restore_point(&prefix).await,
            Err(BettertricksError::Busy(_))
        ));
    }

    struct FakeFetcher(HashMap<String, Vec<u8>>);

    #[async_trait]
    impl Fetcher for FakeFetcher {
        async fn fetch(
            &self,
            url: &str,
            destination: &Path,
            context: &mut ExecutionContext,
        ) -> Result<()> {
            let bytes = self
                .0
                .get(url)
                .ok_or_else(|| BettertricksError::Io(std::io::Error::other("404")))?;
            tokio::fs::write(destination, bytes).await?;
            context.download_progress(bytes.len() as u64, Some(bytes.len() as u64));
            Ok(())
        }
    }

    #[tokio::test]
    async fn quarantines_bad_cache_and_falls_through_mismatching_mirrors() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        let good = b"bettertricks".to_vec();
        write_recipe(
            &catalog_root,
            "fetch",
            "maturity = \"native\"\n[[files]]\nid = \"payload\"\nfilename = \"payload.bin\"\nurls = [\"https://bad.example/payload.bin\", \"https://good.example/payload.bin\"]\nsha256 = \"93413c6ce1e4603c4adaa057e4ad0ce316e72589ba485ecc573e4a1331706139\"\n[[steps]]\ntype = \"download\"\nfile = \"payload\"",
            "test",
        );
        let catalog = load_catalog(catalog_root, &paths, "test");
        let cached = paths.winetricks_cache.join("fetch/payload.bin");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        // Truncated leftover from an interrupted Winetricks download.
        std::fs::write(&cached, b"better").unwrap();
        let prefix_path = temp.path().join("prefix");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        let plan = plan_for(&catalog, "fetch", test_prefix(prefix_path, None));
        assert_eq!(plan.downloads[0].cache_state, crate::CacheState::Present);
        let mut engine = OperationEngine::new(
            catalog,
            paths.clone(),
            Arc::new(Store::open_in_memory().unwrap()),
        );
        engine.fetcher = Arc::new(FakeFetcher(HashMap::from([
            (
                "https://bad.example/payload.bin".to_owned(),
                b"tampered mirror".to_vec(),
            ),
            ("https://good.example/payload.bin".to_owned(), good.clone()),
        ])));
        let (sink, _) = recording_sink();

        engine.run(plan, sink).await.unwrap();

        assert_eq!(std::fs::read(&cached).unwrap(), good);
        let names = std::fs::read_dir(cached.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(
            names
                .iter()
                .any(|name| name.contains("bettertricks-invalid")),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name.ends_with(".part")),
            "{names:?}"
        );
    }

    #[test]
    fn download_progress_is_throttled_per_operation() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let prefix = test_prefix(temp.path().join("prefix"), None);
        let plan = OperationPlan {
            id: Uuid::new_v4(),
            prefix,
            requested_recipes: Vec::new(),
            resolved_recipes: Vec::new(),
            steps: Vec::new(),
            inputs: Vec::new(),
            downloads: Vec::new(),
            conflicts: Vec::new(),
            warnings: Vec::new(),
            restore_recommended: false,
            estimated_download_bytes: None,
            options: Default::default(),
        };
        let (sink, events) = recording_sink();
        let mut context = ExecutionContext::new(
            plan,
            Arc::new(AtomicBool::new(false)),
            sink,
            paths,
            Arc::new(Store::open_in_memory().unwrap()),
        );
        let total = 600 * 1024 * 1024;
        for chunk in 1..=40_000u64 {
            context.download_progress((chunk * total / 40_000).min(total), Some(total));
        }
        // First chunk and the completed download; a 40 000-chunk loop finishes well under 250 ms.
        let emitted = events.lock().len();
        assert!((2..=4).contains(&emitted), "{emitted} progress events");
    }

    #[test]
    fn operation_lifecycle_transition_table() {
        use OperationState::*;
        let all = [
            Planned,
            Preflight,
            Running,
            WaitingForUser,
            Cancelling,
            Succeeded,
            Failed,
            Cancelled,
            Interrupted,
        ];
        for terminal in [Succeeded, Failed, Cancelled, Interrupted] {
            assert!(terminal.is_terminal());
            assert!(all.iter().all(|next| !terminal.can_transition_to(*next)));
        }
        assert!(Planned.can_transition_to(Preflight));
        assert!(!Planned.can_transition_to(Running));
        assert!(Preflight.can_transition_to(Running));
        assert!(Running.can_transition_to(WaitingForUser));
        assert!(WaitingForUser.can_transition_to(Running));
        assert!(!WaitingForUser.can_transition_to(Succeeded));
        assert!(Running.can_transition_to(Cancelling));
        assert!(Cancelling.can_transition_to(Cancelled));
        assert!(!Cancelling.can_transition_to(Running));
        assert!(!Preflight.can_transition_to(Succeeded));
        for live in [Planned, Preflight, Running, WaitingForUser, Cancelling] {
            assert!(live.can_transition_to(Interrupted));
            assert!(live.can_transition_to(Failed));
        }
    }

    #[tokio::test]
    async fn restoring_a_trashed_manual_prefix_registers_it_again() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::isolated(temp.path()).unwrap();
        let catalog_root = temp.path().join("catalog");
        std::fs::create_dir(&catalog_root).unwrap();
        write_recipe(
            &catalog_root,
            "noop",
            "maturity = \"native\"\n[[steps]]\ntype = \"native_action\"\naction = \"noop\"",
            "test",
        );
        let catalog = load_catalog(catalog_root, &paths, "test");
        let store = Arc::new(Store::open(&paths).unwrap());
        let mut engine = OperationEngine::new(catalog, paths, store.clone());
        engine.trash_program = fake_trash(temp.path());
        let prefix_path = temp.path().join("games/manual");
        std::fs::create_dir_all(prefix_path.join("drive_c")).unwrap();
        std::fs::write(prefix_path.join("system.reg"), "REGEDIT4\n").unwrap();
        let (wine, _) = fake_wine(temp.path());
        let mut prefix = test_prefix(prefix_path.clone(), Some(wine));
        prefix.name = "Manual".into();
        store
            .register_prefix(
                &prefix_path,
                "Manual",
                PrefixSource::Manual,
                prefix.runtime.as_deref(),
            )
            .unwrap();
        let point = engine.create_restore_point(&prefix).await.unwrap();
        engine.trash_prefix(&prefix).await.unwrap();
        assert!(!prefix_path.exists());
        assert!(temp.path().join("trash/manual/system.reg").is_file());
        assert!(store.registered_prefixes().unwrap().is_empty());

        engine.restore(&point, false).await.unwrap();

        assert!(prefix_path.join("system.reg").is_file());
        let registered = store.registered_prefixes().unwrap();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].0, prefix_path);
        assert_eq!(registered[0].3, prefix.runtime);
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
            status: Default::default(),
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
            status: Default::default(),
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
