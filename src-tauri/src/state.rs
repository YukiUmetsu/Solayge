use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

use crate::models::{PersistedState, Snapshot};

/// Bundle identifier this app used before it was renamed to Solayge. Its data
/// dir is imported once so previously added projects/tasks survive the rename.
const LEGACY_IDENTIFIER: &str = "com.devtools.orchestrator";

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
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Self {
        let mut state = load(&data_dir);
        if migrate_legacy(&data_dir, &mut state) {
            let _ = save(&data_dir, &state);
        }
        let st = AppState {
            data_dir,
            inner: Mutex::new(state),
            running: Mutex::new(HashMap::new()),
            reviewing: Mutex::new(HashMap::new()),
            merging: Mutex::new(HashMap::new()),
            models: Mutex::new(HashMap::new()),
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
        if let Ok(inner) = self.inner.lock() {
            let _ = save(&self.data_dir, &inner);
        }
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

/// If the current state is empty, import the pre-rename app data (state +
/// logs). Returns whether anything was migrated.
fn migrate_legacy(data_dir: &Path, state: &mut PersistedState) -> bool {
    if !state.projects.is_empty() || !state.tasks.is_empty() {
        return false;
    }
    let Some(parent) = data_dir.parent() else {
        return false;
    };
    let legacy = parent.join(LEGACY_IDENTIFIER);
    let legacy_state = legacy.join("state.json");
    let Ok(text) = std::fs::read_to_string(&legacy_state) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<PersistedState>(&text) else {
        return false;
    };
    if parsed.projects.is_empty() && parsed.tasks.is_empty() {
        return false;
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
    true
}

pub fn snapshot(app: &AppHandle) -> Snapshot {
    let st = app.state::<AppState>();
    let inner = st.inner.lock().expect("state lock");
    let running = st.running.lock().expect("running lock").len();
    Snapshot {
        projects: inner.projects.clone(),
        tasks: inner.tasks.clone(),
        concurrency: inner.concurrency,
        running,
        settings: inner.settings.clone(),
    }
}
