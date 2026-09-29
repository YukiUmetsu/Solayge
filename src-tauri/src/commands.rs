use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::models::*;
use crate::scheduler::{emit_state, now};
use crate::state::AppState;
use crate::{agent, cache, git, opencode, secrets};

#[tauri::command]
pub fn get_snapshot(app: AppHandle) -> Snapshot {
    crate::state::snapshot(&app)
}

/// POSIX-style environment variable names only, so `Command::env` never sees a
/// name that would confuse the child process.
fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Clear a finished task's run state so it can run again. Worktrees are
/// recreated (their path is deterministic); a chosen branch is kept so retries
/// reuse the same name instead of piling up new branches.
fn reset_for_rerun(t: &mut Task) {
    t.not_before = None;
    t.exit_code = None;
    t.error = None;
    t.started_at = None;
    t.finished_at = None;
    t.last_permission = None;
    t.used_fallback = false;
    t.ask = None;
    if t.isolation == Isolation::Worktree {
        t.worktree_path = None;
        t.branch = None;
    }
    if let Some(r) = t.review.as_mut() {
        r.status = ReviewStatus::None;
        r.summary = None;
        r.started_at = None;
        r.finished_at = None;
    }
    t.status = if t.depends_on.is_empty() {
        TaskStatus::Ready
    } else {
        TaskStatus::Waiting
    };
}

#[tauri::command]
pub async fn add_project(app: AppHandle, path: String) -> Result<Snapshot, String> {
    let p = PathBuf::from(&path);
    if !git::is_repo(&p).await {
        return Err("Not a git repository (a git repo is required for worktrees)".into());
    }
    let name = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&path)
        .to_string();
    {
        let st = app.state::<AppState>();
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.projects.iter().any(|x| x.path == path) {
            inner.projects.push(Project {
                path: path.clone(),
                name,
                added_at: now(),
                default_base_ref: None,
                default_profile: None,
                provider: None,
                model: None,
                fallback_provider: None,
                fallback_model: None,
                review_provider: None,
                review_model: None,
                review_mode: None,
                editor: None,
                env_vars: Vec::new(),
                skills: Vec::new(),
                system_prompt: None,
                conflict_mode: None,
            });
        }
        drop(inner);
        st.save();
    }
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn remove_project(app: AppHandle, path: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let ids: Vec<(String, Option<String>, Option<String>)> = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .tasks
            .iter()
            .filter(|t| t.project_path == path)
            .map(|t| (t.id.clone(), t.worktree_path.clone(), t.branch.clone()))
            .collect()
    };
    for (id, wt, branch) in &ids {
        if let Some(w) = wt {
            let _ = git::worktree_remove(Path::new(&path), Path::new(w)).await;
        }
        if let Some(b) = branch {
            let _ = git::branch_delete(Path::new(&path), b).await;
        }
        let _ = std::fs::remove_file(st.log_path(id));
    }
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.projects.retain(|p| p.path != path);
        inner.tasks.retain(|t| t.project_path != path);
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn project_status(path: String) -> Result<GitStatus, String> {
    git::status(Path::new(&path))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn project_worktrees(path: String) -> Result<Vec<Worktree>, String> {
    git::worktrees(Path::new(&path))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn git_diff(path: String, target: Option<String>) -> Result<DiffResult, String> {
    let dir = target.unwrap_or(path);
    git::diff(Path::new(&dir)).await.map_err(|e| e.to_string())
}

/// The repository's default branch (origin/HEAD, else main/master, else HEAD).
#[tauri::command]
pub async fn project_default_branch(path: String) -> String {
    git::default_branch(Path::new(&path)).await
}

/// Diff of the working tree against the default branch, including uncommitted
/// and untracked changes.
#[tauri::command]
pub async fn project_branch_diff(path: String) -> Result<DiffResult, String> {
    let dir = Path::new(&path);
    let base = git::default_branch(dir).await;
    git::diff_against(dir, &base)
        .await
        .map_err(|e| e.to_string())
}

/// Release every draft task in a project, and re-queue failed / canceled /
/// blocked ones, so the scheduler runs them again.
#[tauri::command]
pub fn execute_project(app: AppHandle, project_path: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = now();
        for t in inner.tasks.iter_mut() {
            if t.project_path != project_path {
                continue;
            }
            match t.status {
                TaskStatus::Draft => {
                    let time_ok = t.not_before.is_none_or(|nb| now >= nb);
                    t.status = if t.depends_on.is_empty() && time_ok {
                        TaskStatus::Ready
                    } else {
                        TaskStatus::Waiting
                    };
                    t.error = None;
                }
                TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Blocked
                | TaskStatus::Interrupted => {
                    reset_for_rerun(t);
                }
                _ => {}
            }
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn create_tasks(
    app: AppHandle,
    project_path: String,
    tasks: Vec<NewTask>,
    default_isolation: Option<Isolation>,
) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    let existing: HashSet<String> = inner.tasks.iter().map(|t| t.id.clone()).collect();
    let project_default = inner
        .projects
        .iter()
        .find(|p| p.path == project_path)
        .and_then(|p| p.default_profile);
    let global_default = inner.settings.default_profile;
    let resolved = {
        let proj = inner.projects.iter().find(|p| p.path == project_path);
        agent::resolve(proj, &inner.settings)
    };
    let base = now();

    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut new_ids: Vec<String> = Vec::with_capacity(tasks.len());
    for nt in &tasks {
        let nid = uuid::Uuid::new_v4().to_string();
        if let Some(lid) = nt.local_id.as_ref().filter(|s| !s.is_empty()) {
            id_map.insert(lid.clone(), nid.clone());
        }
        new_ids.push(nid);
    }

    for (i, nt) in tasks.iter().enumerate() {
        let id = new_ids[i].clone();
        let mut depends_on: Vec<String> = Vec::new();
        for a in &nt.after {
            if let Some(mapped) = id_map.get(a) {
                depends_on.push(mapped.clone());
            } else if existing.contains(a) {
                depends_on.push(a.clone());
            }
        }
        let delay = nt.delay_seconds.unwrap_or(0).max(0);
        let not_before = if delay > 0 { Some(base + delay) } else { None };
        // Tasks are created as drafts: nothing runs until the project is
        // executed (or the task is started explicitly).
        let status = TaskStatus::Draft;
        inner.tasks.push(Task {
            id,
            project_path: project_path.clone(),
            title: nt.title.clone(),
            prompt: nt.prompt.clone(),
            isolation: nt
                .isolation
                .unwrap_or(default_isolation.unwrap_or_default()),
            profile: nt
                .profile
                .or(project_default)
                .or(global_default)
                .unwrap_or_default(),
            last_permission: None,
            base_ref: nt.base_ref.clone(),
            branch: None,
            worktree_path: None,
            not_before,
            depends_on,
            status,
            exit_code: None,
            error: None,
            created_at: base + i as i64,
            started_at: None,
            finished_at: None,
            provider: Some(resolved.provider),
            model: resolved.model.clone(),
            fallback_provider: resolved.fallback_provider,
            fallback_model: resolved.fallback_model.clone(),
            used_fallback: false,
            review: if resolved.review_mode == ReviewMode::Off {
                None
            } else {
                Some(TaskReview {
                    mode: resolved.review_mode,
                    status: ReviewStatus::None,
                    provider: Some(resolved.review_provider),
                    model: resolved.review_model.clone(),
                    summary: None,
                    started_at: None,
                    finished_at: None,
                })
            },
            kind: nt.kind.unwrap_or_default(),
            git_op: nt.git_op,
            command: nt.command.clone(),
            merge: nt.merge.clone(),
            branch_mode: nt.branch_mode.unwrap_or_default(),
            new_branch: nt.new_branch.clone(),
            ask: None,
        });
    }
    drop(inner);
    st.save();

    // Cache the prompts for re-use in New task / Plan with AI (agent tasks only).
    for nt in &tasks {
        if !matches!(nt.kind, None | Some(TaskKind::Agent)) {
            continue;
        }
        cache::record_prompt(
            &st,
            Some(project_path.clone()),
            nt.title.clone(),
            nt.prompt.clone(),
            nt.profile.or(project_default).or(global_default),
            nt.isolation.or(default_isolation),
        );
    }
    cache::prune(&st);

    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn update_task(app: AppHandle, task_id: String, patch: TaskPatch) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            if let Some(v) = patch.title {
                t.title = v;
            }
            if let Some(v) = patch.prompt {
                t.prompt = v;
            }
            if let Some(v) = patch.isolation {
                t.isolation = v;
            }
            if let Some(v) = patch.profile {
                t.profile = v;
            }
            if let Some(v) = patch.base_ref {
                t.base_ref = Some(v);
            }
            if let Some(v) = patch.delay_seconds {
                t.not_before = if v > 0 { Some(now() + v) } else { None };
            }
            if let Some(v) = patch.depends_on {
                t.depends_on = v;
            }
            if let Some(v) = patch.command {
                t.command = Some(v);
            }
            if matches!(t.status, TaskStatus::Waiting | TaskStatus::Ready) {
                t.status = if t.depends_on.is_empty() && t.not_before.is_none() {
                    TaskStatus::Ready
                } else {
                    TaskStatus::Waiting
                };
            }
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn delete_task(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let info = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.tasks.iter().find(|t| t.id == task_id).map(|t| {
            (
                t.project_path.clone(),
                t.worktree_path.clone(),
                t.branch.clone(),
            )
        })
    };
    if let Some((project, wt, branch)) = info {
        if let Some(w) = wt {
            let _ = git::worktree_remove(Path::new(&project), Path::new(&w)).await;
        }
        if let Some(b) = branch {
            let _ = git::branch_delete(Path::new(&project), &b).await;
        }
        let _ = std::fs::remove_file(st.log_path(&task_id));
    }
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.tasks.retain(|t| t.id != task_id);
        for t in inner.tasks.iter_mut() {
            t.depends_on.retain(|d| d != &task_id);
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn start_task_now(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            if !t.status.is_terminal() && t.status != TaskStatus::Running {
                t.not_before = None;
                t.status = TaskStatus::Ready;
                t.error = None;
            }
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn cancel_task(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            if !t.status.is_terminal() {
                t.status = TaskStatus::Canceled;
                t.finished_at = Some(now());
            }
        }
    }
    {
        let mut running = st.running.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = running.get_mut(&task_id) {
            let _ = child.start_kill();
        }
    }
    {
        let mut reviewing = st.reviewing.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = reviewing.get_mut(&task_id) {
            let _ = child.start_kill();
        }
    }
    {
        let mut merging = st.merging.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = merging.get_mut(&task_id) {
            let _ = child.start_kill();
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn retry_task(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            if !t.status.is_terminal() {
                return Err("task is already active".into());
            }
            reset_for_rerun(t);
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn answer_task(
    app: AppHandle,
    task_id: String,
    answer: serde_json::Value,
) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let ask = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .and_then(|t| t.ask.clone())
    };
    let Some(ask) = ask else {
        return Err("this task has no pending question".into());
    };
    let session = ask
        .session_id
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "the question is not attached to a live session".to_string())?;

    let conn = crate::opencode_server::ensure_server().await?;
    match ask.kind {
        AskKind::Question => {
            crate::opencode_server::reply_form(&conn, &session, &ask.id, answer).await?;
        }
        AskKind::Permission => {
            let decision = answer
                .get("decision")
                .and_then(|v| v.as_str())
                .unwrap_or("reject");
            crate::opencode_server::reply_permission(&conn, &session, &ask.id, decision, None)
                .await?;
        }
    }

    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            if t.ask.as_ref().map(|a| a.id.as_str()) == Some(ask.id.as_str()) {
                t.ask = None;
            }
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn remove_task_worktree(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let info = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.tasks.iter().find(|t| t.id == task_id).map(|t| {
            (
                t.project_path.clone(),
                t.worktree_path.clone(),
                t.branch.clone(),
            )
        })
    };
    if let Some((project, wt, branch)) = info {
        if let Some(w) = wt {
            let _ = git::worktree_remove(Path::new(&project), Path::new(&w)).await;
        }
        if let Some(b) = branch {
            let _ = git::branch_delete(Path::new(&project), &b).await;
        }
    }
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            t.worktree_path = None;
            t.branch = None;
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn clear_finished(app: AppHandle, project_path: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let ids: Vec<(String, Option<String>, Option<String>)> = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .tasks
            .iter()
            .filter(|t| t.project_path == project_path && t.status.is_terminal())
            .map(|t| (t.id.clone(), t.worktree_path.clone(), t.branch.clone()))
            .collect()
    };
    for (id, wt, branch) in &ids {
        if let Some(w) = wt {
            let _ = git::worktree_remove(Path::new(&project_path), Path::new(w)).await;
        }
        if let Some(b) = branch {
            let _ = git::branch_delete(Path::new(&project_path), b).await;
        }
        let _ = std::fs::remove_file(st.log_path(id));
    }
    let removed: HashSet<String> = ids.into_iter().map(|(id, _, _)| id).collect();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.tasks.retain(|t| !removed.contains(&t.id));
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn get_task_log(app: AppHandle, task_id: String) -> Result<String, String> {
    let st = app.state::<AppState>();
    std::fs::read_to_string(st.log_path(&task_id)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_concurrency(app: AppHandle, value: usize) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.concurrency = value.clamp(1, 16);
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn set_project_default_profile(
    app: AppHandle,
    path: String,
    profile: Option<PermissionProfile>,
) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = inner.projects.iter_mut().find(|p| p.path == path) {
            p.default_profile = profile;
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn plan_with_opencode(
    app: AppHandle,
    project_path: String,
    goal: String,
    max_tasks: Option<usize>,
    timeout_secs: Option<u64>,
) -> Result<PlanResult, String> {
    let mt = max_tasks.unwrap_or(8).clamp(1, 30);
    let st = app.state::<AppState>();
    let (provider, model, templates, skills, keys, kind) = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let proj = inner.projects.iter().find(|p| p.path == project_path);
        let r = agent::resolve(proj, &inner.settings);
        let skills = proj.map(|p| p.skills.clone()).unwrap_or_default();
        let keys: Vec<String> = proj
            .map(|p| p.env_vars.iter().map(|e| e.key.clone()).collect())
            .unwrap_or_default();
        (
            r.provider,
            r.model,
            inner.settings.command_templates.clone(),
            skills,
            keys,
            secrets::StoreKind::parse(inner.settings.secret_store.as_deref()),
        )
    };
    let env = secrets::Secrets::new(&st.data_dir, kind)
        .get_many(&project_path, &keys)
        .unwrap_or_default();
    let result = opencode::plan(
        &project_path,
        &goal,
        mt,
        timeout_secs.unwrap_or(300),
        provider,
        model.as_deref(),
        &templates,
        &skills,
        &env,
    )
    .await
    .map_err(|e| e.to_string())?;

    // Cache the goal so it can be re-used as a starting point.
    let st = app.state::<AppState>();
    let label: String = goal.trim().chars().take(60).collect();
    cache::record_prompt(
        &st,
        Some(project_path),
        format!("Plan: {label}"),
        goal,
        None,
        None,
    );
    cache::prune(&st);
    Ok(result)
}

// ---- settings & cache ----

#[tauri::command]
pub fn update_settings(app: AppHandle, settings: Settings) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let (from, to, entries) = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let from = secrets::StoreKind::parse(inner.settings.secret_store.as_deref());
        let to = secrets::StoreKind::parse(settings.secret_store.as_deref());
        let entries: Vec<(String, String)> = inner
            .projects
            .iter()
            .flat_map(|p| {
                p.env_vars
                    .iter()
                    .map(move |e| (p.path.clone(), e.key.clone()))
            })
            .collect();
        (from, to, entries)
    };
    // Re-encrypt any stored secrets when the store changes; reject the change
    // if the new store is unavailable (e.g. no keychain on this platform).
    if from != to {
        secrets::Secrets::migrate(&st.data_dir, from, to, &entries).map_err(|e| e.to_string())?;
    }
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = settings;
        next.cache_retention_days = next.cache_retention_days.clamp(0, 3650);
        inner.settings = next;
    }
    st.save();
    cache::prune(&st);
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn get_prompt_history(
    app: AppHandle,
    project_path: Option<String>,
    limit: Option<usize>,
) -> Vec<PromptEntry> {
    let st = app.state::<AppState>();
    cache::history(&st, project_path.as_deref(), limit)
}

#[tauri::command]
pub fn get_cache_stats(app: AppHandle) -> CacheStats {
    let st = app.state::<AppState>();
    cache::stats(&st)
}

#[tauri::command]
pub fn clear_cache(app: AppHandle, prompts: bool, logs: bool) -> CacheStats {
    let st = app.state::<AppState>();
    let stats = cache::clear(&st, prompts, logs);
    emit_state(&app);
    stats
}

// ---- projects: order, remote, config, editor ----

#[tauri::command]
pub fn reorder_projects(app: AppHandle, paths: Vec<String>) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut ordered: Vec<Project> = Vec::with_capacity(inner.projects.len());
        for p in &paths {
            if let Some(found) = inner.projects.iter().find(|x| &x.path == p) {
                ordered.push(found.clone());
            }
        }
        for p in inner.projects.iter() {
            if !paths.iter().any(|x| x == &p.path) {
                ordered.push(p.clone());
            }
        }
        inner.projects = ordered;
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub async fn project_remote(path: String) -> Option<String> {
    git::remote_url(Path::new(&path))
        .await
        .map(|u| agent::normalize_remote(&u))
}

#[tauri::command]
pub fn open_external(target: String) -> Result<(), String> {
    agent::open_with_system(Path::new(&target)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_in_editor(
    app: AppHandle,
    path: String,
    editor: Option<String>,
) -> Result<String, String> {
    let chosen = match editor {
        Some(e) => Some(e),
        None => {
            let st = app.state::<AppState>();
            let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
            let proj = inner.projects.iter().find(|p| p.path == path);
            agent::resolve_editor(
                proj.and_then(|p| p.editor.as_deref()),
                inner.settings.editor.as_deref(),
            )
        }
    };
    agent::open_in_editor(Path::new(&path), chosen.as_deref())
}

#[tauri::command]
pub fn update_project_config(
    app: AppHandle,
    path: String,
    config: ProjectConfig,
) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let kind = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        secrets::StoreKind::parse(inner.settings.secret_store.as_deref())
    };
    let store = secrets::Secrets::new(&st.data_dir, kind);
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = inner.projects.iter_mut().find(|p| p.path == path) {
            p.provider = config.provider;
            p.model = config.model;
            p.fallback_provider = config.fallback_provider;
            p.fallback_model = config.fallback_model;
            p.review_provider = config.review_provider;
            p.review_model = config.review_model;
            p.review_mode = config.review_mode;
            p.editor = config.editor;
            p.conflict_mode = config.conflict_mode;

            if let Some(vars) = config.env_vars {
                let previous: Vec<String> = p.env_vars.iter().map(|e| e.key.clone()).collect();
                let mut kept: Vec<ProjectEnvVar> = Vec::with_capacity(vars.len());
                for v in vars {
                    let key = v.key.trim().to_string();
                    if key.is_empty() {
                        continue;
                    }
                    if !valid_env_key(&key) {
                        return Err(format!(
                            "invalid environment variable name: \"{key}\" (use letters, digits, underscore)"
                        ));
                    }
                    store
                        .set(&path, &key, &v.value)
                        .map_err(|e| e.to_string())?;
                    kept.push(ProjectEnvVar {
                        key,
                        secret: v.secret,
                    });
                }
                // Drop values for variables that were removed or renamed.
                for old in &previous {
                    if !kept.iter().any(|e| &e.key == old) {
                        let _ = store.delete(&path, old);
                    }
                }
                p.env_vars = kept;
            }

            if let Some(skills) = config.skills {
                p.skills = skills
                    .into_iter()
                    .filter(|s| !(s.name.trim().is_empty() && s.command.trim().is_empty()))
                    .collect();
            }

            if let Some(sp) = config.system_prompt {
                p.system_prompt = if sp.text.trim().is_empty() {
                    None
                } else {
                    Some(sp)
                };
            }
        }
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

/// The decrypted values of a project's environment variables, for editing.
#[tauri::command]
pub fn get_project_secrets(app: AppHandle, path: String) -> Result<Vec<EnvValue>, String> {
    let st = app.state::<AppState>();
    let (vars, kind) = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let p = inner
            .projects
            .iter()
            .find(|p| p.path == path)
            .ok_or_else(|| format!("no project at {path}"))?;
        (
            p.env_vars.clone(),
            secrets::StoreKind::parse(inner.settings.secret_store.as_deref()),
        )
    };
    let store = secrets::Secrets::new(&st.data_dir, kind);
    let mut out = Vec::with_capacity(vars.len());
    for v in vars {
        let value = store
            .get(&path, &v.key)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        out.push(EnvValue {
            key: v.key,
            value,
            secret: v.secret,
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn get_resolved_config(app: AppHandle, path: String) -> Result<ResolvedConfig, String> {
    let st = app.state::<AppState>();
    let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
    let proj = inner.projects.iter().find(|p| p.path == path);
    Ok(agent::resolve(proj, &inner.settings))
}

#[tauri::command]
pub fn get_review_log(app: AppHandle, task_id: String) -> String {
    let st = app.state::<AppState>();
    std::fs::read_to_string(st.review_log_path(&task_id)).unwrap_or_default()
}

/// Explain why a resolved CLI isn't runnable.
fn shim_note(path: &Path) -> Option<String> {
    #[cfg(windows)]
    {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") {
                return Some(
                    "Windows .cmd/.bat shim — wrap it in `cmd /C` in the command template".into(),
                );
            }
        }
    }
    let _ = path;
    None
}

fn tool_status(name: &str) -> ToolStatus {
    let resolved = agent::which(name);
    let note = resolved.as_deref().and_then(shim_note);
    ToolStatus {
        name: name.to_string(),
        found: resolved.is_some(),
        path: resolved.map(|p| p.to_string_lossy().to_string()),
        note,
    }
}

/// Preflight: are `git`, `gh`, and each provider CLI on `PATH`?
#[tauri::command]
pub fn environment_check(app: AppHandle) -> EnvironmentStatus {
    let templates = {
        let st = app.state::<AppState>();
        st.inner
            .lock()
            .map(|i| i.settings.command_templates.clone())
            .unwrap_or_default()
    };
    let providers = [
        Provider::Opencode,
        Provider::Codex,
        Provider::Claude,
        Provider::Cursor,
    ]
    .into_iter()
    .map(|p| {
        let command = templates
            .for_provider(p)
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let resolved = agent::which(&command);
        let note = resolved.as_deref().and_then(shim_note);
        ProviderTool {
            provider: p,
            command,
            found: resolved.is_some(),
            path: resolved.map(|x| x.to_string_lossy().to_string()),
            note,
        }
    })
    .collect();

    EnvironmentStatus {
        git: tool_status("git"),
        gh: tool_status("gh"),
        providers,
    }
}

#[tauri::command]
pub async fn list_models(
    app: AppHandle,
    provider: Provider,
    force: bool,
) -> Result<Vec<String>, String> {
    let key = provider.command_key().to_string();
    if !force {
        let cached = {
            let st = app.state::<AppState>();
            st.models
                .lock()
                .ok()
                .and_then(|cache| cache.get(&key).cloned())
        };
        if let Some(hit) = cached {
            return Ok(hit);
        }
    }
    let models = agent::fetch_models(provider).await;
    {
        let st = app.state::<AppState>();
        if let Ok(mut cache) = st.models.lock() {
            cache.insert(key, models.clone());
        };
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::reset_for_rerun;
    use crate::models::{
        BranchMode, Isolation, PermissionProfile, ReviewMode, ReviewStatus, Task, TaskKind,
        TaskReview, TaskStatus,
    };

    fn task(status: TaskStatus, isolation: Isolation) -> Task {
        Task {
            id: "t".into(),
            project_path: "/p".into(),
            title: "t".into(),
            prompt: "p".into(),
            isolation,
            profile: PermissionProfile::default(),
            last_permission: None,
            base_ref: None,
            branch: None,
            worktree_path: None,
            not_before: None,
            depends_on: Vec::new(),
            status,
            exit_code: None,
            error: None,
            created_at: 1,
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

    #[test]
    fn reset_for_rerun_makes_an_interrupted_task_runnable_again() {
        let mut t = task(TaskStatus::Interrupted, Isolation::Worktree);
        t.worktree_path = Some("/wt".into());
        t.branch = Some("devtools/abc".into());
        t.error = Some("Interrupted: ...".into());
        t.exit_code = Some(1);
        t.started_at = Some(10);
        t.finished_at = Some(20);
        t.used_fallback = true;
        t.review = Some(TaskReview {
            mode: ReviewMode::Autofix,
            status: ReviewStatus::Running,
            provider: None,
            model: None,
            summary: Some("stale".into()),
            started_at: Some(10),
            finished_at: None,
        });

        reset_for_rerun(&mut t);

        assert_eq!(t.status, TaskStatus::Ready);
        assert!(t.error.is_none());
        assert!(t.exit_code.is_none());
        assert!(t.started_at.is_none() && t.finished_at.is_none());
        assert!(!t.used_fallback);
        // Worktree isolation gets a fresh worktree/branch on the next attempt.
        assert!(t.worktree_path.is_none());
        assert!(t.branch.is_none());
        let r = t.review.as_ref().unwrap();
        assert_eq!(r.status, ReviewStatus::None);
        assert!(r.summary.is_none() && r.finished_at.is_none());
    }

    #[test]
    fn reset_for_rerun_waits_for_dependencies() {
        let mut t = task(TaskStatus::Interrupted, Isolation::Shared);
        t.depends_on = vec!["dep".into()];
        reset_for_rerun(&mut t);
        assert_eq!(t.status, TaskStatus::Waiting);
    }
}
