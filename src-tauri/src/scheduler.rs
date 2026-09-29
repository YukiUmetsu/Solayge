use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};

use crate::agent;
use crate::git;
use crate::models::{
    BranchMode, CommandTemplates, ConflictMode, GitOp, Isolation, LogEvent, MergeSpec,
    MergeStrategy, PermissionProfile, Provider, ReviewMode, ReviewStatus, SystemPrompt, TaskKind,
    TaskStatus,
};
use crate::state::AppState;
use tauri_plugin_notification::NotificationExt;

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
            step(&app).await;
        }
    });
}

async fn step(app: &AppHandle) {
    if reap(app) {
        // finalize() already emitted.
    }
    reap_reviews(app);
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
    {
        let mut running = st.running.lock().expect("running lock");
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
                    Ok(None) => {}
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
    let mut start_review_now = false;
    let mut retry_note: Option<(PathBuf, String)> = None;
    {
        let mut inner = st.inner.lock().expect("state lock");
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if t.status == TaskStatus::Running {
                let ok = code == Some(0);
                if ok {
                    t.exit_code = code;
                    t.finished_at = Some(now());
                    t.status = TaskStatus::Succeeded;
                    if let Some(r) = t.review.as_mut() {
                        if r.mode != ReviewMode::Off {
                            r.status = ReviewStatus::Pending;
                            start_review_now = true;
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

    if start_review_now {
        let a = app.clone();
        let tid = id.to_string();
        tauri::async_runtime::spawn(async move {
            start_review(a, tid).await;
        });
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

/// Promote waiting tasks and dispatch ready ones. Returns (started ids, mutated).
fn dispatch(st: &AppState) -> (Vec<String>, bool) {
    let mut started = Vec::new();
    let mut mutated = false;
    let mut inner = st.inner.lock().expect("state lock");
    let now = now();

    let succeeded: HashSet<String> = inner
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Succeeded)
        .map(|t| t.id.clone())
        .collect();
    let failed: HashSet<String> = inner
        .tasks
        .iter()
        .filter(|t| {
            matches!(
                t.status,
                TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Blocked
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
        let mut inner = st.inner.lock().expect("state lock");
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
    let inner = st.inner.lock().expect("state lock");
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
    })
}

fn project_context(app: &AppHandle, project: &str) -> ProjectContext {
    let st = app.state::<AppState>();
    let (name, system_prompt, keys, kind, conflict_mode) = {
        let inner = st.inner.lock().expect("state lock");
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

async fn init_log(app: &AppHandle, id: &str) {
    let (log_path, logs_dir) = {
        let st = app.state::<AppState>();
        (st.log_path(id), st.logs_dir())
    };
    let _ = tokio::fs::create_dir_all(&logs_dir).await;
    let _ = tokio::fs::write(&log_path, b"").await;
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

/// Spawn a child, stream its output to the task log, and register it so it can
/// be cancelled. Finalization is left to `reap`.
fn spawn_simple(app: &AppHandle, id: &str, mut cmd: tokio::process::Command) {
    match cmd.spawn() {
        Ok(mut child) => {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            {
                let st = app.state::<AppState>();
                st.running
                    .lock()
                    .expect("running lock")
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
        Err(e) => fail_task(app, id, format!("failed to launch: {e}")),
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
    match prepare_worktree(project_path, &run.id, run.base_ref.as_deref()).await {
        Ok((wt, branch)) => {
            {
                let st = app.state::<AppState>();
                let mut inner = st.inner.lock().expect("state lock");
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

/// Create and check out the task's new branch, returning its name. A blank
/// requested name is chosen by the agent and de-duplicated.
async fn ensure_new_branch(
    app: &AppHandle,
    run: &TaskRun,
    project_path: &Path,
) -> Result<String, String> {
    let requested = run.new_branch.as_deref().map(str::trim).unwrap_or("");

    let name = if !requested.is_empty() {
        let name = agent::sanitize_branch_name(requested);
        if git::branch_exists(project_path, &name).await {
            return Err(format!("branch \"{name}\" already exists"));
        }
        name
    } else {
        let mut cmd = agent::build_command(
            run.provider,
            run.model.as_deref(),
            &command_templates(app),
            &agent::branch_name_prompt(&run.title),
            PermissionProfile::Readonly,
            project_path,
        )?;
        agent::apply_env(&mut cmd, &project_context(app, &run.project).env);
        let suggested = match tokio::time::timeout(Duration::from_secs(45), cmd.output()).await {
            Ok(Ok(out)) => agent::sanitize_branch_name(&String::from_utf8_lossy(&out.stdout)),
            // Fall back to a slug of the title if the agent is unavailable.
            _ => agent::sanitize_branch_name(&run.title),
        };
        let mut name = suggested.clone();
        let mut n = 1;
        while git::branch_exists(project_path, &name).await {
            n += 1;
            name = format!("{suggested}-{n}");
        }
        name
    };

    let mut args = vec!["checkout", "-b", name.as_str()];
    if let Some(base) = run.base_ref.as_deref().filter(|b| !b.trim().is_empty()) {
        if !git::valid_ref(base) {
            return Err(format!("invalid base ref: {base}"));
        }
        args.push(base);
    }
    let output = git::git_cmd(project_path, &args)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "could not create branch {name}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    {
        let st = app.state::<AppState>();
        let mut inner = st.inner.lock().expect("state lock");
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == run.id) {
            t.branch = Some(name.clone());
        }
    }
    log_note(app, &run.id, &format!("On new branch {name}"));
    Ok(name)
}

async fn start_task(app: AppHandle, id: String) {
    let Some(run) = load_run(&app, &id) else {
        return;
    };
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
    let inner = st.inner.lock().expect("state lock");
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
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log_note(app, id, &format!("failed to start command: {e}"));
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
            .expect("merging lock")
            .insert(id.to_string(), child);
    }
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let st = app.state::<AppState>();
        let mut map = st.merging.lock().expect("merging lock");
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
    let inner = st.inner.lock().expect("state lock");
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
        let mut inner = st.inner.lock().expect("state lock");
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
        let mut inner = st.inner.lock().expect("state lock");
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
        let inner = st.inner.lock().expect("state lock");
        let Some(t) = inner.tasks.iter().find(|t| t.id == id) else {
            return;
        };
        let Some(r) = t.review.as_ref() else { return };
        if r.mode == ReviewMode::Off {
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
            let inner = st.inner.lock().expect("state lock");
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
    let _ = tokio::fs::write(&log_path, b"").await;

    {
        let st = app.state::<AppState>();
        let mut inner = st.inner.lock().expect("state lock");
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if let Some(r) = t.review.as_mut() {
                r.status = ReviewStatus::Running;
                r.started_at = Some(now());
                r.finished_at = None;
                r.summary = None;
            }
        }
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
                    .expect("reviewing lock")
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
        let mut inner = st.inner.lock().expect("state lock");
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == id) {
            if let Some(r) = t.review.as_mut() {
                r.status = ReviewStatus::Failed;
                r.summary = Some(msg);
                r.finished_at = Some(now());
            }
        }
    }
    st.save();
    emit_state(app);
}

fn reap_reviews(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut finished: Vec<(String, Option<i32>)> = Vec::new();
    {
        let mut reviewing = st.reviewing.lock().expect("reviewing lock");
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
                    Ok(None) => {}
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
    for (id, code) in finished {
        finish_review(app, &id, code);
    }
}

fn finish_review(app: &AppHandle, id: &str, code: Option<i32>) {
    let st = app.state::<AppState>();
    let text = std::fs::read_to_string(st.review_log_path(id)).unwrap_or_default();
    let mut outcome: Option<(String, ReviewStatus, Option<String>)> = None;
    {
        let mut inner = st.inner.lock().expect("state lock");
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
            if line.to_ascii_lowercase().contains("permission requested") {
                {
                    let st = app.state::<AppState>();
                    let mut inner = st.inner.lock().expect("state lock");
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
    use super::{dispatch, now};
    use crate::models::{BranchMode, Isolation, PermissionProfile, Task, TaskKind, TaskStatus};
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
}
