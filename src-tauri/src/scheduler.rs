use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};

use crate::agent;
use crate::git;
use crate::opencode_server;
use crate::models::{
    BranchMode, CommandTemplates, ConflictMode, GitOp, Isolation, LogEvent, MergeSpec,
    MergeStrategy, PermissionProfile, Provider, ReviewMode, ReviewStatus, SystemPrompt, Task,
    TaskKind, TaskStatus,
};
use crate::state::AppState;
use tauri_plugin_notification::NotificationExt;

/// A task that is `running` but has shown no sign of life (live child, streamed
/// output, or a managed step) for this long is presumed orphaned.
const STALL_SECS: i64 = 600;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn emit_state(app: &AppHandle) {
    let snap = crate::state::snapshot(app);
    let _ = app.emit("state://changed", snap);
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

pub fn spawn_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(1000));
        loop {
            tick.tick().await;
            // Run each tick in its own task and observe the result, so a panic in
            // one tick is logged and the loop keeps going instead of silently
            // dying and freezing every running task forever.
            let a = app.clone();
            if let Err(e) = supervised_tick(move || async move { step(&a).await }).await {
                record_scheduler_error(&app, &e);
            }
        }
    });
}

/// Run one tick in its own task, turning a panic into an `Err` so the caller's
/// loop survives. Kept generic and free of Tauri types so it is unit-testable.
async fn supervised_tick<F, Fut>(tick_fn: F) -> Result<(), String>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    match tokio::spawn(tick_fn()).await {
        Ok(()) => Ok(()),
        Err(e) => Err(format!("scheduler tick failed: {e}")),
    }
}

/// Record a scheduler-level problem where the user can see it: appended to
/// `logs/scheduler.log` and surfaced as a notification.
fn record_scheduler_error(app: &AppHandle, message: &str) {
    eprintln!("[Solayge] {message}");
    let path = {
        let st = app.state::<AppState>();
        st.logs_dir().join("scheduler.log")
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "[{}] {message}", now());
    }
    notify(app, "Solayge", message);
}

/// Mark the given task interrupted and explain why in its log.
fn finish_interrupted(app: &AppHandle, id: &str, reason: &str) {
    let st = app.state::<AppState>();
    let mut title: Option<String> = None;
    {
        let mut inner = crate::state::lock(&st.inner);
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if t.status != TaskStatus::Running {
                return;
            }
            t.status = TaskStatus::Interrupted;
            t.error = Some(reason.to_string());
            t.finished_at = Some(
                last_activity(app, id)
                    .or(t.started_at)
                    .unwrap_or_else(now),
            );
            title = Some(t.title.clone());
        }
    }
    // Drop any half-registered child so it cannot be reaped as this task.
    if let Some(mut child) = crate::state::lock(&st.running).remove(id) {
        let _ = child.start_kill();
    }
    if let Some(mut child) = crate::state::lock(&st.merging).remove(id) {
        let _ = child.start_kill();
    }
    crate::state::lock(&st.heartbeat).remove(id);
    log_note(app, id, &format!("Interrupted: {reason}"));
    st.save();
    emit_state(app);
    if let Some(title) = title {
        notify(app, "Solayge needs you", &format!("{title} was interrupted"));
    }
}

/// Which running tasks and running reviews have gone silent. Pure so it can be
/// tested without an app handle.
#[derive(Debug, Default, PartialEq, Eq)]
struct StallSweep {
    tasks: Vec<String>,
    reviews: Vec<String>,
}

fn stall_sweep(
    tasks: &[Task],
    heartbeat: &HashMap<String, i64>,
    now: i64,
    stall_secs: i64,
) -> StallSweep {
    let mut out = StallSweep::default();
    for t in tasks {
        let last = heartbeat.get(&t.id).copied().or(t.started_at).unwrap_or(now);
        if now - last <= stall_secs {
            continue;
        }
        if t.status == TaskStatus::Running {
            out.tasks.push(t.id.clone());
        } else if t
            .review
            .as_ref()
            .is_some_and(|r| r.status == ReviewStatus::Running)
        {
            out.reviews.push(t.id.clone());
        }
    }
    out
}

/// For every `running` task (or running review) whose last sign of life is older
/// than [`STALL_SECS`], mark it interrupted so the UI can recover and retry.
fn reap_stalled(app: &AppHandle) {
    let st = app.state::<AppState>();
    let now = now();
    let sweep = {
        let inner = crate::state::lock(&st.inner);
        let beat = crate::state::lock(&st.heartbeat);
        stall_sweep(&inner.tasks, &beat, now, STALL_SECS)
    };
    for id in sweep.tasks {
        let reason = format!(
            "No activity for over {} minutes; the run appears to have stalled or lost its \
             process. Retry to run it again.",
            STALL_SECS / 60
        );
        finish_interrupted(app, &id, &reason);
    }
    for id in sweep.reviews {
        finish_review_failed(
            app,
            &id,
            format!(
                "The review stopped responding for over {} minutes and was abandoned.",
                STALL_SECS / 60
            ),
        );
    }
}

/// Claim every succeeded task whose review is queued, returning their ids and
/// flipping the review to running. Pure so the selection can be tested.
fn reviews_to_start(tasks: &mut [Task]) -> Vec<String> {
    let mut out = Vec::new();
    for t in tasks {
        if t.status != TaskStatus::Succeeded {
            continue;
        }
        if let Some(r) = t.review.as_mut() {
            if r.mode != ReviewMode::Off && r.status == ReviewStatus::Pending {
                r.status = ReviewStatus::Running;
                out.push(t.id.clone());
            }
        }
    }
    out
}

/// Start (or restart) reviews left queued by a previous tick or session. This is
/// the single path that launches a review, so there is no race at success time.
fn resume_reviews(app: &AppHandle) {
    let st = app.state::<AppState>();
    let to_start: Vec<String> = {
        let mut inner = crate::state::lock(&st.inner);
        reviews_to_start(&mut inner.tasks)
    };
    if to_start.is_empty() {
        return;
    }
    {
        let mut beat = crate::state::lock(&st.heartbeat);
        for id in &to_start {
            beat.insert(id.clone(), now());
        }
    }
    st.save();
    emit_state(app);
    for id in to_start {
        let a = app.clone();
        tauri::async_runtime::spawn(async move {
            start_review(a, id).await;
        });
    }
}

async fn step(app: &AppHandle) {
    reap(app);
    reap_reviews(app);
    resume_reviews(app);
    reap_stalled(app);
    let (started, mutated) = {
        let st = app.state::<AppState>();
        let out = dispatch(&st);
        if out.1 {
            st.save();
        }
        out
    };
    if mutated {
        emit_state(app);
    }
    // Keep the heartbeat map bounded: finished and deleted tasks must not leave
    // entries behind forever.
    prune_heartbeat(app);
    for id in started {
        let a = app.clone();
        tauri::async_runtime::spawn(async move {
            start_task(a, id).await;
        });
    }
}

/// Reap finished child processes. Returns true if any task was finalized.
fn reap(app: &AppHandle) -> bool {
    let st = app.state::<AppState>();
    let mut finished: Vec<(String, Option<i32>)> = Vec::new();
    let now = now();
    let mut alive: Vec<String> = Vec::new();
    {
        let mut running = crate::state::lock(&st.running);
        let ids: Vec<String> = running.keys().cloned().collect();
        for id in ids {
            let mut done = false;
            let mut code: Option<i32> = None;
            if let Some(child) = running.get_mut(&id) {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        done = true;
                        code = status.code();
                    }
                    // Still alive: that is a definitive liveness signal.
                    Ok(None) => alive.push(id.clone()),
                    Err(_) => {
                        done = true;
                    }
                }
            }
            if done {
                finished.push((id, code));
            }
        }
        for (id, _) in &finished {
            running.remove(id);
        }
    }
    if !alive.is_empty() {
        let mut beat = crate::state::lock(&st.heartbeat);
        for id in alive {
            beat.insert(id, now);
        }
    }
    if finished.is_empty() {
        return false;
    }
    for (id, code) in finished {
        finalize(app, &id, code);
    }
    true
}

fn finalize(app: &AppHandle, id: &str, code: Option<i32>) {
    let st = app.state::<AppState>();
    let mut outcome: Option<(String, bool)> = None;
    let mut retry_note: Option<(PathBuf, String)> = None;
    {
        let mut inner = crate::state::lock(&st.inner);
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if t.status == TaskStatus::Running {
                let ok = code == Some(0);
                if ok {
                    t.exit_code = code;
                    t.finished_at = Some(now());
                    t.status = TaskStatus::Succeeded;
                    // The review is queued here; `resume_reviews` picks it up on
                    // the next scheduler tick. Dependents stay held until the
                    // review reaches a terminal status (see `review_clear`).
                    if let Some(r) = t.review.as_mut() {
                        if r.mode != ReviewMode::Off {
                            r.status = ReviewStatus::Pending;
                            r.summary = None;
                            r.started_at = None;
                            r.finished_at = None;
                        }
                    }
                    outcome = Some((t.title.clone(), true));
                } else if !t.used_fallback && t.fallback_provider.is_some() {
                    // Retry once with the backup provider/model.
                    t.used_fallback = true;
                    let backup = t.fallback_provider;
                    t.provider = backup;
                    t.model = t.fallback_model.take();
                    t.status = TaskStatus::Ready;
                    t.started_at = None;
                    t.finished_at = None;
                    t.exit_code = None;
                    t.error = None;
                    let why = code
                        .map(|c| format!("exit {c}"))
                        .unwrap_or_else(|| "killed by signal".into());
                    retry_note = Some((
                        st.log_path(id),
                        format!(
                            "attempt failed ({why}); retrying with backup provider {}",
                            backup
                                .map(|p| p.command_key().to_string())
                                .unwrap_or_default()
                        ),
                    ));
                } else {
                    t.exit_code = code;
                    t.finished_at = Some(now());
                    t.status = TaskStatus::Failed;
                    if t.error.is_none() {
                        t.error = Some(match code {
                            Some(c) => format!("exited with code {c}"),
                            None => "terminated by signal".to_string(),
                        });
                    }
                    outcome = Some((t.title.clone(), false));
                }
            }
        }
    }
    crate::state::lock(&st.heartbeat).remove(id);
    st.save();
    emit_state(app);

    if let Some((path, note)) = retry_note {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "\n[Solayge] {note}");
        }
    }

    if let Some((title, ok)) = outcome {
        let body = if ok {
            format!("{title} — finished")
        } else {
            format!("{title} — failed")
        };
        notify(app, "Solayge", &body);
    }
}

/// Whether a succeeded task has fully finished, including its auto review, so
/// its dependents may start. A pending or running review holds them back.
fn review_clear(t: &Task) -> bool {
    match t.review.as_ref() {
        Some(r) => !matches!(r.status, ReviewStatus::Pending | ReviewStatus::Running),
        None => true,
    }
}

/// Promote waiting tasks and dispatch ready ones. Returns (started ids, mutated).
fn dispatch(st: &AppState) -> (Vec<String>, bool) {
    let mut started = Vec::new();
    let mut mutated = false;
    let mut inner = crate::state::lock(&st.inner);
    let now = now();

    // Only a task that succeeded *and* cleared its review satisfies dependents.
    let succeeded: HashSet<String> = inner
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Succeeded && review_clear(t))
        .map(|t| t.id.clone())
        .collect();
    let failed: HashSet<String> = inner
        .tasks
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Failed
                    | TaskStatus::Canceled
                    | TaskStatus::Blocked
                    | TaskStatus::Interrupted
            )
        })
        .map(|t| t.id.clone())
        .collect();

    for i in 0..inner.tasks.len() {
        if inner.tasks[i].status != TaskStatus::Waiting {
            continue;
        }
        let deps = inner.tasks[i].depends_on.clone();
        let not_before = inner.tasks[i].not_before;
        if deps.iter().any(|d| failed.contains(d)) {
            inner.tasks[i].status = TaskStatus::Blocked;
            inner.tasks[i].error = Some("dependency failed or was canceled".into());
            mutated = true;
            continue;
        }
        let deps_ok = deps.iter().all(|d| succeeded.contains(d));
        let time_ok = not_before.is_none_or(|nb| now >= nb);
        if deps_ok && time_ok {
            inner.tasks[i].status = TaskStatus::Ready;
            mutated = true;
        }
    }

    let concurrency = inner.concurrency.max(1);
    let mut running_total = inner
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Running)
        .count();
    let mut shared_active: HashSet<String> = inner
        .tasks
        .iter()
        .filter(|t| {
            t.status == TaskStatus::Running
                && (t.isolation == Isolation::Shared
                    || matches!(t.kind, TaskKind::Git | TaskKind::Merge))
        })
        .map(|t| t.project_path.clone())
        .collect();

    let mut candidates: Vec<usize> = inner
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.status == TaskStatus::Ready)
        .map(|(i, _)| i)
        .collect();
    candidates.sort_by_key(|&i| inner.tasks[i].created_at);

    for i in candidates {
        if running_total >= concurrency {
            break;
        }
        let isolation = inner.tasks[i].isolation;
        let project = inner.tasks[i].project_path.clone();
        let serial = isolation == Isolation::Shared
            || matches!(inner.tasks[i].kind, TaskKind::Git | TaskKind::Merge);
        if serial && shared_active.contains(&project) {
            continue;
        }
        inner.tasks[i].status = TaskStatus::Running;
        inner.tasks[i].started_at = Some(now);
        inner.tasks[i].error = None;
        inner.tasks[i].exit_code = None;
        inner.tasks[i].last_permission = None;
        if serial {
            shared_active.insert(project);
        }
        running_total += 1;
        mutated = true;
        started.push(inner.tasks[i].id.clone());
    }

    if !started.is_empty() {
        let mut beat = crate::state::lock(&st.heartbeat);
        for id in &started {
            beat.insert(id.clone(), now);
        }
    }

    (started, mutated)
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

async fn prepare_worktree(
    project: &Path,
    id: &str,
    base_ref: Option<&str>,
) -> anyhow::Result<(PathBuf, String)> {
    let root = git::worktree_root(project);
    let wt = root.join(short_id(id));
    let branch = format!("devtools/{}", short_id(id));
    if let Some(base) = base_ref {
        if !git::valid_ref(base) {
            anyhow::bail!("invalid base ref: {base}");
        }
    }
    if wt.exists() {
        let _ = git::worktree_remove(project, &wt).await;
    }
    // Drop a stale branch from a previous attempt, if any.
    let _ = git::branch_delete(project, &branch).await;
    tokio::fs::create_dir_all(&root).await?;
    let base = base_ref.unwrap_or("HEAD");
    git::worktree_add(project, &wt, &branch, base).await?;
    Ok((wt, branch))
}

fn fail_task(app: &AppHandle, id: &str, msg: String) {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            t.status = TaskStatus::Failed;
            t.error = Some(msg);
            t.finished_at = Some(now());
        }
    }
    st.save();
    emit_state(app);
}

/// Everything a task needs at start, read once from state.
struct TaskRun {
    id: String,
    project: String,
    title: String,
    prompt: String,
    isolation: Isolation,
    profile: PermissionProfile,
    base_ref: Option<String>,
    provider: Provider,
    model: Option<String>,
    existing_wt: Option<String>,
    kind: TaskKind,
    git_op: Option<GitOp>,
    command: Option<String>,
    merge: Option<MergeSpec>,
    branch_mode: BranchMode,
    new_branch: Option<String>,
    /// The branch recorded on a previous run, if any (for reuse on retry).
    branch: Option<String>,
}

/// A project's run-time context: encrypted env vars and its system prompt.
struct ProjectContext {
    name: String,
    system_prompt: Option<SystemPrompt>,
    env: Vec<(String, String)>,
    conflict_mode: ConflictMode,
}

fn load_run(app: &AppHandle, id: &str) -> Option<TaskRun> {
    let st = app.state::<AppState>();
    let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    let t = inner.tasks.iter().find(|t| t.id == id)?;
    Some(TaskRun {
        id: t.id.clone(),
        project: t.project_path.clone(),
        title: t.title.clone(),
        prompt: t.prompt.clone(),
        isolation: t.isolation,
        profile: t.profile,
        base_ref: t.base_ref.clone(),
        provider: t.provider.unwrap_or(Provider::Opencode),
        model: t.model.clone(),
        existing_wt: t.worktree_path.clone(),
        kind: t.kind,
        git_op: t.git_op,
        command: t.command.clone(),
        merge: t.merge.clone(),
        branch_mode: t.branch_mode,
        new_branch: t.new_branch.clone(),
        branch: t.branch.clone(),
    })
}

fn project_context(app: &AppHandle, project: &str) -> ProjectContext {
    let st = app.state::<AppState>();
    let (name, system_prompt, keys, kind, conflict_mode) = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = inner.projects.iter().find(|p| p.path == project);
        (
            p.map(|p| p.name.clone()).unwrap_or_default(),
            p.and_then(|p| p.system_prompt.clone()),
            p.map(|p| p.env_vars.iter().map(|e| e.key.clone()).collect::<Vec<_>>())
                .unwrap_or_default(),
            crate::secrets::StoreKind::parse(inner.settings.secret_store.as_deref()),
            p.and_then(|p| p.conflict_mode).unwrap_or_default(),
        )
    };
    let env = crate::secrets::Secrets::new(&st.data_dir, kind)
        .get_many(project, &keys)
        .unwrap_or_default();
    ProjectContext {
        name,
        system_prompt,
        env,
        conflict_mode,
    }
}

fn command_templates(app: &AppHandle) -> CommandTemplates {
    let st = app.state::<AppState>();
    st.inner
        .lock()
        .map(|i| i.settings.command_templates.clone())
        .unwrap_or_default()
}

/// Prepare a log file for a new attempt without destroying the previous one. If
/// it already has content, a separator is appended so the earlier attempt (often
/// the only evidence of what failed) is kept.
async fn reset_log(path: &Path) {
    let existing = tokio::fs::metadata(path).await.map(|m| m.len()).unwrap_or(0);
    if existing > 0 {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path).await {
            let sep = format!("\n[Solayge] ---- new attempt (unix {}) ----\n", now());
            let _ = f.write_all(sep.as_bytes()).await;
        }
    } else {
        let _ = tokio::fs::write(path, b"").await;
    }
}

async fn init_log(app: &AppHandle, id: &str) {
    let (log_path, logs_dir) = {
        let st = app.state::<AppState>();
        (st.log_path(id), st.logs_dir())
    };
    let _ = tokio::fs::create_dir_all(&logs_dir).await;
    reset_log(&log_path).await;
}

/// Append a `[Solayge]` line to the task log and stream it to the UI.
fn log_note(app: &AppHandle, id: &str, message: &str) {
    use std::io::Write;
    let line = format!("[Solayge] {message}");
    let st = app.state::<AppState>();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(st.log_path(id))
    {
        let _ = writeln!(f, "{line}");
    }
    let _ = app.emit(
        "task://log",
        LogEvent {
            task_id: id.to_string(),
            stream: "solayge".to_string(),
            line,
        },
    );
}

/// When this task's log was last written, as unix seconds. Best-effort.
fn last_activity(app: &AppHandle, id: &str) -> Option<i64> {
    let st = app.state::<AppState>();
    let modified = std::fs::metadata(st.log_path(id)).and_then(|m| m.modified()).ok()?;
    modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Record that a task is still making progress, so the stall watchdog leaves it
/// alone.
fn touch(app: &AppHandle, id: &str) {
    let st = app.state::<AppState>();
    crate::state::lock(&st.heartbeat).insert(id.to_string(), now());
}

/// Ids that still need a heartbeat entry: a running task, or a task whose review
/// is running. Everything else (finished, blocked, deleted) can be dropped.
fn active_heartbeat_ids(tasks: &[Task]) -> HashSet<String> {
    tasks
        .iter()
        .filter(|t| {
            t.status == TaskStatus::Running
                || t.review
                    .as_ref()
                    .is_some_and(|r| r.status == ReviewStatus::Running)
        })
        .map(|t| t.id.clone())
        .collect()
}

/// Drop heartbeat entries for tasks that are no longer running, so the map does
/// not grow with every task ever run (and survives task deletion).
fn prune_heartbeat(app: &AppHandle) {
    let st = app.state::<AppState>();
    let active = {
        let inner = crate::state::lock(&st.inner);
        active_heartbeat_ids(&inner.tasks)
    };
    let mut beat = crate::state::lock(&st.heartbeat);
    beat.retain(|id, _| active.contains(id));
}

/// Spawn a child, stream its output to the task log, and register it so it can
/// be cancelled. Finalization is left to `reap`.
fn spawn_simple(app: &AppHandle, id: &str, mut cmd: tokio::process::Command) {
    let program = cmd
        .as_std()
        .get_program()
        .to_string_lossy()
        .to_string();
    let cwd = cmd
        .as_std()
        .get_current_dir()
        .map(|p| p.to_string_lossy().to_string());
    match cmd.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let st = app.state::<AppState>();
                st.running
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.to_string(), child);
            }
            if let Some(out) = stdout {
                spawn_reader(app.clone(), id.to_string(), out, "stdout");
            }
            if let Some(err) = stderr {
                spawn_reader(app.clone(), id.to_string(), err, "stderr");
            }
            emit_state(app);
        }
        Err(e) => fail_task(
            app,
            id,
            format!("failed to launch \"{program}\"{cwd}: {e}", cwd = cwd.map(|c| format!(" in {c}")).unwrap_or_default()),
        ),
    }
}

/// Reuse a task's worktree, or create one.
async fn prepare_or_reuse(
    app: &AppHandle,
    run: &TaskRun,
    project_path: &Path,
) -> Result<PathBuf, String> {
    if let Some(wt) = run.existing_wt.as_ref().filter(|w| Path::new(w).exists()) {
        return Ok(PathBuf::from(wt));
    }
    touch(app, &run.id);
    match prepare_worktree(project_path, &run.id, run.base_ref.as_deref()).await {
        Ok((wt, branch)) => {
            {
                let st = app.state::<AppState>();
                let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == run.id) {
                    t.worktree_path = Some(wt.to_string_lossy().to_string());
                    t.branch = Some(branch);
                }
            }
            Ok(wt)
        }
        Err(e) => Err(format!("failed to create git worktree: {e}")),
    }
}

/// Which branch to use for a task that creates one.
enum BranchChoice {
    /// Use this exact name. `reused` marks a name recorded on a previous
    /// attempt, which is allowed to already exist.
    Named { name: String, reused: bool },
    /// No name was known; ask the agent for one.
    AskAgent,
}

/// Decide the branch name source: an explicit request, a name recorded on a
/// previous attempt, or the agent. Pure, so it can be unit-tested.
fn choose_branch(requested: &str, reuse: Option<&str>) -> BranchChoice {
    if !requested.is_empty() {
        let name = agent::sanitize_branch_name(requested);
        let reused = reuse == Some(name.as_str());
        return BranchChoice::Named { name, reused };
    }
    if let Some(branch) = reuse.filter(|b| !b.trim().is_empty()) {
        return BranchChoice::Named {
            name: branch.to_string(),
            reused: true,
        };
    }
    BranchChoice::AskAgent
}

/// Whether the chosen branch exists right now, and whether that is an error.
///
/// A name recorded on a previous attempt is allowed to *not* exist anymore (for
/// example the branch was deleted by "remove worktree"): it is reported as
/// missing so the caller recreates it instead of failing the retry.
async fn resolve_named_branch(
    project_path: &Path,
    name: &str,
    reused: bool,
) -> Result<(String, bool), String> {
    let exists = git::branch_exists(project_path, name).await;
    if exists && !reused {
        return Err(format!("branch \"{name}\" already exists"));
    }
    Ok((name.to_string(), exists))
}

/// Create (or reuse) the task's new branch, returning its name. A blank
/// requested name is chosen by the agent and de-duplicated; a branch recorded on
/// a previous attempt is reused if it still exists, and otherwise recreated, so
/// retries don't pile up new branches or fail on a deleted one.
async fn ensure_new_branch(
    app: &AppHandle,
    run: &TaskRun,
    project_path: &Path,
) -> Result<String, String> {
    touch(app, &run.id);
    let requested = run.new_branch.as_deref().map(str::trim).unwrap_or("");
    let reuse = run.branch.clone().filter(|b| !b.trim().is_empty());

    let (name, exists) = match choose_branch(requested, reuse.as_deref()) {
        BranchChoice::Named { name, reused } => {
            resolve_named_branch(project_path, &name, reused).await?
        }
        BranchChoice::AskAgent => {
            let suggested = {
                let mut cmd = agent::build_command(
                    run.provider,
                    run.model.as_deref(),
                    &command_templates(app),
                    &agent::branch_name_prompt(&run.title),
                    PermissionProfile::Readonly,
                    project_path,
                )?;
                agent::apply_env(&mut cmd, &project_context(app, &run.project).env);
                match tokio::time::timeout(Duration::from_secs(45), cmd.output()).await {
                    Ok(Ok(out)) => {
                        agent::sanitize_branch_name(&String::from_utf8_lossy(&out.stdout))
                    }
                    // Fall back to a slug of the title if the agent is unavailable.
                    _ => agent::sanitize_branch_name(&run.title),
                }
            };
            let mut name = suggested.clone();
            let mut n = 1;
            while git::branch_exists(project_path, &name).await {
                n += 1;
                name = format!("{suggested}-{n}");
            }
            (name, false)
        }
    };

    // Check out the branch, creating it if it doesn't exist yet.
    let output = if exists {
        git::git_cmd(project_path, &["checkout", name.as_str()])
            .output()
            .await
    } else {
        let mut args = vec!["checkout", "-b", name.as_str()];
        if let Some(base) = run.base_ref.as_deref().filter(|b| !b.trim().is_empty()) {
            if !git::valid_ref(base) {
                return Err(format!("invalid base ref: {base}"));
            }
            args.push(base);
        }
        git::git_cmd(project_path, &args).output().await
    }
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "could not use branch {name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    {
        let st = app.state::<AppState>();
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == run.id) {
            t.branch = Some(name.clone());
            // Remember the chosen name so a retry reuses the same branch.
            t.new_branch = Some(name.clone());
        }
    }
    log_note(app, &run.id, &format!("On new branch {name}"));
    Ok(name)
}

async fn start_task(app: AppHandle, id: String) {
    let Some(run) = load_run(&app, &id) else {
        return;
    };
    touch(&app, &id);
    // opencode agent tasks run through the server so the agent can ask questions
    // and request permission. If the server cannot be reached we fall back to the
    // non-interactive CLI rather than failing the task.
    if run.kind == TaskKind::Agent && run.provider == Provider::Opencode {
        match opencode_setup(&app, &run).await {
            Ok((conn, session)) => {
                start_opencode_loop(app, run, conn, session).await;
                return;
            }
            Err(e) => {
                log_note(
                    &app,
                    &id,
                    &format!("interactive mode unavailable ({e}); running non-interactively"),
                );
            }
        }
    }
    match run.kind {
        TaskKind::Agent => start_agent_task(app, run).await,
        TaskKind::Shell => start_shell_task(app, run).await,
        TaskKind::Git => start_git_task(app, run).await,
        TaskKind::Merge => start_merge_task(app, run).await,
    }
}

async fn start_agent_task(app: AppHandle, run: TaskRun) {
    let project_path = PathBuf::from(&run.project);
    let mut cwd = project_path.clone();
    if run.isolation == Isolation::Worktree {
        match prepare_or_reuse(&app, &run, &project_path).await {
            Ok(wt) => cwd = wt,
            Err(e) => {
                fail_task(&app, &run.id, e);
                return;
            }
        }
    }
    init_log(&app, &run.id).await;

    // In the project directory, optionally start a new branch first.
    if let Err(e) = maybe_new_branch(&app, &run, &project_path).await {
        fail_task(&app, &run.id, e);
        return;
    }

    let ctx = project_context(&app, &run.project);
    let branch = git::current_branch(&project_path).await;
    let pctx = agent::PromptContext {
        project_name: &ctx.name,
        project_path: &run.project,
        current_branch: branch.as_deref(),
        env: &ctx.env,
    };
    let prompt = agent::apply_system_prompt(&run.prompt, ctx.system_prompt.as_ref(), &pctx);

    let mut cmd = match agent::build_command(
        run.provider,
        run.model.as_deref(),
        &command_templates(&app),
        &prompt,
        run.profile,
        &cwd,
    ) {
        Ok(c) => c,
        Err(e) => {
            fail_task(&app, &run.id, e);
            return;
        }
    };
    agent::apply_env(&mut cmd, &ctx.env);
    spawn_simple(&app, &run.id, cmd);
}

/// Prepare an opencode agent task: worktree/branch, prompt, server session, and
/// the initial prompt. Returns the connection and session id. Any error here is
/// safe to fall back from (nothing has been committed to yet).
async fn opencode_setup(
    app: &AppHandle,
    run: &TaskRun,
) -> Result<(opencode_server::Connection, String), String> {
    let project_path = PathBuf::from(&run.project);
    let mut cwd = project_path.clone();
    if run.isolation == Isolation::Worktree {
        cwd = prepare_or_reuse(app, run, &project_path).await?;
    }
    init_log(app, &run.id).await;
    maybe_new_branch(app, run, &project_path).await?;

    let ctx = project_context(app, &run.project);
    let branch = git::current_branch(&project_path).await;
    let pctx = agent::PromptContext {
        project_name: &ctx.name,
        project_path: &run.project,
        current_branch: branch.as_deref(),
        env: &ctx.env,
    };
    let prompt = agent::apply_system_prompt(&run.prompt, ctx.system_prompt.as_ref(), &pctx);

    let conn = opencode_server::ensure_server().await?;
    let session = opencode_server::create_session(
        &conn,
        &cwd.to_string_lossy(),
        &run.title,
        run.model.as_deref(),
    )
    .await?;
    log_note(app, &run.id, &format!("opencode session {session}"));
    opencode_server::prompt(&conn, &session, &prompt).await?;
    Ok((conn, session))
}

/// Poll an opencode session: stream output to the task log, surface questions
/// and permission requests, and finalize when the session goes idle.
async fn start_opencode_loop(
    app: AppHandle,
    run: TaskRun,
    conn: opencode_server::Connection,
    session: String,
) {
    use std::collections::HashSet;
    let mut logged: HashSet<String> = HashSet::new();
    let mut surfaced: Option<String> = None;

    loop {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        if opencode_stopped(&app, &run.id) {
            let _ = opencode_server::interrupt(&conn, &session).await;
            return;
        }
        touch(&app, &run.id);

        let msgs = match opencode_server::messages(&conn, &session).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Log each completed assistant message once.
        for m in &msgs {
            if m.get("type").and_then(|v| v.as_str()) != Some("assistant") {
                continue;
            }
            if m.get("time").and_then(|t| t.get("completed")).is_none() {
                continue;
            }
            let id = m.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            if id.is_empty() || !logged.insert(id.to_string()) {
                continue;
            }
            if let Some(line) = opencode_server::part_line(m) {
                log_note(&app, &run.id, &line);
            }
        }

        // If a surfaced ask was answered, allow the next one to surface.
        if surfaced.is_some() && !task_has_ask(&app, &run.id) {
            surfaced = None;
        }

        // A question (form) or a supervised permission request pauses for input.
        if surfaced.is_none() {
            if let Ok(forms) = opencode_server::forms(&conn, &session).await {
                if let Some(ask) = forms
                    .first()
                    .and_then(|f| opencode_server::form_to_ask(&session, f))
                {
                    surfaced = Some(ask.id.clone());
                    set_task_ask(&app, &run.id, ask);
                }
            }
        }
        if surfaced.is_none() {
            if let Ok(perms) = opencode_server::permissions(&conn, &session).await {
                if let Some(ask) =
                    perms.first().and_then(|p| opencode_server::permission_to_ask(&session, p))
                {
                    match run.profile {
                        PermissionProfile::Autonomous => {
                            let _ = opencode_server::reply_permission(
                                &conn, &session, &ask.id, "always", None,
                            )
                            .await;
                        }
                        PermissionProfile::Readonly => {
                            let _ = opencode_server::reply_permission(
                                &conn, &session, &ask.id, "reject", None,
                            )
                            .await;
                        }
                        PermissionProfile::Supervised => {
                            surfaced = Some(ask.id.clone());
                            set_task_ask(&app, &run.id, ask);
                        }
                    }
                }
            }
        }

        if let Some(outcome) = opencode_server::session_outcome(&msgs) {
            clear_task_ask(&app, &run.id);
            let ok = outcome == "succeeded";
            if ok {
                // Queue the auto review, matching the CLI path's `finalize`.
                queue_review(&app, &run.id);
            }
            finish_managed(
                &app,
                &run.id,
                ok,
                if ok {
                    None
                } else {
                    Some(format!("the agent session {outcome}"))
                },
            );
            return;
        }
    }
}

/// Set a succeeded task's review to `pending` so `resume_reviews` runs it.
fn queue_review(app: &AppHandle, id: &str) {
    let st = app.state::<AppState>();
    let mut inner = crate::state::lock(&st.inner);
    if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
        if let Some(r) = t.review.as_mut() {
            if r.mode != ReviewMode::Off {
                r.status = ReviewStatus::Pending;
                r.summary = None;
                r.started_at = None;
                r.finished_at = None;
            }
        }
    }
}

/// True when the task was cancelled (blocked-for-input does not stop the loop).
fn opencode_stopped(app: &AppHandle, id: &str) -> bool {
    let st = app.state::<AppState>();
    let inner = crate::state::lock(&st.inner);
    inner
        .tasks
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.status == TaskStatus::Canceled)
        .unwrap_or(true)
}

/// Whether the task currently has an unanswered ask.
fn task_has_ask(app: &AppHandle, id: &str) -> bool {
    let st = app.state::<AppState>();
    let inner = crate::state::lock(&st.inner);
    inner
        .tasks
        .iter()
        .find(|t| t.id == id)
        .is_some_and(|t| t.ask.is_some())
}

/// Attach a pending question/permission to a task and notify the user. The task
/// stays `running`, so dependents wait without becoming permanently blocked.
fn set_task_ask(app: &AppHandle, id: &str, ask: crate::models::TaskAsk) {
    let st = app.state::<AppState>();
    let title = ask.title.clone();
    {
        let mut inner = crate::state::lock(&st.inner);
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            t.ask = Some(ask);
        }
    }
    log_note(app, id, &format!("Waiting for you: {title}"));
    st.save();
    emit_state(app);
    notify(app, "Solayge needs you", &title);
}

fn clear_task_ask(app: &AppHandle, id: &str) {
    let st = app.state::<AppState>();
    let changed = {
        let mut inner = crate::state::lock(&st.inner);
        match inner.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) if t.ask.is_some() => {
                t.ask = None;
                true
            }
            _ => false,
        }
    };
    if changed {
        st.save();
        emit_state(app);
    }
}

/// Create a new branch before the task runs, when the task asked for one and is
/// not isolated in a worktree.
async fn maybe_new_branch(
    app: &AppHandle,
    run: &TaskRun,
    project_path: &Path,
) -> Result<(), String> {
    if run.isolation != Isolation::Worktree && run.branch_mode == BranchMode::New {
        ensure_new_branch(app, run, project_path).await?;
    }
    Ok(())
}

/// Run a built-in git operation as a managed task. Uses direct commands (no
/// shell) so the behaviour is identical on macOS, Linux, and Windows.
async fn start_git_task(app: AppHandle, run: TaskRun) {
    let project_path = PathBuf::from(&run.project);
    init_log(&app, &run.id).await;
    if let Err(e) = maybe_new_branch(&app, &run, &project_path).await {
        fail_task(&app, &run.id, e);
        return;
    }
    let env = project_context(&app, &run.project).env;
    match execute_git(&app, &run, &env).await {
        Ok(()) => finish_managed(&app, &run.id, true, None),
        Err(reason) => {
            log_note(&app, &run.id, &format!("git step failed: {reason}"));
            finish_managed(&app, &run.id, false, Some(reason));
        }
    }
}

async fn execute_git(
    app: &AppHandle,
    run: &TaskRun,
    env: &[(String, String)],
) -> Result<(), String> {
    let dir = Path::new(&run.project);
    let op = run.git_op.ok_or_else(|| "git task has no operation".to_string())?;
    let arg = run.command.clone().unwrap_or_default();
    let check = |code: i32, what: &str| -> Result<(), String> {
        if code == 0 {
            Ok(())
        } else {
            Err(format!("{what} failed"))
        }
    };
    let cancelled = || Err("cancelled".to_string());

    match op {
        GitOp::AddCommit => {
            let message = if arg.trim().is_empty() {
                "chore: update".to_string()
            } else {
                arg.clone()
            };
            if git::has_changes(dir).await {
                run_git(app, run, dir, &["add", "-A"], env).await;
                if is_cancelled(app, &run.id) {
                    return cancelled();
                }
                let code = run_git(app, run, dir, &["commit", "-m", message.as_str()], env).await;
                check(code, "git commit")?;
            } else {
                log_note(app, &run.id, "Nothing to commit.");
            }
        }
        GitOp::Push => {
            let code = run_git(app, run, dir, &["push", "-u", "origin", "HEAD"], env).await;
            check(code, "git push")?;
        }
        GitOp::Pull => {
            let code = run_git(app, run, dir, &["pull"], env).await;
            check(code, "git pull")?;
        }
        GitOp::Checkout => {
            let branch = if arg.trim().is_empty() {
                git::default_branch(dir).await
            } else {
                arg.clone()
            };
            if !git::valid_ref(&branch) {
                return Err(format!("invalid branch name: {branch}"));
            }
            let code = run_git(app, run, dir, &["checkout", branch.as_str()], env).await;
            check(code, "git checkout")?;
        }
        GitOp::PrCreate => {
            let repo = agent::normalize_remote(&git::remote_url(dir).await.unwrap_or_default());
            if repo.is_empty() {
                return Err("no git remote to create a PR against".into());
            }
            let base = git::default_branch(dir).await;
            let title = if arg.trim().is_empty() {
                format!("Ship: {}", run.title)
            } else {
                arg.clone()
            };
            let body = format!("Automated by Solayge from task \"{}\".", run.title);
            if git::gh_available().await {
                let args = [
                    "pr",
                    "create",
                    "--base",
                    base.as_str(),
                    "--title",
                    title.as_str(),
                    "--body",
                    body.as_str(),
                ];
                let mut cmd = git::gh_cmd(dir, &args);
                agent::apply_env(&mut cmd, env);
                if run_streamed(app, &run.id, cmd).await != Some(0) {
                    return Err("gh pr create failed".into());
                }
            } else if agent::is_web_url(&repo) {
                let url = format!("{repo}/compare/{base}...HEAD");
                log_note(app, &run.id, &format!("gh not found; opening {url}"));
                let mut cmd = agent::open_url_command(&url);
                agent::apply_env(&mut cmd, env);
                let _ = run_streamed(app, &run.id, cmd).await;
            } else {
                return Err("gh is not installed and the remote is not an http(s) URL".into());
            }
        }
        GitOp::PrMerge => {
            let repo = agent::normalize_remote(&git::remote_url(dir).await.unwrap_or_default());
            if repo.is_empty() {
                return Err("no git remote to merge a PR against".into());
            }
            let method = match arg.trim() {
                "merge" => "--merge",
                "rebase" => "--rebase",
                _ => "--squash",
            };
            if git::gh_available().await {
                let mut cmd = git::gh_cmd(dir, &["pr", "merge", method, "--delete-branch"]);
                agent::apply_env(&mut cmd, env);
                if run_streamed(app, &run.id, cmd).await != Some(0) {
                    return Err("gh pr merge failed".into());
                }
            } else if agent::is_web_url(&repo) {
                let url = format!("{repo}/pulls");
                log_note(app, &run.id, &format!("gh not found; opening {url}"));
                let mut cmd = agent::open_url_command(&url);
                agent::apply_env(&mut cmd, env);
                let _ = run_streamed(app, &run.id, cmd).await;
            } else {
                return Err("gh is not installed and the remote is not an http(s) URL".into());
            }
        }
    }
    Ok(())
}

/// Run a shell task: a user-provided command in the platform's shell.
async fn start_shell_task(app: AppHandle, run: TaskRun) {
    let project_path = PathBuf::from(&run.project);
    let mut cwd = project_path.clone();
    if run.isolation == Isolation::Worktree {
        match prepare_or_reuse(&app, &run, &project_path).await {
            Ok(wt) => cwd = wt,
            Err(e) => {
                fail_task(&app, &run.id, e);
                return;
            }
        }
    }
    init_log(&app, &run.id).await;
    if let Err(e) = maybe_new_branch(&app, &run, &project_path).await {
        fail_task(&app, &run.id, e);
        return;
    }

    let script = run
        .command
        .clone()
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| run.prompt.clone());
    if script.trim().is_empty() {
        fail_task(&app, &run.id, "task has no command to run".into());
        return;
    }
    let ctx = project_context(&app, &run.project);
    log_note(&app, &run.id, &format!("$ {script}"));
    let mut cmd = git::shell_cmd(&cwd, &script);
    agent::apply_env(&mut cmd, &ctx.env);
    spawn_simple(&app, &run.id, cmd);
}

fn is_cancelled(app: &AppHandle, id: &str) -> bool {
    let st = app.state::<AppState>();
    let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    inner
        .tasks
        .iter()
        .find(|t| t.id == id)
        .map(|t| t.status != TaskStatus::Running)
        .unwrap_or(true)
}

/// Spawn a child for a managed (merge) task, stream it, and wait. Returns the
/// exit code, or `None` when the process could not start.
async fn run_streamed(app: &AppHandle, id: &str, mut cmd: tokio::process::Command) -> Option<i32> {
    let program = cmd.as_std().get_program().to_string_lossy().to_string();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log_note(app, id, &format!("failed to start \"{program}\": {e}"));
            return None;
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Some(out) = stdout {
        spawn_reader(app.clone(), id.to_string(), out, "stdout");
    }
    if let Some(err) = stderr {
        spawn_reader(app.clone(), id.to_string(), err, "stderr");
    }
    {
        let st = app.state::<AppState>();
        st.merging
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), child);
    }
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let st = app.state::<AppState>();
        // A managed step is making progress while its child runs.
        crate::state::lock(&st.heartbeat).insert(id.to_string(), now());
        let mut map = crate::state::lock(&st.merging);
        match map.get_mut(id) {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.code().unwrap_or(-1);
                    map.remove(id);
                    return Some(code);
                }
                Ok(None) => {}
                Err(_) => {
                    map.remove(id);
                    return Some(-1);
                }
            },
            None => return None,
        }
    }
}

async fn run_git(app: &AppHandle, run: &TaskRun, dir: &Path, args: &[&str], env: &[(String, String)]) -> i32 {
    let mut cmd = git::git_cmd(dir, args);
    agent::apply_env(&mut cmd, env);
    run_streamed(app, &run.id, cmd).await.unwrap_or(-1)
}

async fn run_shell(app: &AppHandle, run: &TaskRun, dir: &Path, script: &str, env: &[(String, String)]) -> i32 {
    let mut cmd = git::shell_cmd(dir, script);
    agent::apply_env(&mut cmd, env);
    run_streamed(app, &run.id, cmd).await.unwrap_or(-1)
}

/// Run an agent (for conflict resolution / fixes) inside a managed task.
async fn run_agent_step(
    app: &AppHandle,
    run: &TaskRun,
    dir: &Path,
    prompt: &str,
    env: &[(String, String)],
) -> i32 {
    let mut cmd = match agent::build_command(
        run.provider,
        run.model.as_deref(),
        &command_templates(app),
        prompt,
        PermissionProfile::Autonomous,
        dir,
    ) {
        Ok(c) => c,
        Err(e) => {
            log_note(app, &run.id, &e);
            return -1;
        }
    };
    agent::apply_env(&mut cmd, env);
    run_streamed(app, &run.id, cmd).await.unwrap_or(-1)
}

fn resolve_sources(app: &AppHandle, sources: &[String]) -> Vec<String> {
    let st = app.state::<AppState>();
    let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    sources
        .iter()
        .filter_map(|s| {
            if let Some(t) = inner.tasks.iter().find(|t| t.id == *s) {
                t.branch.clone()
            } else if !s.trim().is_empty() {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect()
}

/// A managed-step failure: pause for the user (`Attention`) or hard-fail.
enum StepError {
    Attention(String),
    Failed(String),
    Cancelled,
}

impl From<String> for StepError {
    fn from(message: String) -> Self {
        StepError::Failed(message)
    }
}

impl From<&str> for StepError {
    fn from(message: &str) -> Self {
        StepError::Failed(message.to_string())
    }
}

fn ensure_running(app: &AppHandle, id: &str) -> Result<(), StepError> {
    if is_cancelled(app, id) {
        Err(StepError::Cancelled)
    } else {
        Ok(())
    }
}

async fn resolve_conflicts(
    app: &AppHandle,
    run: &TaskRun,
    dir: &Path,
    sources: &[String],
    env: &[(String, String)],
) -> Result<(), StepError> {
    log_note(
        app,
        &run.id,
        &format!("Conflicts merging {} - handing them to the agent.", sources.join(", ")),
    );
    let list = git::conflicted_files(dir).await;
    let prompt = format!(
        "You are resolving merge conflicts in the git repository at {}. Combine these branches: {}.\n\n\
         Conflicted files:\n{}\n\n\
         Resolve every conflict marker (<<<<<<<, =======, >>>>>>>) by keeping the intent of both sides. \
         Do NOT run `git commit` or `git merge --continue`; just edit the files so they are correct and \
         conflict-free. When done, make sure no conflict markers remain.",
        dir.display(),
        sources.join(", "),
        if list.is_empty() {
            "(see `git status`)".to_string()
        } else {
            list.join("\n")
        }
    );
    if run_agent_step(app, run, dir, &prompt, env).await != 0 {
        return Err(StepError::Failed(
            "the agent could not resolve the conflicts".into(),
        ));
    }
    let remaining = git::conflicted_files(dir).await;
    if !remaining.is_empty() {
        return Err(StepError::Failed(format!(
            "conflict markers remain in: {}",
            remaining.join(", ")
        )));
    }
    // Record the resolution. A merge or rebase is in progress; tolerate an agent
    // that already committed it.
    run_git(app, run, dir, &["add", "-A"], env).await;
    let rebasing = dir.join(".git").join("rebase-merge").exists()
        || dir.join(".git").join("rebase-apply").exists();
    if rebasing {
        let mut cmd = git::git_cmd(dir, &["rebase", "--continue"]);
        cmd.env("GIT_EDITOR", "true");
        agent::apply_env(&mut cmd, env);
        if run_streamed(app, &run.id, cmd).await != Some(0) {
            return Err(StepError::Failed("failed to continue the rebase".into()));
        }
    } else if git::has_staged_changes(dir).await {
        if run_git(app, run, dir, &["commit", "--no-edit"], env).await != 0 {
            return Err(StepError::Failed(
                "failed to commit the conflict resolution".into(),
            ));
        }
    } else {
        log_note(app, &run.id, "Resolution already committed.");
    }
    Ok(())
}

/// Apply a project's conflict policy when merging `source` fails.
async fn handle_conflict(
    app: &AppHandle,
    run: &TaskRun,
    dir: &Path,
    source: &str,
    env: &[(String, String)],
    mode: ConflictMode,
) -> Result<(), StepError> {
    match mode {
        ConflictMode::User => Err(StepError::Attention(format!(
            "merge conflict while combining {source}. Resolve it in {}, then retry - or choose an \
             automatic conflict mode in this project's settings.",
            dir.display()
        ))),
        ConflictMode::AgentReview | ConflictMode::AgentAuto => {
            resolve_conflicts(app, run, dir, &[source.to_string()], env).await?;
            if mode == ConflictMode::AgentReview {
                Err(StepError::Attention(format!(
                    "the agent resolved the conflict from {source}. Review the changes, then retry to \
                     continue the workflow."
                )))
            } else {
                Ok(())
            }
        }
    }
}

async fn run_merge_steps(
    app: &AppHandle,
    run: &TaskRun,
    dir: &Path,
    spec: &MergeSpec,
    ctx: &ProjectContext,
) -> Result<(), StepError> {
    let env = &ctx.env;
    let target = match spec.target.as_ref().filter(|t| !t.trim().is_empty()) {
        Some(t) => t.clone(),
        None => git::default_branch(dir).await,
    };
    if git::has_changes(dir).await {
        return Err("the working tree has uncommitted changes; commit or stash them first".into());
    }
    let sources = resolve_sources(app, &spec.sources);
    if sources.is_empty() {
        return Err("no source branches to combine".into());
    }
    if !git::valid_ref(&target) {
        return Err(format!("invalid target branch: {target}").into());
    }
    for source in &sources {
        if !git::valid_ref(source) {
            return Err(format!("invalid source branch: {source}").into());
        }
    }
    log_note(
        app,
        &run.id,
        &format!("Integrating {} into {target}", sources.join(", ")),
    );

    if run_git(app, run, dir, &["checkout", &target], env).await != 0 {
        return Err(format!("could not check out {target}").into());
    }
    ensure_running(app, &run.id)?;

    match spec.strategy {
        MergeStrategy::Octopus => {
            let mut args = vec!["merge", "--no-edit"];
            for s in &sources {
                args.push(s);
            }
            if run_git(app, run, dir, &args, env).await != 0 {
                let label = sources.join(", ");
                handle_conflict(app, run, dir, &label, env, ctx.conflict_mode).await?;
            }
        }
        MergeStrategy::Rebase => {
            for source in &sources {
                ensure_running(app, &run.id)?;
                if run_git(app, run, dir, &["rebase", source], env).await != 0 {
                    handle_conflict(app, run, dir, source, env, ctx.conflict_mode).await?;
                }
            }
        }
        MergeStrategy::Merge => {
            for source in &sources {
                ensure_running(app, &run.id)?;
                if run_git(app, run, dir, &["merge", "--no-edit", source], env).await != 0 {
                    handle_conflict(app, run, dir, source, env, ctx.conflict_mode).await?;
                }
            }
        }
    }

    ensure_running(app, &run.id)?;
    if let Some(test) = spec.test_command.as_ref().filter(|t| !t.trim().is_empty()) {
        log_note(app, &run.id, &format!("Running tests: {test}"));
        if run_shell(app, run, dir, test, env).await != 0 {
            if !spec.fix_on_failure {
                return Err("tests failed".into());
            }
            log_note(app, &run.id, "Tests failed - asking the agent to fix them.");
            let prompt = format!(
                "The test command `{test}` failed in the repository at {}. Inspect the output above, fix \
                 the underlying problem (do not weaken or delete the tests), and leave the tree in a \
                 committed-clean state.",
                dir.display()
            );
            if run_agent_step(app, run, dir, &prompt, env).await != 0 {
                return Err("the agent failed while fixing the tests".into());
            }
            if run_shell(app, run, dir, test, env).await != 0 {
                return Err("tests are still failing after the agent's fix".into());
            }
        }
    }

    if spec.push_target {
        ensure_running(app, &run.id)?;
        if run_git(app, run, dir, &["push", "-u", "origin", &target], env).await != 0 {
            return Err(format!("failed to push {target}").into());
        }
    }
    Ok(())
}

fn finish_managed(app: &AppHandle, id: &str, ok: bool, error: Option<String>) {
    let st = app.state::<AppState>();
    let mut title: Option<String> = None;
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if t.status != TaskStatus::Running {
                return; // cancelled meanwhile
            }
            t.exit_code = Some(if ok { 0 } else { 1 });
            t.finished_at = Some(now());
            if ok {
                t.status = TaskStatus::Succeeded;
            } else {
                t.status = TaskStatus::Failed;
                t.error = error.or_else(|| Some("integration failed".into()));
            }
            title = Some(t.title.clone());
        }
    }
    crate::state::lock(&st.heartbeat).remove(id);
    st.save();
    emit_state(app);
    if let Some(title) = title {
        let body = format!("{title} — {}", if ok { "finished" } else { "failed" });
        notify(app, "Solayge", &body);
    }
}

/// Pause a managed task for the user (a conflict or a review gate).
fn finish_blocked(app: &AppHandle, id: &str, reason: String) {
    let st = app.state::<AppState>();
    let mut title: Option<String> = None;
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if t.status != TaskStatus::Running {
                return;
            }
            t.status = TaskStatus::Blocked;
            t.error = Some(reason);
            t.finished_at = Some(now());
            title = Some(t.title.clone());
        }
    }
    crate::state::lock(&st.heartbeat).remove(id);
    st.save();
    emit_state(app);
    if let Some(title) = title {
        notify(app, "Solayge needs you", &format!("{title} is waiting for you"));
    }
}

async fn start_merge_task(app: AppHandle, run: TaskRun) {
    let dir = PathBuf::from(&run.project);
    init_log(&app, &run.id).await;
    let ctx = project_context(&app, &run.project);
    let spec = run.merge.clone().unwrap_or_default();
    if is_cancelled(&app, &run.id) {
        return;
    }
    match run_merge_steps(&app, &run, &dir, &spec, &ctx).await {
        Ok(()) => finish_managed(&app, &run.id, true, None),
        Err(StepError::Attention(reason)) => {
            git::merge_abort(&dir).await;
            log_note(&app, &run.id, &format!("Waiting for you: {reason}"));
            finish_blocked(&app, &run.id, reason);
        }
        Err(StepError::Failed(reason)) => {
            git::merge_abort(&dir).await;
            log_note(&app, &run.id, &format!("Integration failed: {reason}"));
            finish_managed(&app, &run.id, false, Some(reason));
        }
        Err(StepError::Cancelled) => {
            git::merge_abort(&dir).await;
        }
    }
}

/// Run the auto code review for a finished task.
async fn start_review(app: AppHandle, id: String) {
    let (cwd, title, task_prompt, mode, provider, model, project) = {
        let st = app.state::<AppState>();
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = inner.tasks.iter().find(|t| t.id == id) else {
            return;
        };
        let Some(r) = t.review.as_ref() else { return };
        if r.mode == ReviewMode::Off {
            return;
        }
        // The task may have been canceled after the review was queued.
        if t.status != TaskStatus::Succeeded {
            return;
        }
        let cwd = t
            .worktree_path
            .clone()
            .unwrap_or_else(|| t.project_path.clone());
        (
            PathBuf::from(cwd),
            t.title.clone(),
            t.prompt.clone(),
            r.mode,
            r.provider.unwrap_or(Provider::Opencode),
            r.model.clone(),
            t.project_path.clone(),
        )
    };

    let templates = {
        let st = app.state::<AppState>();
        st.inner
            .lock()
            .map(|i| i.settings.command_templates.clone())
            .unwrap_or_default()
    };
    let env = {
        let st = app.state::<AppState>();
        let (keys, kind) = {
            let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
            let proj = inner.projects.iter().find(|p| p.path == project);
            (
                proj.map(|p| p.env_vars.iter().map(|e| e.key.clone()).collect::<Vec<_>>())
                    .unwrap_or_default(),
                crate::secrets::StoreKind::parse(inner.settings.secret_store.as_deref()),
            )
        };
        crate::secrets::Secrets::new(&st.data_dir, kind)
            .get_many(&project, &keys)
            .unwrap_or_default()
    };
    let prompt = agent::review_prompt(&title, &task_prompt, mode);
    let (log_path, logs_dir) = {
        let st = app.state::<AppState>();
        (st.review_log_path(&id), st.logs_dir())
    };
    let _ = tokio::fs::create_dir_all(&logs_dir).await;
    reset_log(&log_path).await;

    {
        let st = app.state::<AppState>();
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if let Some(r) = t.review.as_mut() {
                r.status = ReviewStatus::Running;
                r.started_at = Some(now());
                r.finished_at = None;
                r.summary = None;
            }
        }
        crate::state::lock(&st.heartbeat).insert(id.clone(), now());
    }
    // Persist *after* releasing the state lock: `AppState::save` locks `inner`
    // again, and a `std::sync::Mutex` is not reentrant.
    {
        let st = app.state::<AppState>();
        st.save();
    }
    emit_state(&app);

    // Autonomous so the reviewer can run git/tests and (in Autofix) edit.
    let mut cmd = match agent::build_command(
        provider,
        model.as_deref(),
        &templates,
        &prompt,
        PermissionProfile::Autonomous,
        &cwd,
    ) {
        Ok(c) => c,
        Err(e) => {
            finish_review_failed(&app, &id, e);
            return;
        }
    };
    agent::apply_env(&mut cmd, &env);

    match cmd.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let st = app.state::<AppState>();
                st.reviewing
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone(), child);
            }
            if let Some(out) = stdout {
                spawn_review_reader(app.clone(), id.clone(), out);
            }
            if let Some(err) = stderr {
                spawn_review_reader(app.clone(), id.clone(), err);
            }
            emit_state(&app);
        }
        Err(e) => {
            finish_review_failed(
                &app,
                &id,
                format!("failed to launch reviewer {}: {e}", provider.command_key()),
            );
        }
    }
}

fn finish_review_failed(app: &AppHandle, id: &str, msg: String) {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if let Some(r) = t.review.as_mut() {
                r.status = ReviewStatus::Failed;
                r.summary = Some(msg);
                r.finished_at = Some(now());
            }
        }
    }
    crate::state::lock(&st.heartbeat).remove(id);
    st.save();
    emit_state(app);
}

fn reap_reviews(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut finished: Vec<(String, Option<i32>)> = Vec::new();
    let now = now();
    let mut alive: Vec<String> = Vec::new();
    {
        let mut reviewing = crate::state::lock(&st.reviewing);
        let ids: Vec<String> = reviewing.keys().cloned().collect();
        for id in ids {
            let mut done = false;
            let mut code: Option<i32> = None;
            if let Some(child) = reviewing.get_mut(&id) {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        done = true;
                        code = status.code();
                    }
                    Ok(None) => alive.push(id.clone()),
                    Err(_) => done = true,
                }
            }
            if done {
                finished.push((id, code));
            }
        }
        for (id, _) in &finished {
            reviewing.remove(id);
        }
    }
    if !alive.is_empty() {
        let mut beat = crate::state::lock(&st.heartbeat);
        for id in alive {
            beat.insert(id, now);
        }
    }
    for (id, code) in finished {
        finish_review(app, &id, code);
    }
}

fn finish_review(app: &AppHandle, id: &str, code: Option<i32>) {
    let st = app.state::<AppState>();
    let text = std::fs::read_to_string(st.review_log_path(id)).unwrap_or_default();
    let mut outcome: Option<(String, ReviewStatus, Option<String>)> = None;
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            let (status, summary) = if code == Some(0) {
                agent::parse_verdict(&text)
            } else {
                (ReviewStatus::Failed, None)
            };
            let summary = summary
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    let tail = agent::tail(&text);
                    if tail.trim().is_empty() {
                        None
                    } else {
                        Some(tail)
                    }
                });
            let mut pause = false;
            if let Some(r) = t.review.as_mut() {
                r.status = status;
                r.summary = summary.clone();
                r.finished_at = Some(now());
                pause = r.mode == ReviewMode::Pause && status == ReviewStatus::Issues;
            }
            if pause {
                let msg = summary
                    .clone()
                    .unwrap_or_else(|| "review found issues".into());
                t.status = TaskStatus::Blocked;
                t.error = Some(format!("review found issues: {msg}"));
                t.finished_at = Some(now());
            }
            outcome = Some((t.title.clone(), status, summary));
        }
    }
    crate::state::lock(&st.heartbeat).remove(id);
    st.save();
    emit_state(app);
    if let Some((title, status, summary)) = outcome {
        let body = match status {
            ReviewStatus::Passed => format!("{title} — review passed"),
            ReviewStatus::Issues => format!(
                "{title} — review: {}",
                summary.unwrap_or_else(|| "issues".into())
            ),
            ReviewStatus::Failed => format!("{title} — review failed"),
            _ => format!("{title} — review done"),
        };
        notify(app, "Solayge review", &body);
    }
}

fn spawn_review_reader<R>(app: AppHandle, id: String, reader: R)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let log_path = {
            let st = app.state::<AppState>();
            st.review_log_path(&id)
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .ok();
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(f) = file.as_mut() {
                let _ = f.write_all(line.as_bytes()).await;
                let _ = f.write_all(b"\n").await;
            }
        }
    });
}

fn spawn_reader<R>(app: AppHandle, id: String, reader: R, stream: &'static str)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let log_path = {
            let st = app.state::<AppState>();
            st.log_path(&id)
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await
            .ok();
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(f) = file.as_mut() {
                let _ = f.write_all(line.as_bytes()).await;
                let _ = f.write_all(b"\n").await;
            }
            {
                let st = app.state::<AppState>();
                crate::state::lock(&st.heartbeat).insert(id.clone(), now());
            }
            if line.to_ascii_lowercase().contains("permission requested") {
                {
                    let st = app.state::<AppState>();
                    let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
                        t.last_permission = Some(line.clone());
                    }
                }
                let _ = app.emit(
                    "task://permission",
                    LogEvent {
                        task_id: id.clone(),
                        stream: stream.to_string(),
                        line: line.clone(),
                    },
                );
                emit_state(&app);
                notify(&app, "Permission requested", &line);
            }
            let _ = app.emit(
                "task://log",
                LogEvent {
                    task_id: id.clone(),
                    stream: stream.to_string(),
                    line,
                },
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        active_heartbeat_ids, choose_branch, dispatch, now, resolve_named_branch, review_clear,
        reviews_to_start, stall_sweep, supervised_tick, BranchChoice,
    };
    use std::path::Path;
    use crate::models::{
        BranchMode, Isolation, PermissionProfile, ReviewMode, ReviewStatus, Task, TaskKind,
        TaskReview, TaskStatus,
    };
    use crate::state::AppState;

    fn task(
        id: &str,
        project: &str,
        deps: &[&str],
        isolation: Isolation,
        not_before: Option<i64>,
    ) -> Task {
        Task {
            id: id.to_string(),
            project_path: project.to_string(),
            title: id.to_string(),
            prompt: "p".to_string(),
            isolation,
            profile: PermissionProfile::default(),
            last_permission: None,
            base_ref: None,
            branch: None,
            worktree_path: None,
            not_before,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            status: if deps.is_empty() && not_before.is_none() {
                TaskStatus::Ready
            } else {
                TaskStatus::Waiting
            },
            exit_code: None,
            error: None,
            created_at: now(),
            started_at: None,
            finished_at: None,
            provider: None,
            model: None,
            fallback_provider: None,
            fallback_model: None,
            used_fallback: false,
            review: None,
            kind: TaskKind::Agent,
            git_op: None,
            command: None,
            merge: None,
            branch_mode: BranchMode::Current,
            new_branch: None,
            ask: None,
        }
    }

    fn state_with(tasks: Vec<Task>, concurrency: usize) -> AppState {
        let dir = std::env::temp_dir().join(format!("dt-test-{}", uuid::Uuid::new_v4()));
        let st = AppState::new(dir);
        {
            let mut inner = st.inner.lock().unwrap();
            inner.concurrency = concurrency;
            inner.tasks = tasks;
        }
        st
    }

    fn count(st: &AppState, status: TaskStatus) -> usize {
        st.inner
            .lock()
            .unwrap()
            .tasks
            .iter()
            .filter(|t| t.status == status)
            .count()
    }

    fn reviewed(mut t: Task, mode: ReviewMode, status: ReviewStatus) -> Task {
        t.review = Some(TaskReview {
            mode,
            status,
            provider: None,
            model: None,
            summary: None,
            started_at: None,
            finished_at: None,
        });
        t
    }

    fn running(id: &str, started_at: Option<i64>) -> Task {
        let mut t = task(id, "/p", &[], Isolation::Worktree, None);
        t.status = TaskStatus::Running;
        t.started_at = started_at;
        t
    }

    fn review_is(st: &AppState, id: &str) -> ReviewStatus {
        st.inner
            .lock()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.review.as_ref())
            .map(|r| r.status)
            .expect("task has a review")
    }

    // ---- review gating ----

    #[test]
    fn review_clear_reflects_review_state() {
        let plain = task("t", "/p", &[], Isolation::Worktree, None);
        assert!(review_clear(&plain), "no review means clear");

        assert!(!review_clear(&reviewed(
            plain.clone(),
            ReviewMode::Autofix,
            ReviewStatus::Pending
        )));
        assert!(!review_clear(&reviewed(
            plain.clone(),
            ReviewMode::Autofix,
            ReviewStatus::Running
        )));
        assert!(review_clear(&reviewed(
            plain.clone(),
            ReviewMode::Autofix,
            ReviewStatus::Passed
        )));
        assert!(review_clear(&reviewed(
            plain.clone(),
            ReviewMode::Report,
            ReviewStatus::Issues
        )));
        assert!(review_clear(&reviewed(
            plain,
            ReviewMode::Autofix,
            ReviewStatus::Failed
        )));
    }

    #[test]
    fn autofix_review_releases_dependents_only_when_done() {
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Succeeded;
        let a = reviewed(a, ReviewMode::Autofix, ReviewStatus::Running);
        let b = task("b", "/p", &["a"], Isolation::Worktree, None);
        let st = state_with(vec![a, b], 3);

        assert!(
            dispatch(&st).0.is_empty(),
            "dependent started while the autofix review was running"
        );

        st.inner.lock().unwrap().tasks[0]
            .review
            .as_mut()
            .unwrap()
            .status = ReviewStatus::Passed;
        assert_eq!(dispatch(&st).0, vec!["b".to_string()]);
    }

    #[test]
    fn settled_review_issues_block_dependents() {
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Blocked; // review pause mode stopped it
        let a = reviewed(a, ReviewMode::Pause, ReviewStatus::Issues);
        let b = task("b", "/p", &["a"], Isolation::Worktree, None);
        let st = state_with(vec![a, b], 3);

        dispatch(&st);
        let b = st
            .inner
            .lock()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "b")
            .map(|t| t.status);
        assert_eq!(b, Some(TaskStatus::Blocked));
        assert_eq!(count(&st, TaskStatus::Running), 0);
    }

    #[test]
    fn reviews_to_start_claims_only_queued_reviews() {
        let mut queued = task("queued", "/p", &[], Isolation::Worktree, None);
        queued.status = TaskStatus::Succeeded;
        let mut in_flight = task("in_flight", "/p", &[], Isolation::Worktree, None);
        in_flight.status = TaskStatus::Succeeded;
        let mut off = task("off", "/p", &[], Isolation::Worktree, None);
        off.status = TaskStatus::Succeeded;
        let not_done = task("not_done", "/p", &[], Isolation::Worktree, None);

        let mut tasks = vec![
            reviewed(queued, ReviewMode::Autofix, ReviewStatus::Pending),
            reviewed(in_flight, ReviewMode::Autofix, ReviewStatus::Running),
            reviewed(off, ReviewMode::Off, ReviewStatus::Pending),
            reviewed(not_done, ReviewMode::Autofix, ReviewStatus::Pending),
        ];

        assert_eq!(reviews_to_start(&mut tasks), vec!["queued".to_string()]);
        // The claimed review is flipped to running; nothing else moves.
        assert_eq!(tasks[0].review.as_ref().unwrap().status, ReviewStatus::Running);
        assert_eq!(
            tasks[1].review.as_ref().unwrap().status,
            ReviewStatus::Running
        );
        assert_eq!(tasks[2].review.as_ref().unwrap().status, ReviewStatus::Pending);
        assert_eq!(tasks[3].review.as_ref().unwrap().status, ReviewStatus::Pending);
    }

    /// Reproduces the original freeze at the scheduler level: claiming a queued
    /// review and then persisting must not deadlock on the state lock.
    #[test]
    fn claiming_a_review_then_saving_does_not_deadlock() {
        let mut t = task("t", "/p", &[], Isolation::Worktree, None);
        t.status = TaskStatus::Succeeded;
        let st = state_with(
            vec![reviewed(t, ReviewMode::Autofix, ReviewStatus::Pending)],
            3,
        );
        let claimed = {
            let mut inner = st.inner.lock().unwrap();
            reviews_to_start(&mut inner.tasks)
        }; // the guard is dropped before saving, as it must be
        assert_eq!(claimed, vec!["t".to_string()]);
        st.save(); // hung forever before the fix
        assert_eq!(review_is(&st, "t"), ReviewStatus::Running);
    }

    // ---- stall detection ----

    #[test]
    fn stall_sweep_flags_only_silent_runs() {
        let now = 10_000;
        let tasks = vec![
            running("fresh", Some(now - 1_000)),
            running("stale", Some(now - 5_000)),
            running("no_heartbeat", None),
            task("not_running", "/p", &[], Isolation::Worktree, None),
        ];
        let mut hb = std::collections::HashMap::new();
        hb.insert("fresh".to_string(), now);
        hb.insert("stale".to_string(), now - 5_000);
        // "no_heartbeat" has no heartbeat and no start time, so it is treated as
        // just-started and left alone.

        let sweep = stall_sweep(&tasks, &hb, now, 1_000);
        assert_eq!(sweep.tasks, vec!["stale".to_string()]);
        assert!(sweep.reviews.is_empty());
    }

    #[test]
    fn stall_sweep_boundary_is_inclusive() {
        let now = 10_000;
        let mut hb = std::collections::HashMap::new();
        hb.insert("edge".to_string(), now - 1_000);
        // Exactly at the threshold is not yet stalled.
        let sweep = stall_sweep(&[running("edge", None)], &hb, now, 1_000);
        assert!(sweep.tasks.is_empty());
        // One second past it, it is.
        let sweep = stall_sweep(&[running("edge", None)], &hb, now, 999);
        assert_eq!(sweep.tasks, vec!["edge".to_string()]);
    }

    #[test]
    fn stall_sweep_flags_silent_reviews() {
        let now = 10_000;
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Succeeded;
        let mut b = task("b", "/p", &[], Isolation::Worktree, None);
        b.status = TaskStatus::Succeeded;
        let tasks = vec![
            reviewed(a, ReviewMode::Autofix, ReviewStatus::Running),
            reviewed(b, ReviewMode::Autofix, ReviewStatus::Running),
        ];
        let mut hb = std::collections::HashMap::new();
        hb.insert("a".to_string(), now);
        hb.insert("b".to_string(), now - 9_000);

        let sweep = stall_sweep(&tasks, &hb, now, 1_000);
        assert!(sweep.tasks.is_empty());
        assert_eq!(sweep.reviews, vec!["b".to_string()]);
    }

    #[test]
    fn dispatches_only_up_to_concurrency() {
        let st = state_with(
            (0..4)
                .map(|i| task(&format!("t{i}"), "/p", &[], Isolation::Worktree, None))
                .collect(),
            2,
        );
        let (started, _) = dispatch(&st);
        assert_eq!(started.len(), 2);
        assert_eq!(count(&st, TaskStatus::Running), 2);
        assert_eq!(count(&st, TaskStatus::Ready), 2);
    }

    #[test]
    fn delay_holds_task_until_due() {
        let st = state_with(
            vec![task(
                "t0",
                "/p",
                &[],
                Isolation::Worktree,
                Some(now() + 3600),
            )],
            3,
        );
        let (started, _) = dispatch(&st);
        assert!(started.is_empty());
        assert_eq!(count(&st, TaskStatus::Waiting), 1);
    }

    #[test]
    fn due_delay_lets_task_run() {
        let st = state_with(
            vec![task("t0", "/p", &[], Isolation::Worktree, Some(now() - 5))],
            3,
        );
        let (started, _) = dispatch(&st);
        assert_eq!(started.len(), 1);
    }

    #[test]
    fn failed_dependency_blocks_downstream() {
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Failed;
        let b = task("b", "/p", &["a"], Isolation::Worktree, None);
        let st = state_with(vec![a, b], 3);
        dispatch(&st);
        assert_eq!(count(&st, TaskStatus::Blocked), 1);
        assert_eq!(count(&st, TaskStatus::Running), 0);
    }

    #[test]
    fn dependency_success_releases_downstream() {
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Succeeded;
        let b = task("b", "/p", &["a"], Isolation::Worktree, None);
        let st = state_with(vec![a, b], 3);
        let (started, _) = dispatch(&st);
        assert_eq!(started, vec!["b".to_string()]);
    }

    #[test]
    fn pending_review_holds_dependents_until_it_finishes() {
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Succeeded;
        a.review = Some(TaskReview {
            mode: ReviewMode::Pause,
            status: ReviewStatus::Pending,
            provider: None,
            model: None,
            summary: None,
            started_at: None,
            finished_at: None,
        });
        let b = task("b", "/p", &["a"], Isolation::Worktree, None);
        let st = state_with(vec![a, b], 3);

        let (started, _) = dispatch(&st);
        assert!(started.is_empty(), "dependent started before review finished");
        assert_eq!(count(&st, TaskStatus::Waiting), 1);

        // The dependent is released only once the review reaches a verdict.
        st.inner.lock().unwrap().tasks[0].review.as_mut().unwrap().status =
            ReviewStatus::Passed;
        let (started, _) = dispatch(&st);
        assert_eq!(started, vec!["b".to_string()]);
    }

    #[test]
    fn interrupted_dependency_blocks_downstream() {
        let mut a = task("a", "/p", &[], Isolation::Worktree, None);
        a.status = TaskStatus::Interrupted;
        let b = task("b", "/p", &["a"], Isolation::Worktree, None);
        let st = state_with(vec![a, b], 3);
        dispatch(&st);
        assert_eq!(count(&st, TaskStatus::Blocked), 1);
        assert_eq!(count(&st, TaskStatus::Running), 0);
    }

    #[test]
    fn shared_tasks_in_same_project_are_serialized() {
        let st = state_with(
            vec![
                task("a", "/p", &[], Isolation::Shared, None),
                task("b", "/p", &[], Isolation::Shared, None),
            ],
            4,
        );
        let (started, _) = dispatch(&st);
        assert_eq!(started.len(), 1);
        assert_eq!(count(&st, TaskStatus::Running), 1);
        assert_eq!(count(&st, TaskStatus::Ready), 1);
    }

    #[test]
    fn shared_tasks_in_different_projects_run_concurrently() {
        let st = state_with(
            vec![
                task("a", "/p1", &[], Isolation::Shared, None),
                task("b", "/p2", &[], Isolation::Shared, None),
            ],
            4,
        );
        let (started, _) = dispatch(&st);
        assert_eq!(started.len(), 2);
    }

    // ---- scheduler supervision (the app must never freeze if a tick breaks) ----

    #[tokio::test]
    async fn a_healthy_tick_completes_normally() {
        assert!(supervised_tick(|| async {}).await.is_ok());
    }

    #[tokio::test]
    async fn a_panicking_tick_is_caught_and_reported() {
        // Before the supervisor, a panic here silently ended the scheduler loop
        // and every running task froze. It must now surface as an error and the
        // caller's loop is free to keep ticking.
        let result = supervised_tick(|| async { panic!("simulated tick failure") }).await;
        assert!(
            result.is_err_and(|e| e.contains("tick failed")),
            "a panicking tick must be reported, not propagated"
        );
        // And the supervisor can run another tick afterwards.
        assert!(supervised_tick(|| async {}).await.is_ok());
    }

    // ---- heartbeat growth ----

    #[test]
    fn heartbeat_keeps_only_running_tasks_and_running_reviews() {
        let mut succeeded = task("done", "/p", &[], Isolation::Worktree, None);
        succeeded.status = TaskStatus::Succeeded;
        let mut failed = task("bad", "/p", &[], Isolation::Worktree, None);
        failed.status = TaskStatus::Failed;
        let mut blocked = task("blocked", "/p", &[], Isolation::Worktree, None);
        blocked.status = TaskStatus::Blocked;
        let mut interrupted = task("int", "/p", &[], Isolation::Worktree, None);
        interrupted.status = TaskStatus::Interrupted;

        let mut reviewing = task("rev", "/p", &[], Isolation::Worktree, None);
        reviewing.status = TaskStatus::Succeeded;
        let reviewing = reviewed(reviewing, ReviewMode::Autofix, ReviewStatus::Running);

        let mut review_done = task("revdone", "/p", &[], Isolation::Worktree, None);
        review_done.status = TaskStatus::Succeeded;
        let review_done = reviewed(review_done, ReviewMode::Autofix, ReviewStatus::Passed);

        let tasks = vec![
            running("live", Some(1)),
            succeeded,
            failed,
            blocked,
            interrupted,
            reviewing,
            review_done,
        ];
        let active = active_heartbeat_ids(&tasks);
        assert!(active.contains("live"), "a running task keeps its heartbeat");
        assert!(active.contains("rev"), "a running review keeps its heartbeat");
        assert_eq!(active.len(), 2, "finished/blocked tasks are dropped: {active:?}");
    }

    // ---- new-branch reuse on retry ----

    #[test]
    fn choose_branch_prefers_request_then_reuse_then_agent() {
        // An explicit request wins, and is not treated as "reused" unless it is
        // the same name recorded previously.
        assert!(matches!(
            choose_branch("feat", None),
            BranchChoice::Named { name, reused: false } if name == "feat"
        ));
        assert!(matches!(
            choose_branch("feat", Some("feat")),
            BranchChoice::Named { name, reused: true } if name == "feat"
        ));
        // A blank request falls back to the recorded name.
        assert!(matches!(
            choose_branch("", Some("devtools/abc")),
            BranchChoice::Named { name, reused: true } if name == "devtools/abc"
        ));
        // Neither: ask the agent.
        assert!(matches!(choose_branch("", None), BranchChoice::AskAgent));
        // A recorded-but-blank name is ignored (the caller trims first).
        assert!(matches!(
            choose_branch("", Some("  ")),
            BranchChoice::AskAgent
        ));
    }

    async fn init_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "test"],
            // Don't inherit a global commit-signing config from this machine.
            vec!["config", "commit.gpgsign", "false"],
            vec!["commit", "--allow-empty", "-qm", "init"],
        ] {
            let out = crate::git::git_cmd(dir, &args).output().await.unwrap();
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[tokio::test]
    async fn recreates_a_reused_branch_that_was_deleted_between_attempts() {
        let dir = std::env::temp_dir().join(format!("solayge-branch-{}", uuid::Uuid::new_v4()));
        init_repo(&dir).await;
        let run = |args: Vec<&'static str>| {
            let d = dir.clone();
            async move { crate::git::git_cmd(&d, &args).output().await.unwrap() }
        };
        assert!(run(vec!["branch", "feat"]).await.status.success());

        // While the branch exists, a recorded name is reused as-is.
        let (name, exists) = resolve_named_branch(&dir, "feat", true).await.unwrap();
        assert_eq!(name, "feat");
        assert!(exists, "an existing reused branch should be reported as existing");

        // Simulate the branch being deleted between attempts (e.g. "remove
        // worktree").
        assert!(run(vec!["branch", "-D", "feat"]).await.status.success());
        assert!(!crate::git::branch_exists(&dir, "feat").await);

        // The bug: this used to be assumed to exist, so `git checkout feat`
        // failed and the retry errored out. It must now be reported as missing.
        let (name, exists) = resolve_named_branch(&dir, "feat", true).await.unwrap();
        assert_eq!(name, "feat");
        assert!(
            !exists,
            "a deleted reused branch must be reported as missing so it is recreated"
        );

        // And the caller's recreation path must then succeed.
        let out = run(vec!["checkout", "-b", "feat"]).await;
        assert!(
            out.status.success(),
            "recreating the branch should succeed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(crate::git::current_branch(&dir).await.as_deref(), Some("feat"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_explicit_request_for_an_existing_branch_is_an_error() {
        let dir = std::env::temp_dir().join(format!("solayge-branch2-{}", uuid::Uuid::new_v4()));
        init_repo(&dir).await;
        let out = crate::git::git_cmd(&dir, &["branch", "feat"])
            .output()
            .await
            .unwrap();
        assert!(out.status.success());

        // A fresh explicit request for a name that already exists is rejected...
        assert!(resolve_named_branch(&dir, "feat", false).await.is_err());
        // ...but the same name recorded from a previous attempt is allowed.
        assert!(resolve_named_branch(&dir, "feat", true).await.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
