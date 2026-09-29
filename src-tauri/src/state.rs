use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

use crate::models::{PersistedState, ReviewStatus, Snapshot, TaskStatus};

/// Unix seconds, without pulling `scheduler::now` (which depends on this module).
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub inner: Mutex<PersistedState>,
    pub running: Mutex<HashMap<String, tokio::process::Child>>,
    /// Auto code-review processes, keyed by task id.
    pub reviewing: Mutex<HashMap<String, tokio::process::Child>>,
    /// Multi-step integration (merge) processes, keyed by task id.
    pub merging: Mutex<HashMap<String, tokio::process::Child>>,
    /// Cached provider model lists, keyed by provider command key.
    pub models: Mutex<HashMap<String, Vec<String>>>,
    /// Last time each task showed signs of life (a live child, streamed output,
    /// or a managed step). Used to detect a run that silently orphaned.
    pub heartbeat: Mutex<HashMap<String, i64>>,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Self {
        let mut state = load(&data_dir);
        let migrated = migrate_legacy(&data_dir, &mut state);
        // Nothing can be running yet: any task persisted as `running` (and any
        // review persisted as queued/running) belongs to a previous session that
        // stopped without finalizing it. Reconcile those before we serve state.
        let recovered = recover_interrupted(&data_dir, &mut state);
        if migrated || recovered {
            let _ = save(&data_dir, &state);
        }
        let st = AppState {
            data_dir,
            inner: Mutex::new(state),
            running: Mutex::new(HashMap::new()),
            reviewing: Mutex::new(HashMap::new()),
            merging: Mutex::new(HashMap::new()),
            models: Mutex::new(HashMap::new()),
            heartbeat: Mutex::new(HashMap::new()),
        };
        // Drop cache that is already past its retention window.
        crate::cache::prune(&st);
        st
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn log_path(&self, id: &str) -> PathBuf {
        self.logs_dir().join(format!("{id}.log"))
    }

    pub fn review_log_path(&self, id: &str) -> PathBuf {
        self.logs_dir().join(format!("review-{id}.log"))
    }

    pub fn save(&self) {
        // `std::sync::Mutex` is not reentrant. Wait briefly for a concurrent
        // writer to finish, but never block forever if *this* thread already
        // holds `inner` (a bug): in that case give up with a warning rather than
        // hanging the whole app. The next mutation's save persists the change.
        for _ in 0..50 {
            match self.inner.try_lock() {
                Ok(inner) => {
                    if let Err(e) = save(&self.data_dir, &inner) {
                        // A full disk or a bad path must not fail silently.
                        eprintln!("[Solayge] failed to save state.json: {e}");
                    }
                    return;
                }
                Err(std::sync::TryLockError::Poisoned(e)) => {
                    let inner = e.into_inner();
                    if let Err(e) = save(&self.data_dir, &inner) {
                        eprintln!("[Solayge] failed to save state.json: {e}");
                    }
                    return;
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        }
        eprintln!(
            "[Solayge] save() could not acquire the state lock; skipping this save \
             (a caller is probably holding it)"
        );
    }
}

pub fn load(data_dir: &Path) -> PersistedState {
    let p = data_dir.join("state.json");
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(data_dir: &Path, state: &PersistedState) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let p = data_dir.join("state.json");
    let tmp = data_dir.join("state.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(tmp, p)
}

/// Bundle identifiers this app used previously. On first launch with an empty
/// current state, the most recent one that has data is imported (projects,
/// tasks, permission defaults, and logs).
const LEGACY_IDENTIFIERS: &[&str] = &["com.solayge.app", "com.devtools.orchestrator"];

/// If the current state is empty, import data from a previous identifier.
/// Returns whether anything was migrated.
fn migrate_legacy(data_dir: &Path, state: &mut PersistedState) -> bool {
    if !state.projects.is_empty() || !state.tasks.is_empty() {
        return false;
    }
    let Some(parent) = data_dir.parent() else {
        return false;
    };

    for identifier in LEGACY_IDENTIFIERS {
        let legacy = parent.join(identifier);
        if legacy == data_dir {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(legacy.join("state.json")) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<PersistedState>(&text) else {
            continue;
        };
        if parsed.projects.is_empty() && parsed.tasks.is_empty() {
            continue;
        }
        *state = parsed;

        // Move log files across so existing tasks keep their output.
        let src = legacy.join("logs");
        let dst = data_dir.join("logs");
        if src.is_dir() {
            let _ = std::fs::create_dir_all(&dst);
            if let Ok(entries) = std::fs::read_dir(&src) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let _ = std::fs::copy(&path, dst.join(entry.file_name()));
                    }
                }
            }
        }

        // Preserve file-store secrets. Keychain/DPAPI items are keyed by
        // service + account (not the identifier), so they carry over on their own.
        for name in ["secrets.json", "secrets.key", "secrets.win.json"] {
            let from = legacy.join(name);
            let to = data_dir.join(name);
            if from.is_file() && !to.exists() {
                let _ = std::fs::create_dir_all(data_dir);
                let _ = std::fs::copy(&from, &to);
            }
        }

        return true;
    }
    false
}

/// Reconcile state left behind by a session that stopped mid-flight. Returns
/// whether anything changed.
///
/// A `running` task is moved to `interrupted` with an explanation, because at
/// startup no child process exists for it — the previous session crashed, was
/// force-quit, or ran out of resources. A review still queued or running is put
/// back to `pending` so the scheduler resumes it (the worktree is still there).
/// A `[Solayge]` note is appended to each affected log so the reason is visible
/// in the task's own history.
fn recover_interrupted(data_dir: &Path, state: &mut PersistedState) -> bool {
    let reason = "Interrupted: Solayge stopped while this task was running (crash, \
                  forced quit, or the machine ran out of disk). The task may not have \
                  finished. Retry to run it again; its log above is from the interrupted \
                  attempt.";
    let mut changed = false;
    let mut notes: Vec<(String, String)> = Vec::new();

    for t in state.tasks.iter_mut() {
        if t.status == TaskStatus::Running {
            t.status = TaskStatus::Interrupted;
            // The last log write is the best estimate of when it actually died;
            // falling back to now would overstate the run's duration.
            let died = last_log_time(data_dir, &t.id)
                .or(t.started_at)
                .unwrap_or_else(now);
            t.finished_at = Some(died);
            t.error = Some(reason.to_string());
            changed = true;
            notes.push((format!("{}.log", t.id), reason.to_string()));
        }
    }
    for t in state.tasks.iter_mut() {
        if let Some(r) = t.review.as_mut() {
            if r.status == ReviewStatus::Running {
                // Re-queue so the scheduler runs (or re-runs) the review.
                r.status = ReviewStatus::Pending;
                changed = true;
            }
        }
    }

    if changed {
        for (file, note) in notes {
            append_log_note(data_dir, &file, &note);
        }
    }
    changed
}

/// When a log file was last written, as unix seconds. Best-effort.
fn last_log_time(data_dir: &Path, id: &str) -> Option<i64> {
    let path = data_dir.join("logs").join(format!("{id}.log"));
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// Append a `[Solayge]` line to a log file under `<data_dir>/logs`, best-effort.
fn append_log_note(data_dir: &Path, file: &str, message: &str) {
    use std::io::Write;
    let dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(file))
    {
        let _ = writeln!(f, "\n[Solayge] {message}");
    }
}

/// Lock a mutex, recovering from poisoning instead of panicking. A panic while a
/// lock was held must never take the whole scheduler down.
pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn snapshot(app: &AppHandle) -> Snapshot {
    let st = app.state::<AppState>();
    let inner = lock(&st.inner);
    let running = lock(&st.running).len();
    Snapshot {
        projects: inner.projects.clone(),
        tasks: inner.tasks.clone(),
        concurrency: inner.concurrency,
        running,
        settings: inner.settings.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_state_from_a_previous_identifier() {
        let root = std::env::temp_dir().join(format!("solayge-migrate-{}", uuid::Uuid::new_v4()));
        let legacy = root.join("com.solayge.app");
        let current = root.join("com.solayge.desktop");
        std::fs::create_dir_all(&legacy).unwrap();
        // Minimal JSON: everything else relies on serde defaults.
        let json = r#"{"projects":[{"path":"/p","name":"p","added_at":1}]}"#;
        std::fs::write(legacy.join("state.json"), json).unwrap();

        let mut loaded = PersistedState::default();
        assert!(migrate_legacy(&current, &mut loaded));
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].path, "/p");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn does_not_overwrite_existing_state() {
        let root = std::env::temp_dir().join(format!("solayge-migrate-{}", uuid::Uuid::new_v4()));
        let legacy = root.join("com.solayge.app");
        let current = root.join("com.solayge.desktop");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("state.json"),
            r#"{"projects":[{"path":"/p","name":"p","added_at":1}]}"#,
        )
        .unwrap();

        let mut loaded = PersistedState {
            projects: vec![crate::models::Project {
                path: "/existing".into(),
                name: "e".into(),
                added_at: 2,
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
            }],
            ..Default::default()
        };
        assert!(!migrate_legacy(&current, &mut loaded));
        assert_eq!(loaded.projects[0].path, "/existing");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_while_locked_does_not_deadlock() {
        let dir = std::env::temp_dir().join(format!("solayge-save-{}", uuid::Uuid::new_v4()));
        let st = AppState::new(dir.clone());
        {
            // Simulate the bug: call save() while already holding the state lock.
            // Before the try_lock safety net this hung the whole app forever.
            let _guard = lock(&st.inner);
            st.save();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_recovers_from_a_poisoned_mutex() {
        let dir = std::env::temp_dir().join(format!("solayge-poison-{}", uuid::Uuid::new_v4()));
        let st = std::sync::Arc::new(AppState::new(dir.clone()));
        {
            let mut inner = lock(&st.inner);
            inner.concurrency = 9;
        }
        let st2 = st.clone();
        // A thread that panics while holding the lock poisons it.
        let panicked = std::thread::spawn(move || {
            let _guard = lock(&st2.inner);
            panic!("intentional panic to poison the state lock");
        })
        .join();
        assert!(panicked.is_err(), "the helper thread should have panicked");

        // `lock` must recover instead of propagating the poison, and the data
        // written before the panic must still be there.
        let inner = lock(&st.inner);
        assert_eq!(inner.concurrency, 9);
        drop(inner);
        // Saving through a poisoned lock must also work.
        st.save();
        assert_eq!(load(&dir).concurrency, 9);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_mutations_and_saves_do_not_deadlock_or_lose_data() {
        use crate::models::PromptEntry;
        let dir = std::env::temp_dir().join(format!("solayge-conc-{}", uuid::Uuid::new_v4()));
        let st = std::sync::Arc::new(AppState::new(dir.clone()));
        let threads = 8usize;
        let per_thread = 25usize;

        let handles: Vec<_> = (0..threads)
            .map(|i| {
                let st = st.clone();
                std::thread::spawn(move || {
                    for j in 0..per_thread {
                        {
                            let mut inner = lock(&st.inner);
                            inner.prompts.push(PromptEntry {
                                id: format!("{i}-{j}"),
                                project_path: None,
                                title: "t".into(),
                                prompt: "p".into(),
                                profile: None,
                                isolation: None,
                                created_at: 1,
                                uses: 1,
                            });
                        }
                        st.save();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("worker thread must not panic (no deadlock)");
        }
        // A final save after quiescence; with the bounded-retry save, no
        // mutation should have been lost.
        st.save();
        assert_eq!(load(&dir).prompts.len(), threads * per_thread);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("solayge-rt-{}", uuid::Uuid::new_v4()));
        let st = AppState::new(dir.clone());
        {
            let mut inner = lock(&st.inner);
            inner.concurrency = 5;
        }
        st.save();
        let loaded = load(&dir);
        assert_eq!(loaded.concurrency, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovers_running_tasks_and_queued_reviews() {
        let root = std::env::temp_dir().join(format!("solayge-recover-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut state: PersistedState = serde_json::from_str(
            r#"{"tasks":[
                {"id":"t1","project_path":"/p","title":"one","prompt":"p","status":"running","created_at":1,
                 "review":{"mode":"pause","status":"pending"}},
                {"id":"t2","project_path":"/p","title":"two","prompt":"p","status":"succeeded","created_at":2,
                 "review":{"mode":"report","status":"running"}}
            ]}"#,
        )
        .unwrap();

        assert!(recover_interrupted(&root, &mut state));
        assert_eq!(state.tasks[0].status, TaskStatus::Interrupted);
        assert!(state.tasks[0].error.as_deref().unwrap().contains("Interrupted"));
        assert_eq!(
            state.tasks[1].review.as_ref().unwrap().status,
            ReviewStatus::Pending
        );

        // The reason is written into the interrupted task's own log.
        let log = std::fs::read_to_string(root.join("logs").join("t1.log")).unwrap();
        assert!(log.contains("Interrupted"));

        // Idempotent: a second pass changes nothing.
        assert!(!recover_interrupted(&root, &mut state));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recovery_touches_only_running_and_running_reviews() {
        let root = std::env::temp_dir().join(format!("solayge-recover2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut state: PersistedState = serde_json::from_str(
            r#"{"tasks":[
                {"id":"draft","project_path":"/p","title":"a","prompt":"p","status":"draft","created_at":1},
                {"id":"wait","project_path":"/p","title":"b","prompt":"p","status":"waiting","created_at":2},
                {"id":"ready","project_path":"/p","title":"c","prompt":"p","status":"ready","created_at":3},
                {"id":"done","project_path":"/p","title":"d","prompt":"p","status":"succeeded","created_at":4,
                 "review":{"mode":"report","status":"passed"}},
                {"id":"bad","project_path":"/p","title":"e","prompt":"p","status":"failed","created_at":5},
                {"id":"cancel","project_path":"/p","title":"f","prompt":"p","status":"canceled","created_at":6},
                {"id":"block","project_path":"/p","title":"g","prompt":"p","status":"blocked","created_at":7,
                 "review":{"mode":"pause","status":"issues"}}
            ]}"#,
        )
        .unwrap();
        let before: Vec<TaskStatus> = state.tasks.iter().map(|t| t.status).collect();

        assert!(!recover_interrupted(&root, &mut state));
        let after: Vec<TaskStatus> = state.tasks.iter().map(|t| t.status).collect();
        assert_eq!(before, after);
        // Terminal review verdicts must be left alone.
        assert_eq!(
            state.tasks[3].review.as_ref().unwrap().status,
            ReviewStatus::Passed
        );
        assert_eq!(
            state.tasks[6].review.as_ref().unwrap().status,
            ReviewStatus::Issues
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
