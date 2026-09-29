//! Prompt history and log files are treated as a cache: they are capped, pruned
//! by a retention window, and can be cleared from Settings.

use std::time::UNIX_EPOCH;

use crate::models::{CacheStats, Isolation, PermissionProfile, PromptEntry};
use crate::scheduler::now;
use crate::state::AppState;

/// Hard cap on remembered prompts, independent of the retention window.
const MAX_PROMPTS: usize = 300;
/// Prompts can be long; keep the cache small.
const MAX_PROMPT_CHARS: usize = 8_000;
/// Suggest count returned to the UI by default.
const DEFAULT_SUGGEST: usize = 50;

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Remove prompt history and logs older than the retention window. A retention
/// of `0` means "keep forever" (only the hard cap still applies).
pub fn prune(st: &AppState) {
    let days = crate::state::lock(&st.inner).settings.cache_retention_days;
    if days <= 0 {
        return;
    }
    let cutoff = now() - days * 86_400;

    {
        let mut inner = crate::state::lock(&st.inner);
        inner.prompts.retain(|p| p.created_at >= cutoff);
        cap(&mut inner.prompts);
    }
    prune_logs(st, cutoff);
    st.save();
}

/// Drop the oldest prompts once the hard cap is exceeded.
fn cap(prompts: &mut Vec<PromptEntry>) {
    if prompts.len() <= MAX_PROMPTS {
        return;
    }
    prompts.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    prompts.truncate(MAX_PROMPTS);
}

fn prune_logs(st: &AppState, cutoff: i64) {
    let running: std::collections::HashSet<String> = crate::state::lock(&st.running)
        .keys()
        .cloned()
        .collect();
    let Ok(entries) = std::fs::read_dir(st.logs_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("log") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Never delete the log of a running task.
        if running.contains(id) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        if matches!(modified, Some(m) if m < cutoff) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Remember a prompt for later re-use. Identical prompts for the same project
/// are de-duplicated (their `uses` count and timestamp are bumped instead).
pub fn record_prompt(
    st: &AppState,
    project_path: Option<String>,
    title: String,
    prompt: String,
    profile: Option<PermissionProfile>,
    isolation: Option<Isolation>,
) {
    let text = truncate(prompt.trim(), MAX_PROMPT_CHARS);
    if text.is_empty() {
        return;
    }
    let mut inner = crate::state::lock(&st.inner);
    let existing = inner
        .prompts
        .iter_mut()
        .find(|p| p.prompt == text && p.project_path == project_path);
    match existing {
        Some(entry) => {
            entry.uses += 1;
            entry.created_at = now();
            entry.title = title;
        }
        None => inner.prompts.push(PromptEntry {
            id: uuid::Uuid::new_v4().to_string(),
            project_path,
            title,
            prompt: text,
            profile,
            isolation,
            created_at: now(),
            uses: 1,
        }),
    }
    cap(&mut inner.prompts);
    drop(inner);
    st.save();
}

/// Most recently used prompts (optionally for one project), newest first.
pub fn history(st: &AppState, project_path: Option<&str>, limit: Option<usize>) -> Vec<PromptEntry> {
    let inner = crate::state::lock(&st.inner);
    let mut out: Vec<PromptEntry> = inner
        .prompts
        .iter()
        .filter(|p| match project_path {
            Some(path) => p.project_path.as_deref() == Some(path),
            None => true,
        })
        .cloned()
        .collect();
    out.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    out.truncate(limit.unwrap_or(DEFAULT_SUGGEST));
    out
}

pub fn stats(st: &AppState) -> CacheStats {
    let inner = crate::state::lock(&st.inner);
    let (prompt_count, oldest_prompt, retention_days) = (
        inner.prompts.len(),
        inner.prompts.iter().map(|p| p.created_at).min(),
        inner.settings.cache_retention_days,
    );
    drop(inner);

    let mut log_count = 0usize;
    let mut log_bytes = 0u64;
    if let Ok(entries) = std::fs::read_dir(st.logs_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("log") {
                continue;
            }
            log_count += 1;
            if let Ok(md) = entry.metadata() {
                log_bytes += md.len();
            }
        }
    }

    CacheStats {
        prompt_count,
        oldest_prompt,
        log_count,
        log_bytes,
        retention_days,
    }
}

/// Clear selected parts of the cache and return the fresh stats.
pub fn clear(st: &AppState, prompts: bool, logs: bool) -> CacheStats {
    if prompts {
        crate::state::lock(&st.inner).prompts.clear();
        st.save();
    }
    if logs {
        let running: std::collections::HashSet<String> = crate::state::lock(&st.running)
            .keys()
            .cloned()
            .collect();
        if let Ok(entries) = std::fs::read_dir(st.logs_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("log") {
                    continue;
                }
                let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if running.contains(id) {
                    continue;
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    stats(st)
}
