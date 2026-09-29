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

/// Environment variable names are POSIX-style only, so `Command::env` never sees
/// a name that would confuse the child process.
fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Task ids name files under the logs directory, so they must not carry path
/// separators or traversal. Real ids are UUIDs; plan ids are short labels.
fn valid_task_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether the dependency graph described by `(id, depends_on)` edges contains a
/// cycle. Pure, so the rule can be tested. A cycle would leave every task in it
/// `waiting` forever, so it is rejected before anything is persisted.
fn has_dependency_cycle(edges: &[(String, Vec<String>)]) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Fresh,
        Open,
        Done,
    }

    fn visit(
        i: usize,
        edges: &[(String, Vec<String>)],
        index: &HashMap<&str, usize>,
        marks: &mut [Mark],
    ) -> bool {
        marks[i] = Mark::Open;
        for dep in &edges[i].1 {
            let Some(&j) = index.get(dep.as_str()) else {
                continue; // dangling dep: surfaced separately, not a cycle
            };
            match marks[j] {
                Mark::Open => return true,
                Mark::Fresh => {
                    if visit(j, edges, index, marks) {
                        return true;
                    }
                }
                Mark::Done => {}
            }
        }
        marks[i] = Mark::Done;
        false
    }

    let index: HashMap<&str, usize> = edges
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.as_str(), i))
        .collect();
    let mut marks = vec![Mark::Fresh; edges.len()];
    for i in 0..edges.len() {
        if marks[i] == Mark::Fresh && visit(i, edges, &index, &mut marks) {
            return true;
        }
    }
    false
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
        inner.deleted_tasks.retain(|d| d.task.project_path != path);
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

    let mut new_tasks: Vec<Task> = Vec::with_capacity(tasks.len());
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
        new_tasks.push(Task {
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

    // Refuse a plan that could never finish: a dependency cycle leaves every
    // task in it `waiting` forever. Checked against the whole graph (existing
    // tasks included) before anything is stored.
    let mut edges: Vec<(String, Vec<String>)> = inner
        .tasks
        .iter()
        .map(|t| (t.id.clone(), t.depends_on.clone()))
        .collect();
    edges.extend(
        new_tasks
            .iter()
            .map(|t| (t.id.clone(), t.depends_on.clone())),
    );
    if has_dependency_cycle(&edges) {
        return Err("the plan contains a dependency cycle".into());
    }
    inner.tasks.extend(new_tasks);

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

/// Apply an edit to a task. Only a `draft` task may be edited: once it has been
/// released (ready/waiting) it is queued to run, and once running its prompt is
/// the instruction the agent was already given, so rewriting it would be a lie.
/// Returns an error otherwise, which the UI surfaces to the user.
fn apply_draft_patch(t: &mut Task, patch: TaskPatch) -> Result<(), String> {
    if t.status != TaskStatus::Draft {
        return Err("only draft tasks can be edited".into());
    }
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
    if let Some(v) = patch.branch_mode {
        t.branch_mode = v;
    }
    if let Some(v) = patch.new_branch {
        t.new_branch = Some(v);
    }
    // Changing what the task does must not leave another kind's config behind
    // (e.g. a stray shell command on a task that is now an agent prompt).
    if let Some(kind) = patch.kind {
        t.kind = kind;
        match kind {
            TaskKind::Agent => {
                t.command = None;
                t.git_op = None;
                t.merge = None;
            }
            TaskKind::Shell => {
                t.git_op = None;
                t.merge = None;
            }
            TaskKind::Git => {
                t.merge = None;
            }
            TaskKind::Merge => {
                t.git_op = None;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn update_task(app: AppHandle, task_id: String, patch: TaskPatch) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = crate::state::lock(&st.inner);
        let Some(pos) = inner.tasks.iter().position(|t| t.id == task_id) else {
            return Err("task not found".into());
        };
        if inner.tasks[pos].status != TaskStatus::Draft {
            return Err("only draft tasks can be edited".into());
        }
        // A dependency edit must stay a valid DAG with real targets, or the
        // task (and anything downstream) would wait forever.
        if let Some(deps) = patch.depends_on.as_ref() {
            let ids: HashSet<&str> = inner.tasks.iter().map(|t| t.id.as_str()).collect();
            for d in deps {
                if d == &task_id {
                    return Err("a task cannot depend on itself".into());
                }
                if !ids.contains(d.as_str()) {
                    return Err(format!("unknown dependency: {d}"));
                }
            }
            let edges: Vec<(String, Vec<String>)> = inner
                .tasks
                .iter()
                .map(|t| {
                    if t.id == task_id {
                        (t.id.clone(), deps.clone())
                    } else {
                        (t.id.clone(), t.depends_on.clone())
                    }
                })
                .collect();
            if has_dependency_cycle(&edges) {
                return Err("that change would create a dependency cycle".into());
            }
        }
        apply_draft_patch(&mut inner.tasks[pos], patch)?;
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

/// How many deleted tasks to keep per project ("past X tasks").
const MAX_DELETED_PER_PROJECT: usize = 50;
/// Lines of the task log kept as its "output summary" on delete.
const LOG_TAIL_LINES: usize = 200;
/// Byte cap for the stored diff, so `state.json` stays small.
const MAX_STORED_DIFF: usize = 100_000;

/// The last `lines` lines of a log, with a trailing newline. Keeps the useful
/// end of the output while dropping the bulk.
fn log_tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    let mut out = all[start..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Render a diff as a compact text block, capped at `cap` bytes.
fn render_diff(d: &DiffResult, cap: usize) -> String {
    let mut out = String::new();
    if !d.stat.trim().is_empty() {
        out.push_str(d.stat.trim_end());
        out.push('\n');
    }
    for f in &d.files {
        out.push_str(&format!("\n### {} ({})\n", f.path, f.status));
        out.push_str(&f.diff);
        if !f.diff.ends_with('\n') {
            out.push('\n');
        }
        if out.len() >= cap {
            break;
        }
    }
    if out.len() > cap {
        let mut end = cap;
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("\n… (diff truncated)\n");
    }
    out
}

/// Make a deleted task's transient run state inert: no live ask, not `running`,
/// and any queued/running review abandoned.
fn abandon_run_state(t: &mut Task, at: i64) {
    t.ask = None;
    if !t.status.is_terminal() {
        t.status = TaskStatus::Canceled;
        t.finished_at.get_or_insert(at);
    }
    if let Some(r) = t.review.as_mut() {
        if matches!(r.status, ReviewStatus::Pending | ReviewStatus::Running) {
            r.status = ReviewStatus::Failed;
            r.summary = Some("abandoned when the task was deleted".into());
            r.finished_at = Some(at);
        }
    }
}

/// Keep only the newest `cap` deleted tasks for a project.
fn prune_deleted(deleted: &mut Vec<DeletedTask>, project: &str, cap: usize) {
    let mut indices: Vec<usize> = deleted
        .iter()
        .enumerate()
        .filter(|(_, d)| d.task.project_path == project)
        .map(|(i, _)| i)
        .collect();
    if indices.len() <= cap {
        return;
    }
    indices.sort_by_key(|&i| deleted[i].deleted_at);
    let excess = indices.len() - cap;
    let drop: HashSet<usize> = indices.into_iter().take(excess).collect();
    let mut i = 0usize;
    deleted.retain(|_| {
        let keep = !drop.contains(&i);
        i += 1;
        keep
    });
}

/// Move a task into the deleted list: drop it from the active list, repair the
/// survivors' dependencies, and cap the project's history. Pure, so the policy
/// can be unit-tested. Returns the project path when a task was archived.
fn archive_task(
    tasks: &mut Vec<Task>,
    deleted: &mut Vec<DeletedTask>,
    task_id: &str,
    summary: Option<String>,
    diff: Option<String>,
    cap: usize,
    at: i64,
) -> Option<String> {
    let pos = tasks.iter().position(|t| t.id == task_id)?;
    let mut task = tasks.remove(pos);
    abandon_run_state(&mut task, at);
    for t in tasks.iter_mut() {
        t.depends_on.retain(|d| d != task_id);
    }
    let project = task.project_path.clone();
    deleted.retain(|d| d.task.id != task_id);
    deleted.push(DeletedTask {
        task,
        deleted_at: at,
        summary,
        diff,
    });
    prune_deleted(deleted, &project, cap);
    Some(project)
}

/// Move a deleted task back into the active list. Returns the kept log tail so
/// the caller can restore it to the log file. Pure.
fn restore_task_state(
    tasks: &mut Vec<Task>,
    deleted: &mut Vec<DeletedTask>,
    task_id: &str,
) -> Option<Option<String>> {
    let pos = deleted.iter().position(|d| d.task.id == task_id)?;
    let mut dt = deleted.remove(pos);
    // A restored task has no live process, so it cannot still be running.
    if dt.task.status == TaskStatus::Running {
        dt.task.status = TaskStatus::Interrupted;
    }
    let summary = dt.summary.take();
    tasks.push(dt.task);
    Some(summary)
}

/// Ask any live child process for this task to terminate. Cancel and delete both
/// use it so a stopped task never keeps running in the background.
fn kill_task_children(st: &AppState, task_id: &str) {
    for map in [&st.running, &st.reviewing, &st.merging] {
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = guard.get_mut(task_id) {
            let _ = child.start_kill();
        }
    }
}

#[tauri::command]
pub async fn delete_task(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    // Read the task before its worktree and log go away.
    let task = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.tasks.iter().find(|t| t.id == task_id).cloned()
    };
    let Some(task) = task else {
        return Err("task not found".into());
    };

    // Keep the tail of the log as the task's output summary.
    let summary = {
        let text = std::fs::read_to_string(st.log_path(&task_id)).unwrap_or_default();
        let tail = log_tail(&text, LOG_TAIL_LINES);
        (!tail.trim().is_empty()).then_some(tail)
    };
    // Capture the diff before the worktree is removed.
    let diff = {
        let dir = task
            .worktree_path
            .clone()
            .filter(|w| Path::new(w).exists())
            .unwrap_or_else(|| task.project_path.clone());
        git::diff(Path::new(&dir))
            .await
            .ok()
            .map(|d| render_diff(&d, MAX_STORED_DIFF))
            .filter(|s| !s.trim().is_empty())
    };

    if let Some(w) = task.worktree_path.as_deref() {
        let _ = git::worktree_remove(Path::new(&task.project_path), Path::new(w)).await;
    }
    if let Some(b) = task.branch.as_deref() {
        let _ = git::branch_delete(Path::new(&task.project_path), b).await;
    }
    // Most of the log is dropped; the tail captured above is what is kept.
    let _ = std::fs::remove_file(st.log_path(&task_id));

    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        // Split the borrow out of the guard so the two lists can be passed mutably.
        let state = &mut *inner;
        archive_task(
            &mut state.tasks,
            &mut state.deleted_tasks,
            &task_id,
            summary,
            diff,
            MAX_DELETED_PER_PROJECT,
            now(),
        );
    }
    kill_task_children(&st, &task_id);
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn restore_task(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let summary = {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        // Split the borrow out of the guard so the two lists can be passed mutably.
        let state = &mut *inner;
        restore_task_state(&mut state.tasks, &mut state.deleted_tasks, &task_id)
            .ok_or_else(|| "no deleted task with that id".to_string())?
    };
    // Put the kept log tail back so the restored task's history is visible.
    if let Some(summary) = summary {
        let body = format!("[Solayge] restored from a deleted task\n{summary}");
        let _ = std::fs::write(st.log_path(&task_id), body);
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

/// Mark a non-terminal task cancelled, forgetting any pending ask: the run is
/// being torn down, so the question it was waiting on can no longer be answered.
/// Returns whether the task changed. Pure, so the rule can be unit-tested.
fn cancel_task_state(t: &mut Task) -> bool {
    if t.status.is_terminal() {
        return false;
    }
    t.status = TaskStatus::Canceled;
    t.finished_at = Some(now());
    t.ask = None;
    true
}

#[tauri::command]
pub fn cancel_task(app: AppHandle, task_id: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    {
        let mut inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = inner.tasks.iter_mut().find(|t| t.id == task_id) {
            cancel_task_state(t);
        }
    }
    kill_task_children(&st, &task_id);
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
    let (ask, running) = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        match inner.tasks.iter().find(|t| t.id == task_id) {
            Some(t) => (t.ask.clone(), t.can_answer_ask()),
            None => (None, false),
        }
    };
    let Some(ask) = ask else {
        return Err("this task has no pending question".into());
    };
    // Refuse to reply once the task has stopped: the ask's session is gone (or
    // belongs to a run that no longer exists), so the reply would go nowhere.
    if !running {
        return Err("this task is no longer waiting for an answer".into());
    }
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

/// Whether "Clear finished" may remove this task. Finished successes, failures,
/// and user-cancelled tasks go; `blocked` (paused for you) and `interrupted`
/// (retryable) are kept, as is a success whose review has not settled.
fn is_clearable(t: &Task) -> bool {
    match t.status {
        TaskStatus::Succeeded => t
            .review
            .as_ref()
            .is_none_or(|r| !matches!(r.status, ReviewStatus::Pending | ReviewStatus::Running)),
        TaskStatus::Failed | TaskStatus::Canceled => true,
        _ => false,
    }
}

/// Drop cleared ids from the survivors' dependency lists, so a task that only
/// waited on a cleared task becomes runnable instead of waiting forever on an
/// id that no longer exists.
fn rewire_after_removal(tasks: &mut [Task], removed: &HashSet<String>) {
    for t in tasks.iter_mut() {
        t.depends_on.retain(|d| !removed.contains(d));
    }
}

#[tauri::command]
pub async fn clear_finished(app: AppHandle, project_path: String) -> Result<Snapshot, String> {
    let st = app.state::<AppState>();
    let ids: Vec<(String, Option<String>, Option<String>)> = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .tasks
            .iter()
            .filter(|t| t.project_path == project_path && is_clearable(t))
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
        rewire_after_removal(&mut inner.tasks, &removed);
    }
    st.save();
    emit_state(&app);
    Ok(crate::state::snapshot(&app))
}

#[tauri::command]
pub fn get_task_log(app: AppHandle, task_id: String) -> Result<String, String> {
    if !valid_task_id(&task_id) {
        return Err("invalid task id".into());
    }
    let st = app.state::<AppState>();
    std::fs::read_to_string(st.log_path(&task_id)).map_err(|e| e.to_string())
}

/// The error-only log, for the Settings viewer.
#[tauri::command]
pub fn get_error_log(app: AppHandle) -> String {
    let st = app.state::<AppState>();
    crate::errorlog::read(&st.logs_dir())
}

#[tauri::command]
pub fn clear_error_log(app: AppHandle) -> Result<(), String> {
    let st = app.state::<AppState>();
    crate::errorlog::clear(&st.logs_dir()).map_err(|e| e.to_string())
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
    let (provider, model, templates, skills) = {
        let inner = st.inner.lock().unwrap_or_else(|e| e.into_inner());
        let proj = inner.projects.iter().find(|p| p.path == project_path);
        let r = agent::resolve(proj, &inner.settings);
        let skills = proj.map(|p| p.skills.clone()).unwrap_or_default();
        (
            r.provider,
            r.model,
            inner.settings.command_templates.clone(),
            skills,
        )
    };
    let env = st.project_env(&project_path);
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
    // Reachable from the webview, so this must not be a general "open any path
    // or application" primitive. Only http(s) URLs go to the OS handler; the
    // scoped opener plugin handles files and reveals.
    if !agent::is_web_url(&target) {
        return Err("only http(s) URLs can be opened".into());
    }
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

    // Persist secret values first, and without holding the state lock: a
    // keychain or file write can block. Only once every value is stored do we
    // mutate the project, so a store failure cannot leave it half-updated.
    let mut env_fields: Option<Vec<ProjectEnvVar>> = None;
    if let Some(vars) = config.env_vars.as_ref() {
        let (kind, previous) = {
            let inner = crate::state::lock(&st.inner);
            let kind = secrets::StoreKind::parse(inner.settings.secret_store.as_deref());
            let previous = inner
                .projects
                .iter()
                .find(|p| p.path == path)
                .map(|p| p.env_vars.iter().map(|e| e.key.clone()).collect::<Vec<_>>())
                .unwrap_or_default();
            (kind, previous)
        };
        let store = secrets::Secrets::new(&st.data_dir, kind);

        // Validate every name before writing any of them.
        let mut cleaned: Vec<(String, String, bool)> = Vec::with_capacity(vars.len());
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
            cleaned.push((key, v.value.clone(), v.secret));
        }
        for (key, value, _) in &cleaned {
            store.set(&path, key, value).map_err(|e| e.to_string())?;
        }
        // Drop values for variables that were removed or renamed.
        for old in &previous {
            if !cleaned.iter().any(|(key, _, _)| key == old) {
                let _ = store.delete(&path, old);
            }
        }
        env_fields = Some(
            cleaned
                .into_iter()
                .map(|(key, _, secret)| ProjectEnvVar { key, secret })
                .collect(),
        );
    }

    {
        let mut inner = crate::state::lock(&st.inner);
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

            if let Some(vars) = env_fields {
                p.env_vars = vars;
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
    if !valid_task_id(&task_id) {
        return String::new();
    }
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
    let providers = Provider::ALL
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
    use super::{
        apply_draft_patch, archive_task, cancel_task_state, has_dependency_cycle, is_clearable,
        log_tail, render_diff, reset_for_rerun, restore_task_state, rewire_after_removal,
        valid_task_id,
    };
    use crate::models::{
        BranchMode, DeletedTask, DiffResult, FileDiff, GitOp, Isolation, PermissionProfile,
        ReviewMode, ReviewStatus, Task, TaskAsk, TaskKind, TaskPatch, TaskReview, TaskStatus,
    };
    use std::collections::HashSet;

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

    fn pending_ask() -> TaskAsk {
        serde_json::from_value(serde_json::json!({
            "id": "frm_1",
            "kind": "question",
            "title": "Which environment?",
            "session_id": "ses_1"
        }))
        .unwrap()
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
        // A leftover ask from the interrupted run must not survive the reset.
        t.ask = Some(pending_ask());
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
        assert!(t.ask.is_none(), "the old run's ask must be forgotten");
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

    #[test]
    fn cancelling_a_running_task_clears_its_pending_ask() {
        let mut t = task(TaskStatus::Running, Isolation::Worktree);
        t.ask = Some(pending_ask());

        assert!(cancel_task_state(&mut t));

        assert_eq!(t.status, TaskStatus::Canceled);
        assert!(t.finished_at.is_some());
        assert!(t.ask.is_none(), "a cancelled run has no answerable ask");
    }

    #[test]
    fn cancelling_a_terminal_task_is_a_no_op() {
        let mut t = task(TaskStatus::Interrupted, Isolation::Worktree);
        t.error = Some("Interrupted: ...".into());

        assert!(!cancel_task_state(&mut t));

        assert_eq!(t.status, TaskStatus::Interrupted);
        assert_eq!(t.error.as_deref(), Some("Interrupted: ..."));
    }

    fn patch(title: Option<&str>, prompt: Option<&str>) -> TaskPatch {
        TaskPatch {
            title: title.map(str::to_string),
            prompt: prompt.map(str::to_string),
            isolation: None,
            profile: None,
            base_ref: None,
            delay_seconds: None,
            depends_on: None,
            command: None,
            branch_mode: None,
            new_branch: None,
            kind: None,
        }
    }

    #[test]
    fn a_draft_task_can_be_edited() {
        let mut t = task(TaskStatus::Draft, Isolation::Worktree);
        t.title = "old title".into();
        t.prompt = "old prompt".into();

        apply_draft_patch(&mut t, patch(Some("new title"), Some("new prompt"))).unwrap();

        assert_eq!(t.title, "new title");
        assert_eq!(t.prompt, "new prompt");
    }

    #[test]
    fn a_draft_patch_updates_every_creation_parameter() {
        // Editing a draft must cover everything the New Task form can set.
        let mut t = task(TaskStatus::Draft, Isolation::Worktree);
        t.not_before = None;

        apply_draft_patch(
            &mut t,
            TaskPatch {
                title: Some("new title".into()),
                prompt: Some("new prompt".into()),
                isolation: Some(Isolation::Shared),
                profile: Some(PermissionProfile::Readonly),
                base_ref: Some("main".into()),
                delay_seconds: Some(0),
                depends_on: Some(vec!["other".into()]),
                command: Some("echo hi".into()),
                branch_mode: Some(BranchMode::New),
                new_branch: Some("feat/x".into()),
                kind: Some(TaskKind::Shell),
            },
        )
        .unwrap();

        assert_eq!(t.title, "new title");
        assert_eq!(t.prompt, "new prompt");
        assert_eq!(t.isolation, Isolation::Shared);
        assert_eq!(t.profile, PermissionProfile::Readonly);
        assert_eq!(t.base_ref.as_deref(), Some("main"));
        assert!(t.not_before.is_none(), "delay 0 clears the hold");
        assert_eq!(t.depends_on, vec!["other".to_string()]);
        assert_eq!(t.command.as_deref(), Some("echo hi"));
        assert_eq!(t.branch_mode, BranchMode::New);
        assert_eq!(t.new_branch.as_deref(), Some("feat/x"));
    }

    #[test]
    fn a_positive_delay_becomes_a_future_hold() {
        let mut t = task(TaskStatus::Draft, Isolation::Worktree);
        let mut p = patch(None, None);
        p.delay_seconds = Some(120);
        apply_draft_patch(&mut t, p).unwrap();
        assert!(
            t.not_before.is_some_and(|nb| nb > super::now()),
            "a positive delay holds the task in the future"
        );
    }

    #[test]
    fn switching_to_shell_keeps_the_command_and_clears_git_config() {
        let mut t = task(TaskStatus::Draft, Isolation::Worktree);
        t.kind = TaskKind::Git;
        t.git_op = Some(GitOp::Push);
        let mut p = patch(None, None);
        p.kind = Some(TaskKind::Shell);
        p.command = Some("npm test".into());

        apply_draft_patch(&mut t, p).unwrap();

        assert_eq!(t.kind, TaskKind::Shell);
        assert_eq!(t.command.as_deref(), Some("npm test"));
        assert!(t.git_op.is_none(), "git config must not linger on a shell task");
    }

    #[test]
    fn switching_to_agent_clears_the_old_command() {
        let mut t = task(TaskStatus::Draft, Isolation::Worktree);
        t.kind = TaskKind::Shell;
        t.command = Some("echo hi".into());
        let mut p = patch(None, None);
        p.kind = Some(TaskKind::Agent);

        apply_draft_patch(&mut t, p).unwrap();

        assert_eq!(t.kind, TaskKind::Agent);
        assert!(t.command.is_none(), "a stale shell command must be dropped");
    }

    #[test]
    fn a_released_or_running_task_cannot_be_edited() {
        // Once a task is queued or has run, its prompt is the instruction the
        // agent was given; rewriting it must be rejected, not silently applied.
        for status in [
            TaskStatus::Waiting,
            TaskStatus::Ready,
            TaskStatus::Running,
            TaskStatus::Succeeded,
            TaskStatus::Interrupted,
        ] {
            let mut t = task(status, Isolation::Worktree);
            t.prompt = "original".into();

            let err = apply_draft_patch(&mut t, patch(None, Some("rewritten"))).unwrap_err();

            assert!(err.contains("draft"), "{status:?}: {err}");
            assert_eq!(t.prompt, "original", "{status:?} prompt must not change");
        }
    }

    // ---- clear finished ----

    #[test]
    fn clearable_is_finished_but_not_paused_or_retryable() {
        let clear = |s| is_clearable(&task(s, Isolation::Worktree));
        assert!(clear(TaskStatus::Succeeded));
        assert!(clear(TaskStatus::Failed));
        assert!(clear(TaskStatus::Canceled));
        // Paused (awaiting you) and retryable tasks are not "finished".
        assert!(!clear(TaskStatus::Blocked));
        assert!(!clear(TaskStatus::Interrupted));
        assert!(!clear(TaskStatus::Running));
        assert!(!clear(TaskStatus::Waiting));
        assert!(!clear(TaskStatus::Ready));
        assert!(!clear(TaskStatus::Draft));
    }

    #[test]
    fn a_success_with_an_unsettled_review_is_not_clearable() {
        let mut t = task(TaskStatus::Succeeded, Isolation::Worktree);
        for status in [ReviewStatus::Pending, ReviewStatus::Running] {
            t.review = Some(TaskReview {
                mode: ReviewMode::Autofix,
                status,
                provider: None,
                model: None,
                summary: None,
                started_at: None,
                finished_at: None,
            });
            assert!(!is_clearable(&t), "{status:?} must hold the task");
        }
        t.review.as_mut().unwrap().status = ReviewStatus::Passed;
        assert!(is_clearable(&t));
    }

    #[test]
    fn clearing_rewires_dependents_onto_cleared_tasks() {
        let done = {
            let mut t = task(TaskStatus::Succeeded, Isolation::Worktree);
            t.id = "done".into();
            t
        };
        let mut child = task(TaskStatus::Waiting, Isolation::Worktree);
        child.id = "child".into();
        child.depends_on = vec!["done".into(), "other".into()];

        let mut tasks = vec![done, child];
        let removed: HashSet<String> = ["done".to_string()].into_iter().collect();
        tasks.retain(|t| !removed.contains(&t.id));
        rewire_after_removal(&mut tasks, &removed);

        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].depends_on,
            vec!["other".to_string()],
            "only the cleared id is dropped"
        );
    }

    // ---- soft delete / restore ----

    fn reviewed(t: &mut Task, status: ReviewStatus) {
        t.review = Some(TaskReview {
            mode: ReviewMode::Autofix,
            status,
            provider: None,
            model: None,
            summary: None,
            started_at: None,
            finished_at: None,
        });
    }

    #[test]
    fn log_tail_keeps_only_the_end() {
        assert_eq!(log_tail("a\nb\nc\nd", 2), "c\nd\n");
        assert_eq!(log_tail("only", 5), "only\n");
        assert_eq!(log_tail("", 5), "");
    }

    #[test]
    fn render_diff_caps_its_size() {
        let d = DiffResult {
            stat: " f | 1 +".into(),
            files: vec![FileDiff {
                path: "f".into(),
                status: "M".into(),
                diff: "x".repeat(500),
            }],
        };
        let small = render_diff(&d, 10_000);
        assert!(small.contains("### f (M)"));
        let capped = render_diff(&d, 50);
        assert!(capped.contains("(diff truncated)"));
        assert!(capped.len() <= 50 + "\n… (diff truncated)\n".len());
    }

    #[test]
    fn deleting_archives_and_rewires_instead_of_dropping() {
        let mut done = task(TaskStatus::Succeeded, Isolation::Worktree);
        done.id = "a".into();
        let mut child = task(TaskStatus::Waiting, Isolation::Worktree);
        child.id = "b".into();
        child.depends_on = vec!["a".into()];
        let mut tasks = vec![done, child];
        let mut deleted: Vec<DeletedTask> = Vec::new();

        let project = archive_task(
            &mut tasks,
            &mut deleted,
            "a",
            Some("tail".into()),
            Some("diff".into()),
            50,
            1234,
        )
        .expect("archived");

        assert_eq!(project, "/p");
        assert_eq!(tasks.len(), 1, "only the deleted task leaves the active list");
        assert!(tasks[0].depends_on.is_empty(), "dependents are rewired");
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].task.id, "a");
        assert_eq!(deleted[0].summary.as_deref(), Some("tail"));
        assert_eq!(deleted[0].diff.as_deref(), Some("diff"));
        assert_eq!(deleted[0].deleted_at, 1234);
    }

    #[test]
    fn deleting_a_running_task_makes_it_inert() {
        let mut t = task(TaskStatus::Running, Isolation::Worktree);
        t.id = "r".into();
        t.started_at = Some(1);
        t.ask = Some(pending_ask());
        reviewed(&mut t, ReviewStatus::Running);
        let mut tasks = vec![t];
        let mut deleted: Vec<DeletedTask> = Vec::new();

        archive_task(&mut tasks, &mut deleted, "r", None, None, 50, 9).expect("archived");

        let archived = &deleted[0].task;
        assert_eq!(archived.status, TaskStatus::Canceled);
        assert_eq!(archived.finished_at, Some(9));
        assert!(archived.ask.is_none(), "no live ask survives deletion");
        assert_eq!(
            archived.review.as_ref().unwrap().status,
            ReviewStatus::Failed,
            "an in-flight review is abandoned"
        );
    }

    #[test]
    fn deleted_history_is_capped_per_project_and_keeps_the_newest() {
        let mut deleted: Vec<DeletedTask> = Vec::new();
        for i in 0..60i64 {
            let mut t = task(TaskStatus::Succeeded, Isolation::Worktree);
            t.id = format!("t{i}");
            deleted.push(DeletedTask {
                task: t,
                deleted_at: i,
                summary: None,
                diff: None,
            });
        }
        // A task in another project must not count toward this project's cap.
        let mut other = task(TaskStatus::Succeeded, Isolation::Worktree);
        other.project_path = "/other".into();
        other.id = "other".into();
        deleted.push(DeletedTask {
            task: other,
            deleted_at: 0,
            summary: None,
            diff: None,
        });

        super::prune_deleted(&mut deleted, "/p", 50);

        let ours: Vec<i64> = deleted
            .iter()
            .filter(|d| d.task.project_path == "/p")
            .map(|d| d.deleted_at)
            .collect();
        assert_eq!(ours.len(), 50);
        assert!(ours.iter().all(|&at| at >= 10), "oldest are dropped");
        assert!(
            deleted.iter().any(|d| d.task.id == "other"),
            "another project's history is untouched"
        );
    }

    #[test]
    fn restoring_moves_the_task_back_and_never_leaves_it_running() {
        let mut running = task(TaskStatus::Running, Isolation::Worktree);
        running.id = "x".into();
        let mut tasks: Vec<Task> = Vec::new();
        let mut deleted = vec![DeletedTask {
            task: running,
            deleted_at: 5,
            summary: Some("tail".into()),
            diff: Some("diff".into()),
        }];

        let summary = restore_task_state(&mut tasks, &mut deleted, "x").expect("restored");

        assert_eq!(summary.as_deref(), Some("tail"));
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::Interrupted);
        assert!(deleted.is_empty());

        // Unknown id: nothing happens.
        assert!(restore_task_state(&mut tasks, &mut deleted, "missing").is_none());
        assert_eq!(tasks.len(), 1);
    }

    // ---- input validation ----

    #[test]
    fn task_ids_cannot_escape_the_logs_directory() {
        assert!(valid_task_id("6f1d0a2e-9b3c-4a1e-8f2d-0c7b5a4e3d21"));
        assert!(valid_task_id("t1"));
        assert!(valid_task_id("plan_task-2"));
        assert!(!valid_task_id(""));
        assert!(!valid_task_id("../../../../etc/passwd"));
        assert!(!valid_task_id("/etc/hosts"));
        assert!(!valid_task_id("a/b"));
        assert!(!valid_task_id("a b"));
        assert!(!valid_task_id("a.log"));
    }

    fn edges(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        pairs
            .iter()
            .map(|(id, deps)| (id.to_string(), deps.iter().map(|d| d.to_string()).collect()))
            .collect()
    }

    #[test]
    fn a_dependency_cycle_is_rejected() {
        assert!(!has_dependency_cycle(&edges(&[("a", &[]), ("b", &["a"])])));
        assert!(has_dependency_cycle(&edges(&[
            ("a", &["b"]),
            ("b", &["a"])
        ])));
        // A self-dependency is a cycle too.
        assert!(has_dependency_cycle(&edges(&[("a", &["a"])])));
        // Longer loops are found.
        assert!(has_dependency_cycle(&edges(&[
            ("a", &["b"]),
            ("b", &["c"]),
            ("c", &["a"]),
        ])));
        // A dangling dependency is not a cycle (it is handled separately).
        assert!(!has_dependency_cycle(&edges(&[("a", &["missing"])])));
    }
}
