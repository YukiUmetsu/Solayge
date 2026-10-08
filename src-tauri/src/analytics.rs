//! Local-first analytics for task/run lifecycle, model usage, tokens, and
//! provider-reported cost.
//!
//! Design
//! ------
//! * Solayge owns task state; `analytics.json` only projects it. [`sync_runs`]
//!   derives one run record per `(task, attempt)` from the current tasks on each
//!   scheduler tick, so dispatch, finalize, interruption, cancel, and retry are
//!   all captured without hooking each transition individually. Upserts never
//!   downgrade a terminal run back to non-terminal, so a rerun that resets a
//!   task before its attempt counter increments cannot erase the previous run.
//! * Usage (tokens + provider-reported cost) comes from finalized opencode
//!   assistant messages only, keyed deterministically by
//!   `source:session:message`. Repeated `message.updated` polls therefore upsert
//!   the same row instead of summing partial updates.
//! * Nothing here may slow down or fail a task: collection is best-effort and
//!   errors are logged, never propagated to the scheduler.
//! * Missing data is `None`, never `0`. A provider that does not report usage
//!   (every provider except opencode today) is reported as *unavailable* with
//!   coverage, not as zero.
//!
//! The cost number opencode reports is a model-price calculation, not a bill;
//! the API keeps it as `reported_cost_micros` and the UI labels it accordingly.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Schema version for the analytics store. Bump when the shape changes and add a
/// migration; the loader tolerates unknown/absent fields via serde defaults.
pub const ANALYTICS_VERSION: u32 = 2;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn store_version() -> u32 {
    ANALYTICS_VERSION
}

// ---------------------------------------------------------------------------
// Persisted store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsStore {
    #[serde(default = "store_version")]
    pub version: u32,
    #[serde(default)]
    pub runs: Vec<RunRecord>,
    #[serde(default)]
    pub usages: Vec<UsageRecord>,
    /// Categorized, fingerprinted error events (Phase 2). Failure *occurrences*,
    /// deliberately kept distinct from task failure.
    #[serde(default)]
    pub errors: Vec<ErrorRecord>,
    /// Structured human-interaction events (Phase 3): questions, permission
    /// prompts, and manual confirmations. Never stores prompt/question text.
    #[serde(default)]
    pub interactions: Vec<InteractionRecord>,
}

impl Default for AnalyticsStore {
    fn default() -> Self {
        AnalyticsStore {
            version: ANALYTICS_VERSION,
            runs: Vec::new(),
            usages: Vec::new(),
            errors: Vec::new(),
            interactions: Vec::new(),
        }
    }
}

/// One execution attempt of a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// `{task_id}:{attempt}` — deterministic and stable across restarts.
    pub run_id: String,
    pub task_id: String,
    pub project_path: String,
    pub attempt: u32,
    /// `agent` | `shell` | `git` | `merge`.
    pub kind: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `running` | `succeeded` | `failed` | `canceled` | `blocked` | `interrupted`.
    pub status: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub started_at_ms: Option<i64>,
    #[serde(default)]
    pub ended_at_ms: Option<i64>,
    /// Wall-clock active time (`ended - started`) for a finalized run. Paused and
    /// human-waiting time is not separated yet, so this is an upper bound.
    #[serde(default)]
    pub active_ms: Option<i64>,
    /// `partial` until the run is finalized; `none` for non-agent kinds that
    /// never report usage.
    pub telemetry_quality: String,
}

/// One model invocation's normalized usage. Duplicate polls of the same message
/// update the same row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// `{source}:{session_id}:{message_id}`.
    pub usage_id: String,
    pub source: String,
    pub session_id: String,
    pub message_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub provider_total_tokens: Option<u64>,
    /// Provider-reported cost in integer micro-USD (1e-6 USD), so sums stay exact.
    #[serde(default)]
    pub reported_cost_micros: Option<i64>,
    /// `complete` when both tokens and cost were reported, otherwise `partial`.
    pub telemetry_quality: String,
}

impl UsageRecord {
    /// The normalized total: trust a provider-supplied total when present,
    /// otherwise add the reported categories. `None` means unknown, not zero.
    pub fn total_tokens(&self) -> Option<u64> {
        if let Some(t) = self.provider_total_tokens {
            return Some(t);
        }
        let parts = [
            self.input_tokens,
            self.output_tokens,
            self.reasoning_tokens,
            self.cache_read_tokens,
            self.cache_write_tokens,
        ];
        if parts.iter().all(Option::is_none) {
            return None;
        }
        Some(parts.iter().flatten().sum())
    }
}

/// One categorized, fingerprinted error occurrence. Repeated occurrences never
/// imply task failure: this is deliberately a separate axis from outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorRecord {
    /// Deterministic and stable across restarts.
    pub error_id: String,
    /// `opencode` (runtime tool/message error) or `solayge` (task outcome).
    pub source: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// One of the taxonomy in [`error_category`].
    pub category: String,
    /// Deterministic grouping key; never raw text.
    pub fingerprint: String,
    /// The lifecycle stage the error occurred in (`run`, `review`, `tool`, …).
    pub stage: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    /// A redacted, length-bounded sample. Only stored when the user opts in.
    #[serde(default)]
    pub summary_redacted: Option<String>,
    pub occurred_at_ms: i64,
    /// `true`/`false`/`null(unknown)` is derived at query time from the run
    /// outcome, not persisted, so a late-successful attempt cannot be missed.
    #[serde(default)]
    pub recovered: Option<bool>,
}

/// A structured human-interaction event: a question, permission request, or
/// manual confirmation. Text is never stored; only classification and timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionRecord {
    /// `{task_id}:{ask_id}` — deterministic and stable across restarts.
    pub interaction_id: String,
    /// `structured` (a real provider event) or `inferred` (derived).
    pub source_confidence: String,
    pub source: String,
    pub task_id: String,
    pub project_path: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// `question` | `permission` | `manual_confirmation`.
    pub kind: String,
    /// A [`question_category`] value.
    pub category: String,
    /// Deterministic grouping key; never raw text.
    pub fingerprint: String,
    pub asked_at_ms: i64,
    #[serde(default)]
    pub resolved_at_ms: Option<i64>,
    /// Whether the run paused for a human. Structured questions/permissions do.
    #[serde(default)]
    pub blocking: bool,
    /// `answered` | `rejected` | `canceled`, once resolved.
    #[serde(default)]
    pub outcome: Option<String>,
}

// ---------------------------------------------------------------------------
// Classification, fingerprinting, and redaction
// ---------------------------------------------------------------------------

/// Known OS/network errno-style codes worth recognizing on their own.
const KNOWN_CODES: [&str; 14] = [
    "ENOENT",
    "EACCES",
    "EPERM",
    "ETIMEDOUT",
    "ECONNREFUSED",
    "ECONNRESET",
    "ENOTFOUND",
    "EEXIST",
    "ENOSPC",
    "EMFILE",
    "EADDRINUSE",
    "SIGKILL",
    "SIGTERM",
    "SIGSEGV",
];

fn is_secret_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.starts_with("sk_")
        || lower.starts_with("ghp_")
        || lower.starts_with("gho_")
        || lower.starts_with("xoxb-")
        || lower.starts_with("akia")
        || lower.contains("bearer")
}

/// Whether a whitespace-delimited token looks like a filesystem path on any
/// supported platform: a Unix path, a Windows drive path (`C:\…`, `C:/…`), a
/// UNC share (`\\server\share`), a root-relative Windows path (`\Users\…`), or
/// a traversal-relative path (`../…`, `..\…`). Coverage here directly bounds
/// what can leak into a redacted error sample, so all supported platforms are
/// handled, not just Unix.
fn is_absolute_path_token(token: &str) -> bool {
    if token.starts_with('/')
        || token.starts_with('~')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with(".\\")
        || token.starts_with("..\\")
    {
        return true;
    }
    // Root-relative and UNC paths: a leading backslash with an embedded
    // separator, so a regex fragment like `\d+` is not mistaken for a path.
    if token.starts_with('\\') && token.matches('\\').count() >= 2 {
        return true;
    }
    let bytes = token.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Best-effort redaction before persistence. Removes absolute paths, URLs,
/// long hex/uuid identifiers, long digit runs, and obvious secret tokens. This
/// runs at ingestion so nothing sensitive is written, not just at render time.
pub fn redact_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        let t = token.trim_end_matches([',', ';', '.']);
        let trailing = &token[t.len()..];
        let replacement: String = if is_secret_token(t) {
            "<secret>".to_string()
        } else if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www.") {
            "<url>".to_string()
        } else if is_absolute_path_token(t) {
            "<path>".to_string()
        } else if t.len() >= 7 && t.chars().all(|c| c.is_ascii_digit()) {
            "<num>".to_string()
        } else if t.len() >= 16
            && t.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
            && t.chars().any(|c| c.is_ascii_digit())
        {
            "<id>".to_string()
        } else {
            t.to_string()
        };
        out.push_str(&replacement);
        out.push_str(trailing);
        token.clear();
    };
    for ch in input.chars() {
        if ch.is_whitespace() {
            flush(&mut token, &mut out);
            out.push(' ');
        } else {
            token.push(ch);
        }
    }
    flush(&mut token, &mut out);
    out.trim().to_string()
}

/// Collapse whitespace and clip, after redaction. Never returns raw paths.
pub fn sanitize_summary(input: &str, max: usize) -> String {
    let redacted = redact_text(input);
    let collapsed: String = redacted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let mut out: String = collapsed.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Extract the first stable error code from text: `TS2322`, `E0425`, an
/// errno, or an HTTP status. Returns `None` when no clear signal exists, so an
/// uncertain error is grouped as `unknown` rather than guessed.
pub fn error_code_token(text: &str) -> Option<String> {
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if raw.len() < 3 || raw.len() > 32 {
            continue;
        }
        let upper = raw.to_ascii_uppercase();
        if KNOWN_CODES.contains(&upper.as_str()) {
            return Some(upper);
        }
        let has_digit = raw.chars().any(|c| c.is_ascii_digit());
        let has_alpha = raw.chars().any(|c| c.is_ascii_alphabetic());
        // A bare 3-digit HTTP status.
        if !has_alpha {
            if let Ok(n) = raw.parse::<u32>() {
                if (100..=599).contains(&n) {
                    return Some(upper);
                }
            }
            continue;
        }
        // Compiler/runtime codes like TS2322, E0425, P1001, or 429 in a label.
        if has_digit && upper.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            return Some(upper);
        }
    }
    None
}

/// The top-level error taxonomy. Deterministic and conservative: anything
/// unclear is `unknown` rather than a guessed precise cause.
pub fn error_category(text: &str, tool: Option<&str>, kind: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);

    if has("rate limit")
        || has("429")
        || has("quota")
        || has("insufficient")
        || has("context length")
        || has("context window")
        || has("too long")
        || has("overloaded")
        || has("authentication")
        || has("unauthorized")
        || has("401")
        || has("403")
        || has("api key")
        || has("model not found")
        || has("timed out")
        || has("timeout")
        || has("econnreset")
        || has("connection reset")
        || has("temporarily unavailable")
    {
        return "provider";
    }
    if has("permission denied")
        || has("not permitted")
        || has("blocked by policy")
        || has("eacces")
        || has("read-only file system")
    {
        return "security_policy";
    }
    if has("assertion")
        || has("assert_eq")
        || has("test failed")
        || has("failing test")
        || has("tests failed")
        || has("pytest")
        || has("jest")
        || has("vitest")
        || has("cargo test")
    {
        return "test";
    }
    if has("enoent") || has("command not found") || has("no such file") {
        return "tool";
    }
    if has("error[e")
        || has("cannot find")
        || has("compilation")
        || has("compile error")
        || has("ts2")
        || has("tsc ")
        || has("build failed")
        || has("bundler")
        || has("esbuild")
        || has("webpack")
        || has("undefined reference")
    {
        return "build";
    }
    if kind == "git" || kind == "merge" || has("merge conflict") || has("worktree") {
        return "integration";
    }
    if has("panicked") || has("session crash") || has("stuck") || has("invalid state") {
        return "agent_runtime";
    }
    if tool.is_some() || has("exit status") || has("exit code") || has("non-zero") {
        return "tool";
    }
    "unknown"
}

/// A deterministic, text-free grouping key. The category and an extracted code
/// (or `unknown`) form the fingerprint; tool errors keep the tool name so
/// distinct tools do not collapse together.
pub fn error_fingerprint(category: &str, text: &str, tool: Option<&str>) -> String {
    let code = error_code_token(text).unwrap_or_else(|| "unknown".to_string());
    match (category, tool) {
        ("tool", Some(tool)) => format!("tool:{tool}:{code}"),
        _ => format!("{category}:{code}"),
    }
}

/// The human-readable code segment of a fingerprint.
pub fn fingerprint_code(fingerprint: &str) -> String {
    fingerprint
        .rsplit(':')
        .next()
        .unwrap_or(fingerprint)
        .to_string()
}

/// Classify a question or permission into the interaction taxonomy. Reads text
/// only to choose a category; the text itself is never stored.
pub fn question_category(title: &str, message: Option<&str>, fields: &[String], kind: &str) -> &'static str {
    if kind == "permission" {
        let all = format!("{} {}", title, message.unwrap_or_default()).to_ascii_lowercase();
        if ["credential", "secret", "password", "api key", "token", "login", "auth"]
            .iter()
            .any(|n| all.contains(n))
        {
            return "credentials";
        }
        return "permissions";
    }
    let mut all = format!("{} {}", title, message.unwrap_or_default());
    for f in fields {
        all.push(' ');
        all.push_str(f);
    }
    let lower = all.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);
    if has("credential")
        || has("secret")
        || has("password")
        || has("api key")
        || has("token")
        || has("login")
        || has("authenticate")
    {
        return "credentials";
    }
    if has("approve") || has("deploy") || has("release") || has("publish") || has("push") || has("merge") {
        return "approval_deployment";
    }
    if has("install") || has("setup") || has("environment") || has("dependency") || has("version") {
        return "environment_setup";
    }
    if has("convention") || has("instruction") || has("commands") || has("build command") || has("test command") {
        return "missing_repository_instructions";
    }
    if has("requirement") || has("spec") || has("what should") || has("scope") || has("feature") {
        return "missing_requirements";
    }
    if has("implement") || has("approach") || has("library") || has("framework") || has("design") || has("database") || has("how should") {
        return "implementation_choice";
    }
    if has("success") || has("acceptance") || has("criteria") || has("done") || has("verify") {
        return "ambiguous_success_criteria";
    }
    "other"
}

/// A structural fingerprint for a question ask: sorted field keys, or the
/// permission action. Keeps recurring questions grouped without storing text.
pub fn interaction_fingerprint(kind: &str, action: Option<&str>, field_keys: &[String]) -> String {
    if kind == "permission" {
        return format!("permission:{}", action.unwrap_or("unknown"));
    }
    if field_keys.is_empty() {
        return "question:generic".to_string();
    }
    let mut keys: Vec<String> = field_keys.iter().map(|k| k.to_ascii_lowercase()).collect();
    keys.sort();
    keys.dedup();
    format!("question:{}", keys.join("+"))
}

// ---------------------------------------------------------------------------
// Loading / saving
// ---------------------------------------------------------------------------

fn store_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("analytics.json")
}

pub fn load_store(data_dir: &Path) -> AnalyticsStore {
    let path = store_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            crate::errorlog::append(
                &data_dir.join("logs"),
                "analytics",
                &format!(
                    "could not parse {}: {e}; starting empty (the file was left untouched)",
                    path.display()
                ),
            );
            AnalyticsStore::default()
        }),
        Err(_) => AnalyticsStore::default(),
    }
}

/// Write the store atomically (temp + rename), so a crash cannot truncate it.
/// The file is owner-only on Unix, matching the app's other private data.
pub fn save_store(data_dir: &Path, store: &AnalyticsStore) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = store_path(data_dir);
    let tmp = data_dir.join("analytics.json.tmp");
    let bytes = serde_json::to_vec(store)?;
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.flush()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, &bytes)?;
    }
    std::fs::rename(tmp, path)
}

fn persist(app: &AppHandle, store: &AnalyticsStore) {
    let st = app.state::<AppState>();
    if let Err(e) = save_store(&st.data_dir, store) {
        crate::errorlog::record(app, "analytics", &format!("failed to save analytics: {e}"));
    }
}

/// Force any in-memory analytics to disk. Called on app exit.
pub fn flush(app: &AppHandle) {
    let st = app.state::<AppState>();
    let store = crate::state::lock(&st.analytics);
    if let Err(e) = save_store(&st.data_dir, &store) {
        crate::errorlog::record(app, "analytics", &format!("failed to save analytics: {e}"));
    }
}

fn analytics_enabled(app: &AppHandle) -> bool {
    let st = app.state::<AppState>();
    let inner = crate::state::lock(&st.inner);
    inner.settings.analytics_enabled
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

fn kind_str(kind: crate::models::TaskKind) -> &'static str {
    use crate::models::TaskKind;
    match kind {
        TaskKind::Agent => "agent",
        TaskKind::Shell => "shell",
        TaskKind::Git => "git",
        TaskKind::Merge => "merge",
    }
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        "succeeded" | "failed" | "canceled" | "blocked" | "interrupted"
    )
}

/// Project the current tasks into run records. Only tasks that have been
/// dispatched at least once (`attempts >= 1`) become runs.
fn run_from_task(t: &crate::models::Task) -> Option<RunRecord> {
    if t.attempts == 0 {
        return None;
    }
    let status = status_str(t.status);
    let started_at_ms = t.started_at.map(|s| s.saturating_mul(1000));
    let ended_at_ms = if is_terminal_status(status) {
        t.finished_at.map(|s| s.saturating_mul(1000))
    } else {
        None
    };
    let active_ms = match (started_at_ms, ended_at_ms) {
        (Some(s), Some(e)) if e >= s => Some(e - s),
        _ => None,
    };
    Some(RunRecord {
        run_id: format!("{}:{}", t.id, t.attempts),
        task_id: t.id.clone(),
        project_path: t.project_path.clone(),
        attempt: t.attempts,
        kind: kind_str(t.kind).to_string(),
        provider: t.provider.map(|p| p.command_key().to_string()),
        model: t.model.clone(),
        status: status.to_string(),
        created_at_ms: t.created_at.saturating_mul(1000),
        started_at_ms,
        ended_at_ms,
        active_ms,
        telemetry_quality: if t.kind == crate::models::TaskKind::Agent
            && t.provider == Some(crate::models::Provider::Opencode)
        {
            "partial".to_string()
        } else {
            "none".to_string()
        },
    })
}

fn status_str(status: crate::models::TaskStatus) -> &'static str {
    use crate::models::TaskStatus as S;
    match status {
        S::Draft => "draft",
        S::Waiting => "waiting",
        S::Ready => "ready",
        S::Running => "running",
        S::Succeeded => "succeeded",
        S::Failed => "failed",
        S::Canceled => "canceled",
        S::Blocked => "blocked",
        S::Interrupted => "interrupted",
    }
}

/// Upsert run records. Returns whether the store changed.
///
/// A terminal record is never downgraded (its `ended_at_ms` and status are
/// kept), which protects an attempt's outcome from a rerun that has reset the
/// task but not yet bumped `attempts`.
fn upsert_runs(store: &mut AnalyticsStore, records: Vec<RunRecord>) -> bool {
    let mut index: HashMap<String, usize> = store
        .runs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.run_id.clone(), i))
        .collect();
    let mut changed = false;
    for record in records {
        match index.get(&record.run_id).copied() {
            Some(i) => {
                let existing = &store.runs[i];
                if is_terminal_status(&existing.status) && !is_terminal_status(&record.status) {
                    continue; // never erase a settled attempt
                }
                if *existing != record {
                    store.runs[i] = record;
                    changed = true;
                }
            }
            None => {
                index.insert(record.run_id.clone(), store.runs.len());
                store.runs.push(record);
                changed = true;
            }
        }
    }
    changed
}

/// Project tasks into runs and errors, and reconcile open interactions. Called
/// once per tick. Idempotent, so restarts and reruns never double-count.
pub fn sync_runs(app: &AppHandle) {
    if !analytics_enabled(app) {
        return;
    }
    let capture = capture_error_examples(app);
    let st = app.state::<AppState>();
    let (records, task_errors, tasks): (
        Vec<RunRecord>,
        Vec<ErrorRecord>,
        HashMap<String, Option<i64>>,
    ) = {
        let inner = crate::state::lock(&st.inner);
        let records = inner.tasks.iter().filter_map(run_from_task).collect();
        let task_errors = inner
            .tasks
            .iter()
            .filter_map(|t| errors_from_task(capture, t))
            .collect();
        // Terminal tasks close any interaction still marked open; the timestamp
        // is when the task finished. Active tasks stay open; a task that is no
        // longer present is treated as gone.
        let tasks = inner
            .tasks
            .iter()
            .map(|t| {
                let ended = t.status.is_terminal().then(|| {
                    t.finished_at
                        .map(|s| s.saturating_mul(1000))
                        .unwrap_or_else(now_ms)
                });
                (t.id.clone(), ended)
            })
            .collect();
        (records, task_errors, tasks)
    };
    let mut store = crate::state::lock(&st.analytics);
    let mut changed = upsert_runs(&mut store, records);
    changed |= upsert_errors(&mut store, task_errors);
    changed |= reconcile_interactions_locked(&mut store, &tasks, now_ms());
    if changed {
        persist(app, &store);
    }
}

/// Extract usage from one opencode message. Pure so the wire shape can be
/// tested against real payloads.
pub fn usage_from_opencode_message(
    task_id: Option<&str>,
    project_path: Option<&str>,
    run_id: Option<&str>,
    session: &str,
    message: &Value,
) -> Option<UsageRecord> {
    if message.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message_id = message.get("id").and_then(Value::as_str)?.to_string();
    if message_id.is_empty() {
        return None;
    }
    let model = message.get("model");
    let provider_id = model
        .and_then(|m| m.get("providerID").or_else(|| m.get("provider_id")))
        .and_then(Value::as_str)
        .map(str::to_string);
    let model_id = model
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let variant = model
        .and_then(|m| m.get("variant"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let tokens = message.get("tokens");
    let num = |v: Option<&Value>| v.and_then(Value::as_u64);
    let input_tokens = num(tokens.and_then(|t| t.get("input")));
    let output_tokens = num(tokens.and_then(|t| t.get("output")));
    let reasoning_tokens = num(tokens.and_then(|t| t.get("reasoning")));
    let cache_read_tokens = num(tokens.and_then(|t| t.get("cache")).and_then(|c| c.get("read")));
    let cache_write_tokens = num(tokens.and_then(|t| t.get("cache")).and_then(|c| c.get("write")));
    let provider_total_tokens = num(tokens.and_then(|t| t.get("total")));

    let reported_cost_micros = message.get("cost").and_then(|c| {
        c.as_f64()
            .or_else(|| c.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
            .map(|usd| (usd * 1_000_000.0).round() as i64)
    });

    let has_tokens = input_tokens.is_some()
        || output_tokens.is_some()
        || reasoning_tokens.is_some()
        || cache_read_tokens.is_some()
        || cache_write_tokens.is_some()
        || provider_total_tokens.is_some();
    if !has_tokens && reported_cost_micros.is_none() {
        return None;
    }

    let occurred_at_ms = message
        .get("time")
        .and_then(|t| t.get("completed").or_else(|| t.get("created")))
        .and_then(Value::as_i64)
        .filter(|v| *v > 0)
        .map(|v| if v < 1_000_000_000_000 { v * 1000 } else { v })
        .unwrap_or_else(now_ms);

    let telemetry_quality = if has_tokens && reported_cost_micros.is_some() {
        "complete"
    } else {
        "partial"
    };

    Some(UsageRecord {
        usage_id: format!("opencode:{session}:{message_id}"),
        source: "opencode".to_string(),
        session_id: session.to_string(),
        message_id,
        task_id: task_id.map(str::to_string),
        project_path: project_path.map(str::to_string),
        run_id: run_id.map(str::to_string),
        occurred_at_ms,
        provider_id,
        model_id,
        variant,
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        cache_write_tokens,
        provider_total_tokens,
        reported_cost_micros,
        telemetry_quality: telemetry_quality.to_string(),
    })
}

/// Capture usage from every completed assistant message in a session transcript.
/// Idempotent: repeated polls upsert the same rows. Non-blocking and silent on
/// parse problems.
pub fn record_opencode_messages(
    app: &AppHandle,
    task_id: &str,
    project_path: &str,
    run_id: &str,
    session: &str,
    messages: &[Value],
) {
    if !analytics_enabled(app) {
        return;
    }
    let capture = capture_error_examples(app);
    // Cheap guard: nothing to capture when the transcript has no assistant
    // message with an id, so an idle-only poll does not build the index.
    if !messages.iter().any(|m| {
        m.get("type").and_then(Value::as_str) == Some("assistant") && m.get("id").is_some()
    }) {
        return;
    }
    let st = app.state::<AppState>();
    let mut store = crate::state::lock(&st.analytics);
    let mut index: HashMap<String, usize> = store
        .usages
        .iter()
        .enumerate()
        .map(|(i, u)| (u.usage_id.clone(), i))
        .collect();
    let mut error_index: HashMap<String, usize> = store
        .errors
        .iter()
        .enumerate()
        .map(|(i, e)| (e.error_id.clone(), i))
        .collect();
    let mut changed = false;
    for message in messages {
        if let Some(record) = usage_from_opencode_message(
            Some(task_id),
            Some(project_path),
            Some(run_id),
            session,
            message,
        ) {
            match index.get(&record.usage_id).copied() {
                Some(i) => {
                    if store.usages[i] != record {
                        store.usages[i] = record;
                        changed = true;
                    }
                }
                None => {
                    index.insert(record.usage_id.clone(), store.usages.len());
                    store.usages.push(record);
                    changed = true;
                }
            }
        }
        for record in errors_from_opencode_message(
            capture,
            Some(task_id),
            Some(project_path),
            Some(run_id),
            session,
            message,
        ) {
            match error_index.get(&record.error_id).copied() {
                Some(i) => {
                    if store.errors[i] != record {
                        store.errors[i] = record;
                        changed = true;
                    }
                }
                None => {
                    error_index.insert(record.error_id.clone(), store.errors.len());
                    store.errors.push(record);
                    changed = true;
                }
            }
        }
    }
    if changed {
        persist(app, &store);
    }
}

fn capture_error_examples(app: &AppHandle) -> bool {
    let st = app.state::<AppState>();
    let inner = crate::state::lock(&st.inner);
    inner.settings.analytics_capture_error_examples
}

/// Occurrence time for a message, matching [`usage_from_opencode_message`].
fn message_occurred_at_ms(message: &Value) -> i64 {
    message
        .get("time")
        .and_then(|t| t.get("completed").or_else(|| t.get("created")))
        .and_then(Value::as_i64)
        .filter(|v| *v > 0)
        .map(|v| if v < 1_000_000_000_000 { v * 1000 } else { v })
        .unwrap_or_else(now_ms)
}

/// Flatten the many shapes an opencode error can take into redactable text.
fn json_error_text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        if !s.trim().is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(o) = v.as_object() {
        for key in ["message", "error", "reason", "name", "data"] {
            if let Some(t) = o.get(key).and_then(json_error_text) {
                return Some(t);
            }
        }
    }
    if let Some(a) = v.as_array() {
        let parts: Vec<String> = a.iter().filter_map(json_error_text).collect();
        if !parts.is_empty() {
            return Some(parts.join("; "));
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn build_error(
    capture_examples: bool,
    error_id: String,
    source: &str,
    task_id: Option<&str>,
    project_path: Option<&str>,
    run_id: Option<&str>,
    session_id: Option<&str>,
    kind: &str,
    stage: &str,
    tool_name: Option<String>,
    text: &str,
    occurred_at_ms: i64,
) -> ErrorRecord {
    let category = error_category(text, tool_name.as_deref(), kind);
    let fingerprint = error_fingerprint(category, text, tool_name.as_deref());
    ErrorRecord {
        error_id,
        source: source.to_string(),
        task_id: task_id.map(str::to_string),
        project_path: project_path.map(str::to_string),
        run_id: run_id.map(str::to_string),
        session_id: session_id.map(str::to_string),
        category: category.to_string(),
        fingerprint,
        stage: stage.to_string(),
        tool_name,
        summary_redacted: if capture_examples {
            Some(sanitize_summary(text, 240))
        } else {
            None
        },
        occurred_at_ms,
        recovered: None,
    }
}

/// Extract categorized error occurrences from one opencode assistant message:
/// a whole-message `error`, and any `tool` part that ended in `state.status:
/// "error"`. Repeated polls produce the same deterministic ids, so upserts are
/// idempotent. `capture_examples` (off by default) controls whether a redacted
/// sample is persisted.
pub fn errors_from_opencode_message(
    capture_examples: bool,
    task_id: Option<&str>,
    project_path: Option<&str>,
    run_id: Option<&str>,
    session: &str,
    message: &Value,
) -> Vec<ErrorRecord> {
    if message.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let Some(message_id) = message.get("id").and_then(Value::as_str) else {
        return Vec::new();
    };
    if message_id.is_empty() {
        return Vec::new();
    }
    let occurred_at_ms = message_occurred_at_ms(message);
    let mut out = Vec::new();
    if let Some(text) = message.get("error").and_then(json_error_text) {
        out.push(build_error(
            capture_examples,
            format!("opencode:{session}:{message_id}:message"),
            "opencode",
            task_id,
            project_path,
            run_id,
            Some(session),
            "agent",
            "run",
            None,
            &text,
            occurred_at_ms,
        ));
    }
    if let Some(content) = message.get("content").and_then(Value::as_array) {
        for (i, part) in content.iter().enumerate() {
            if part.get("type").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            let state = part.get("state");
            if state.and_then(|s| s.get("status")).and_then(Value::as_str) != Some("error") {
                continue;
            }
            let tool = part
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let text = state
                .and_then(|s| s.get("error").or_else(|| s.get("output")))
                .and_then(json_error_text)
                .unwrap_or_else(|| "tool error".to_string());
            out.push(build_error(
                capture_examples,
                format!("opencode:{session}:{message_id}:tool:{i}"),
                "opencode",
                task_id,
                project_path,
                run_id,
                Some(session),
                "agent",
                "tool",
                tool,
                &text,
                occurred_at_ms,
            ));
        }
    }
    out
}

/// A Solayge-owned failure: a terminal task that recorded an error. This gives
/// reliable coverage for shell/git/merge tasks and any provider, without parsing
/// terminal stdout.
pub fn errors_from_task(
    capture_examples: bool,
    t: &crate::models::Task,
) -> Option<ErrorRecord> {
    if !is_terminal_status(status_str(t.status)) {
        return None;
    }
    let text = t
        .error
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let kind = kind_str(t.kind);
    let run_id = format!("{}:{}", t.id, t.attempts);
    let occurred = t
        .finished_at
        .or(t.started_at)
        .map(|s| s.saturating_mul(1000))
        .unwrap_or_else(|| t.created_at.saturating_mul(1000));
    Some(build_error(
        capture_examples,
        format!("task:{}:{}", t.id, t.attempts),
        "solayge",
        Some(&t.id),
        Some(&t.project_path),
        Some(&run_id),
        t.session_id.as_deref(),
        kind,
        "run",
        None,
        text,
        occurred,
    ))
}

/// Upsert error records by deterministic id. Unlike runs, errors are additive
/// and never removed here; retention handles aging out. Returns whether the
/// store changed.
pub fn upsert_errors(store: &mut AnalyticsStore, records: Vec<ErrorRecord>) -> bool {
    let mut index: HashMap<String, usize> = store
        .errors
        .iter()
        .enumerate()
        .map(|(i, e)| (e.error_id.clone(), i))
        .collect();
    let mut changed = false;
    for record in records {
        match index.get(&record.error_id).copied() {
            Some(i) => {
                if store.errors[i] != record {
                    store.errors[i] = record;
                    changed = true;
                }
            }
            None => {
                index.insert(record.error_id.clone(), store.errors.len());
                store.errors.push(record);
                changed = true;
            }
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Interaction collection (Phase 3)
// ---------------------------------------------------------------------------

/// Record that an agent surfaced a question or permission request. Idempotent:
/// re-surfacing the same ask keeps its original `asked_at_ms` and never clears a
/// resolution.
pub fn record_ask_surfaced(app: &AppHandle, task_id: &str, ask: &crate::models::TaskAsk) {
    if !analytics_enabled(app) {
        return;
    }
    let st = app.state::<AppState>();
    let (project_path, run_id) = {
        let inner = crate::state::lock(&st.inner);
        match inner.tasks.iter().find(|t| t.id == task_id) {
            Some(t) => (
                t.project_path.clone(),
                format!("{}:{}", t.id, t.attempts),
            ),
            None => return,
        }
    };
    let kind = match ask.kind {
        crate::models::AskKind::Question => "question",
        crate::models::AskKind::Permission => "permission",
    };
    let field_labels: Vec<String> = ask.fields.iter().map(|f| f.label.clone()).collect();
    let field_keys: Vec<String> = ask.fields.iter().map(|f| f.key.clone()).collect();
    let category =
        question_category(&ask.title, ask.message.as_deref(), &field_labels, kind);
    let fingerprint = interaction_fingerprint(kind, ask.action.as_deref(), &field_keys);
    let record = InteractionRecord {
        interaction_id: format!("{task_id}:{}", ask.id),
        source_confidence: "structured".to_string(),
        source: "opencode".to_string(),
        task_id: task_id.to_string(),
        project_path,
        run_id: Some(run_id),
        session_id: ask.session_id.clone(),
        kind: kind.to_string(),
        category: category.to_string(),
        fingerprint,
        asked_at_ms: now_ms(),
        resolved_at_ms: None,
        blocking: true,
        outcome: None,
    };
    let mut store = crate::state::lock(&st.analytics);
    match store
        .interactions
        .iter_mut()
        .find(|i| i.interaction_id == record.interaction_id)
    {
        Some(existing) => {
            // Keep the original ask time and any resolution already recorded.
            if existing.resolved_at_ms.is_none() {
                let asked = existing.asked_at_ms;
                let resolved = existing.resolved_at_ms;
                let outcome = existing.outcome.clone();
                *existing = record;
                existing.asked_at_ms = asked;
                existing.resolved_at_ms = resolved;
                existing.outcome = outcome;
            }
        }
        None => store.interactions.push(record),
    }
    persist(app, &store);
}

/// Mark a surfaced interaction resolved. Called when the user answers, when the
/// runtime rejects, or when the task ends with the ask unanswered.
pub fn record_ask_resolved(app: &AppHandle, task_id: &str, ask_id: &str, outcome: &str) {
    let st = app.state::<AppState>();
    let id = format!("{task_id}:{ask_id}");
    let mut store = crate::state::lock(&st.analytics);
    let mut changed = false;
    if let Some(record) = store.interactions.iter_mut().find(|i| i.interaction_id == id) {
        if record.resolved_at_ms.is_none() {
            record.resolved_at_ms = Some(now_ms());
            record.outcome = Some(outcome.to_string());
            changed = true;
        }
    }
    if changed {
        persist(app, &store);
    }
}

/// Close interactions left open by a task that has since ended or disappeared.
/// `tasks` maps every current task id to its terminal timestamp (`None` while it
/// is still active). An active task keeps its interaction open; an ended task,
/// or one no longer present, closes it as `canceled`. Keeps restart drift from
/// showing a permanently "waiting" question.
fn reconcile_interactions_locked(
    store: &mut AnalyticsStore,
    tasks: &HashMap<String, Option<i64>>,
    now: i64,
) -> bool {
    let mut changed = false;
    for record in store.interactions.iter_mut() {
        if record.resolved_at_ms.is_some() {
            continue;
        }
        let resolved = match tasks.get(&record.task_id) {
            Some(Some(at)) => Some(*at),
            Some(None) => None, // still active: leave open
            None => Some(now),  // task gone: close it
        };
        let Some(resolved) = resolved else {
            continue;
        };
        record.resolved_at_ms = Some(resolved);
        record.outcome = Some("canceled".to_string());
        changed = true;
    }
    changed
}

/// Drop records older than `days` (0 keeps forever). Returns whether anything
/// was removed. Pure, so it can run at startup before an `AppHandle` exists.
pub fn retain_recent(store: &mut AnalyticsStore, days: i64, now_ms: i64) -> bool {
    if days <= 0 {
        return false;
    }
    let cutoff = now_ms.saturating_sub(days.saturating_mul(86_400_000));
    let runs_before = store.runs.len();
    let usages_before = store.usages.len();
    let errors_before = store.errors.len();
    let interactions_before = store.interactions.len();
    store.runs.retain(|r| {
        r.ended_at_ms
            .or(r.started_at_ms)
            .map(|t| t >= cutoff)
            .unwrap_or(true) // a still-running run has no timestamp; keep it
    });
    store.usages.retain(|u| u.occurred_at_ms >= cutoff);
    store.errors.retain(|e| e.occurred_at_ms >= cutoff);
    store.interactions.retain(|i| {
        i.resolved_at_ms.or(Some(i.asked_at_ms)).unwrap_or(i.asked_at_ms) >= cutoff
    });
    store.runs.len() != runs_before
        || store.usages.len() != usages_before
        || store.errors.len() != errors_before
        || store.interactions.len() != interactions_before
}

/// Prune against the configured retention and persist any change.
pub fn prune(app: &AppHandle) {
    let st = app.state::<AppState>();
    let days = {
        let inner = crate::state::lock(&st.inner);
        inner.settings.analytics_retention_days
    };
    let mut store = crate::state::lock(&st.analytics);
    if retain_recent(&mut store, days, now_ms()) {
        persist(app, &store);
    }
}

/// Clear all collected analytics. Task execution is untouched.
pub fn clear(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut store = crate::state::lock(&st.analytics);
    store.runs.clear();
    store.usages.clear();
    store.errors.clear();
    store.interactions.clear();
    persist(app, &store);
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalyticsFilters {
    /// Inclusive lower bound, unix milliseconds.
    #[serde(default)]
    pub from_ms: Option<i64>,
    /// Exclusive upper bound, unix milliseconds.
    #[serde(default)]
    pub to_ms: Option<i64>,
    /// Restrict to these project paths; empty means all.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Restrict usage to these `provider/model`, `model`, or `provider` ids;
    /// empty means all.
    #[serde(default)]
    pub models: Vec<String>,
}

fn in_window(from: Option<i64>, to: Option<i64>, at: i64) -> bool {
    from.is_none_or(|f| at >= f) && to.is_none_or(|t| at < t)
}

fn project_ok(path: Option<&str>, filters: &AnalyticsFilters) -> bool {
    if filters.projects.is_empty() {
        return true;
    }
    path.is_some_and(|p| filters.projects.iter().any(|x| x == p))
}

fn model_ok(provider: Option<&str>, model: Option<&str>, filters: &AnalyticsFilters) -> bool {
    if filters.models.is_empty() {
        return true;
    }
    let provider = provider.unwrap_or_default();
    let model = model.unwrap_or_default();
    filters.models.iter().any(|f| {
        f == model
            || f == &format!("{provider}/{model}")
            || (!provider.is_empty() && f == provider)
    })
}

fn run_in_filters(r: &RunRecord, filters: &AnalyticsFilters) -> bool {
    // Runs are outcome-cohort records: a finalized run is placed by its end, an
    // in-flight run by its start.
    let at = r.ended_at_ms.or(r.started_at_ms).unwrap_or(r.created_at_ms);
    in_window(filters.from_ms, filters.to_ms, at) && project_ok(Some(&r.project_path), filters)
}

/// Task ids that had at least one usage matching the model filter within range.
/// `None` means "no model filter", i.e. every task is allowed.
fn model_allowed_tasks(
    store: &AnalyticsStore,
    filters: &AnalyticsFilters,
) -> Option<HashSet<String>> {
    if filters.models.is_empty() {
        return None;
    }
    let mut set = HashSet::new();
    for u in &store.usages {
        if !in_window(filters.from_ms, filters.to_ms, u.occurred_at_ms) {
            continue;
        }
        if !model_ok(u.provider_id.as_deref(), u.model_id.as_deref(), filters) {
            continue;
        }
        if let Some(task) = &u.task_id {
            set.insert(task.clone());
        }
    }
    Some(set)
}

/// The winning (latest attempt) terminal run for each task, in range.
fn winning_runs<'a>(
    store: &'a AnalyticsStore,
    filters: &AnalyticsFilters,
    allowed: Option<&HashSet<String>>,
) -> Vec<&'a RunRecord> {
    let mut best: HashMap<&str, &RunRecord> = HashMap::new();
    for r in &store.runs {
        if !is_terminal_status(&r.status) || !run_in_filters(r, filters) {
            continue;
        }
        if allowed.is_some_and(|a| !a.contains(&r.task_id)) {
            continue;
        }
        match best.get(r.task_id.as_str()) {
            Some(prev) if prev.attempt >= r.attempt => {}
            _ => {
                best.insert(r.task_id.as_str(), r);
            }
        }
    }
    best.into_values().collect()
}

/// Finalized runs (any attempt) in range, used for duration statistics.
fn finalized_runs<'a>(
    store: &'a AnalyticsStore,
    filters: &AnalyticsFilters,
    allowed: Option<&HashSet<String>>,
) -> Vec<&'a RunRecord> {
    store
        .runs
        .iter()
        .filter(|r| {
            r.ended_at_ms.is_some()
                && run_in_filters(r, filters)
                && !allowed.is_some_and(|a| !a.contains(&r.task_id))
        })
        .collect()
}

fn filtered_usages<'a>(
    store: &'a AnalyticsStore,
    filters: &AnalyticsFilters,
) -> Vec<&'a UsageRecord> {
    store
        .usages
        .iter()
        .filter(|u| {
            in_window(filters.from_ms, filters.to_ms, u.occurred_at_ms)
                && project_ok(u.project_path.as_deref(), filters)
                && model_ok(u.provider_id.as_deref(), u.model_id.as_deref(), filters)
        })
        .collect()
}

#[derive(Default)]
struct TokenSums {
    input: Option<u64>,
    output: Option<u64>,
    reasoning: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    total: Option<u64>,
    cost_micros: Option<i64>,
    has_cost: bool,
    has_any: bool,
    count: u64,
}

fn add_opt(acc: &mut Option<u64>, v: Option<u64>) {
    if let Some(v) = v {
        *acc = Some(acc.unwrap_or(0).saturating_add(v));
    }
}

fn sum_usages(usages: &[&UsageRecord]) -> TokenSums {
    let mut s = TokenSums::default();
    for u in usages {
        s.count += 1;
        if u.provider_total_tokens.is_some()
            || u.input_tokens.is_some()
            || u.output_tokens.is_some()
            || u.reasoning_tokens.is_some()
            || u.cache_read_tokens.is_some()
            || u.cache_write_tokens.is_some()
        {
            s.has_any = true;
        }
        add_opt(&mut s.input, u.input_tokens);
        add_opt(&mut s.output, u.output_tokens);
        add_opt(&mut s.reasoning, u.reasoning_tokens);
        add_opt(&mut s.cache_read, u.cache_read_tokens);
        add_opt(&mut s.cache_write, u.cache_write_tokens);
        add_opt(&mut s.total, u.total_tokens());
        if let Some(c) = u.reported_cost_micros {
            s.has_cost = true;
            s.cost_micros = Some(s.cost_micros.unwrap_or(0).saturating_add(c));
        }
    }
    if !s.has_any {
        s.total = None;
    }
    s
}

fn median(sorted: &[i64]) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    if n % 2 == 1 {
        Some(sorted[n / 2])
    } else {
        Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2)
    }
}

fn duration_stats(runs: &[&RunRecord]) -> (Option<i64>, Option<i64>, u64) {
    let mut values: Vec<i64> = runs
        .iter()
        .filter_map(|r| r.active_ms)
        .filter(|v| *v >= 0)
        .collect();
    values.sort_unstable();
    let sample = values.len() as u64;
    if values.is_empty() {
        return (None, None, 0);
    }
    let avg = values.iter().sum::<i64>() / values.len() as i64;
    (Some(avg), median(&values), sample)
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AnalyticsOverview {
    pub has_data: bool,
    pub tasks_total: u64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub tasks_canceled: u64,
    pub tasks_blocked: u64,
    pub tasks_interrupted: u64,
    pub success_rate: Option<f64>,
    pub success_sample: u64,
    pub avg_active_ms: Option<i64>,
    pub median_active_ms: Option<i64>,
    pub duration_sample: u64,
    pub avg_wait_ms: Option<i64>,
    pub total_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reported_cost_micros: Option<i64>,
    pub inference_count: u64,
    /// Agent/opencode runs that could report usage, and how many did.
    pub usage_coverage_runs: u64,
    pub usage_runs_with_data: u64,
    pub currency: String,
    pub cost_basis: String,
    /// Runs that recorded at least one categorized error.
    pub runs_with_errors: u64,
    /// Categorized error occurrences (not task failures).
    pub error_events: u64,
    /// Tasks that needed more than one dispatch.
    pub retry_rate: Option<f64>,
    /// Successful terminal tasks that needed no retry, over terminal outcomes.
    pub first_pass_success_rate: Option<f64>,
    /// Successful agent tasks with no blocking human interaction.
    pub autonomy_rate: Option<f64>,
    pub autonomy_sample: u64,
    pub generated_at_ms: i64,
}

fn wait_stats(runs: &[&RunRecord]) -> (Option<i64>, u64) {
    let mut values: Vec<i64> = runs
        .iter()
        .filter_map(|r| match (r.started_at_ms, r.created_at_ms) {
            (Some(s), c) if s >= c => Some(s - c),
            _ => None,
        })
        .collect();
    values.sort_unstable();
    let sample = values.len() as u64;
    if values.is_empty() {
        return (None, 0);
    }
    (Some(values.iter().sum::<i64>() / values.len() as i64), sample)
}

pub fn overview(store: &AnalyticsStore, filters: &AnalyticsFilters) -> AnalyticsOverview {
    let allowed = model_allowed_tasks(store, filters);
    let winners = winning_runs(store, filters, allowed.as_ref());
    let finalized = finalized_runs(store, filters, allowed.as_ref());
    let usages = filtered_usages(store, filters);
    let sums = sum_usages(&usages);

    // Every task with activity in range, including ones still running.
    let mut task_ids: HashSet<&str> = HashSet::new();
    for r in &store.runs {
        if run_in_filters(r, filters) && !allowed.as_ref().is_some_and(|a| !a.contains(&r.task_id)) {
            task_ids.insert(r.task_id.as_str());
        }
    }

    let count = |status: &str| winners.iter().filter(|r| r.status == status).count() as u64;
    let completed = count("succeeded");
    let failed = count("failed");
    let success_sample = completed + failed;
    let (avg_active, median_active, duration_sample) = duration_stats(&finalized);
    let (avg_wait, _wait_sample) = wait_stats(&finalized);

    // Usage coverage over agent/opencode runs in range.
    let observable: Vec<&RunRecord> = store
        .runs
        .iter()
        .filter(|r| {
            r.kind == "agent"
                && r.provider.as_deref() == Some("opencode")
                && run_in_filters(r, filters)
                && !allowed.as_ref().is_some_and(|a| !a.contains(&r.task_id))
        })
        .collect();
    let run_ids_with_usage: HashSet<&str> = usages
        .iter()
        .filter_map(|u| u.run_id.as_deref())
        .collect();
    let with_data = observable
        .iter()
        .filter(|r| run_ids_with_usage.contains(r.run_id.as_str()))
        .count() as u64;

    // Reliability + autonomy roll-ups, so the Overview can show them without a
    // second round-trip.
    let errors = filtered_errors(store, filters, allowed.as_ref());
    let run_ids_with_errors: HashSet<&str> = errors
        .iter()
        .filter_map(|e| e.run_id.as_deref())
        .collect();
    let mut attempts_by_task: HashMap<&str, u32> = HashMap::new();
    for r in store.runs.iter().filter(|r| {
        r.started_at_ms.is_some()
            && run_in_filters(r, filters)
            && !allowed.as_ref().is_some_and(|a| !a.contains(&r.task_id))
    }) {
        let entry = attempts_by_task.entry(r.task_id.as_str()).or_insert(0);
        *entry = (*entry).max(r.attempt);
    }
    let attempt_tasks = attempts_by_task.len() as u64;
    let retry_tasks = attempts_by_task.values().filter(|a| **a > 1).count() as u64;
    let first_pass = winners
        .iter()
        .filter(|r| r.status == "succeeded" && r.attempt == 1)
        .count() as u64;
    let blocking_tasks: HashSet<&str> = filtered_interactions(store, filters, allowed.as_ref())
        .into_iter()
        .filter(|i| i.blocking)
        .map(|i| i.task_id.as_str())
        .collect();
    let autonomy_sample = winners
        .iter()
        .filter(|r| r.status == "succeeded" && r.kind == "agent")
        .count() as u64;
    let autonomy_autonomous = winners
        .iter()
        .filter(|r| {
            r.status == "succeeded"
                && r.kind == "agent"
                && !blocking_tasks.contains(r.task_id.as_str())
        })
        .count() as u64;

    AnalyticsOverview {
        has_data: !store.runs.is_empty() || !store.usages.is_empty(),
        tasks_total: task_ids.len() as u64,
        tasks_completed: completed,
        tasks_failed: failed,
        tasks_canceled: count("canceled"),
        tasks_blocked: count("blocked"),
        tasks_interrupted: count("interrupted"),
        success_rate: if success_sample > 0 {
            Some(completed as f64 / success_sample as f64)
        } else {
            None
        },
        success_sample,
        avg_active_ms: avg_active,
        median_active_ms: median_active,
        duration_sample,
        avg_wait_ms: avg_wait,
        total_tokens: sums.total,
        input_tokens: sums.input,
        output_tokens: sums.output,
        reasoning_tokens: sums.reasoning,
        cache_read_tokens: sums.cache_read,
        cache_write_tokens: sums.cache_write,
        reported_cost_micros: if sums.has_cost { sums.cost_micros } else { None },
        inference_count: usages.len() as u64,
        usage_coverage_runs: observable.len() as u64,
        usage_runs_with_data: with_data,
        currency: "USD".to_string(),
        cost_basis: "provider_reported_not_a_bill".to_string(),
        runs_with_errors: run_ids_with_errors.len() as u64,
        error_events: errors.len() as u64,
        retry_rate: if attempt_tasks > 0 {
            Some(retry_tasks as f64 / attempt_tasks as f64)
        } else {
            None
        },
        first_pass_success_rate: if success_sample > 0 {
            Some(first_pass as f64 / success_sample as f64)
        } else {
            None
        },
        autonomy_rate: if autonomy_sample > 0 {
            Some(autonomy_autonomous as f64 / autonomy_sample as f64)
        } else {
            None
        },
        autonomy_sample,
        generated_at_ms: now_ms(),
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TimeSeriesPoint {
    pub bucket_ms: i64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub tasks_terminal: u64,
    pub total_tokens: Option<u64>,
    pub reported_cost_micros: Option<i64>,
    pub inference_count: u64,
}

/// Local midnight (unix ms) for the day containing `ms`.
pub fn local_day_start_ms(ms: i64) -> i64 {
    use chrono::{Local, TimeZone};
    let secs = ms.div_euclid(1000);
    let Some(dt) = Local.timestamp_opt(secs, 0).single() else {
        return ms - ms.rem_euclid(86_400_000);
    };
    let Some(naive) = dt.date_naive().and_hms_opt(0, 0, 0) else {
        return ms - ms.rem_euclid(86_400_000);
    };
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|d| d.timestamp_millis())
        .unwrap_or(ms)
}

pub fn time_series(store: &AnalyticsStore, filters: &AnalyticsFilters) -> Vec<TimeSeriesPoint> {
    let allowed = model_allowed_tasks(store, filters);
    let winners = winning_runs(store, filters, allowed.as_ref());
    let usages = filtered_usages(store, filters);

    let mut buckets: BTreeMap<i64, TimeSeriesPoint> = BTreeMap::new();
    for r in &winners {
        let at = r.ended_at_ms.unwrap_or(r.created_at_ms);
        let entry = buckets.entry(local_day_start_ms(at)).or_insert_with(|| {
            TimeSeriesPoint {
                bucket_ms: local_day_start_ms(at),
                ..Default::default()
            }
        });
        entry.tasks_terminal += 1;
        match r.status.as_str() {
            "succeeded" => entry.tasks_completed += 1,
            "failed" => entry.tasks_failed += 1,
            _ => {}
        }
    }
    for u in &usages {
        let entry = buckets
            .entry(local_day_start_ms(u.occurred_at_ms))
            .or_insert_with(|| TimeSeriesPoint {
                bucket_ms: local_day_start_ms(u.occurred_at_ms),
                ..Default::default()
            });
        entry.inference_count += 1;
        if let Some(t) = u.total_tokens() {
            entry.total_tokens = Some(entry.total_tokens.unwrap_or(0).saturating_add(t));
        }
        if let Some(c) = u.reported_cost_micros {
            entry.reported_cost_micros =
                Some(entry.reported_cost_micros.unwrap_or(0).saturating_add(c));
        }
    }
    buckets.into_values().collect()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProjectMetric {
    pub project_path: String,
    pub project_name: String,
    pub tasks_total: u64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub success_rate: Option<f64>,
    pub success_sample: u64,
    pub avg_active_ms: Option<i64>,
    pub median_active_ms: Option<i64>,
    pub total_tokens: Option<u64>,
    pub reported_cost_micros: Option<i64>,
    pub inference_count: u64,
}

pub fn projects(
    store: &AnalyticsStore,
    filters: &AnalyticsFilters,
    names: &HashMap<String, String>,
) -> Vec<ProjectMetric> {
    let allowed = model_allowed_tasks(store, filters);

    struct Acc {
        completed: u64,
        failed: u64,
    }
    let mut acc: HashMap<String, Acc> = HashMap::new();
    let winners = winning_runs(store, filters, allowed.as_ref());
    for r in &winners {
        let entry = acc.entry(r.project_path.clone()).or_insert(Acc {
            completed: 0,
            failed: 0,
        });
        match r.status.as_str() {
            "succeeded" => entry.completed += 1,
            "failed" => entry.failed += 1,
            _ => {}
        }
    }

    let finalized = finalized_runs(store, filters, allowed.as_ref());
    let mut durations: HashMap<String, Vec<i64>> = HashMap::new();
    for r in &finalized {
        if let Some(ms) = r.active_ms {
            durations.entry(r.project_path.clone()).or_default().push(ms);
        }
    }

    let usages = filtered_usages(store, filters);
    let mut usage_acc: HashMap<String, TokenSums> = HashMap::new();
    for u in &usages {
        if let Some(path) = &u.project_path {
            let sums = usage_acc
                .entry(path.clone())
                .or_default();
            sums.count += 1;
            add_opt(&mut sums.input, u.input_tokens);
            add_opt(&mut sums.output, u.output_tokens);
            add_opt(&mut sums.reasoning, u.reasoning_tokens);
            add_opt(&mut sums.cache_read, u.cache_read_tokens);
            add_opt(&mut sums.cache_write, u.cache_write_tokens);
            add_opt(&mut sums.total, u.total_tokens());
            if u.total_tokens().is_some() {
                sums.has_any = true;
            }
            if let Some(c) = u.reported_cost_micros {
                sums.has_cost = true;
                sums.cost_micros = Some(sums.cost_micros.unwrap_or(0).saturating_add(c));
            }
        }
    }

    // Union of projects that have either outcomes or usage.
    let mut paths: HashSet<String> = HashSet::new();
    paths.extend(acc.keys().cloned());
    paths.extend(durations.keys().cloned());
    paths.extend(usage_acc.keys().cloned());

    let mut out: Vec<ProjectMetric> = paths
        .into_iter()
        .map(|path| {
            let (completed, failed) = acc
                .get(&path)
                .map(|a| (a.completed, a.failed))
                .unwrap_or((0, 0));
            let sample = completed + failed;
            let mut values = durations.remove(&path).unwrap_or_default();
            values.sort_unstable();
            let avg = if values.is_empty() {
                None
            } else {
                Some(values.iter().sum::<i64>() / values.len() as i64)
            };
            let sums = usage_acc.remove(&path).unwrap_or_default();
            ProjectMetric {
                project_name: names.get(&path).cloned().unwrap_or_else(|| {
                    Path::new(&path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&path)
                        .to_string()
                }),
                project_path: path,
                tasks_total: completed + failed,
                tasks_completed: completed,
                tasks_failed: failed,
                success_rate: if sample > 0 {
                    Some(completed as f64 / sample as f64)
                } else {
                    None
                },
                success_sample: sample,
                avg_active_ms: avg,
                median_active_ms: median(&values),
                total_tokens: sums.total,
                reported_cost_micros: if sums.has_cost { sums.cost_micros } else { None },
                inference_count: sums.count,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.tasks_total
            .cmp(&a.tasks_total)
            .then_with(|| a.project_name.cmp(&b.project_name))
    });
    out
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelMetric {
    pub provider_id: String,
    pub model_id: String,
    pub variant: Option<String>,
    pub inference_count: u64,
    pub task_runs: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub reported_cost_micros: Option<i64>,
    /// Task runs whose usage was exclusively this model. Only these are used
    /// for a per-model success rate, so a multi-model task never credits every
    /// model that touched it.
    pub exclusive_task_runs: u64,
    pub exclusive_success: u64,
    pub exclusive_success_rate: Option<f64>,
    /// Task runs that used this model *and* another model (`mixed`).
    pub mixed_task_runs: u64,
}

pub fn models(store: &AnalyticsStore, filters: &AnalyticsFilters) -> Vec<ModelMetric> {
    let usages = filtered_usages(store, filters);
    type ModelKey = (String, String, Option<String>);
    #[derive(Default)]
    struct Acc {
        count: u64,
        runs: HashSet<String>,
        tasks: HashSet<String>,
        input: Option<u64>,
        output: Option<u64>,
        reasoning: Option<u64>,
        cache_read: Option<u64>,
        cache_write: Option<u64>,
        total: Option<u64>,
        cost: Option<i64>,
        has_cost: bool,
        has_any: bool,
    }
    let mut acc: HashMap<ModelKey, Acc> = HashMap::new();
    // Which models each task used, to separate exclusive from mixed attribution.
    let mut task_models: HashMap<String, HashSet<ModelKey>> = HashMap::new();
    for u in &usages {
        let key = (
            u.provider_id.clone().unwrap_or_default(),
            u.model_id.clone().unwrap_or_default(),
            u.variant.clone(),
        );
        if let Some(task) = &u.task_id {
            task_models
                .entry(task.clone())
                .or_default()
                .insert(key.clone());
        }
        let a = acc.entry(key).or_default();
        a.count += 1;
        if let Some(run) = &u.run_id {
            a.runs.insert(run.clone());
        }
        if let Some(task) = &u.task_id {
            a.tasks.insert(task.clone());
        }
        if u.total_tokens().is_some()
            || u.input_tokens.is_some()
            || u.output_tokens.is_some()
            || u.reasoning_tokens.is_some()
            || u.cache_read_tokens.is_some()
            || u.cache_write_tokens.is_some()
        {
            a.has_any = true;
        }
        add_opt(&mut a.input, u.input_tokens);
        add_opt(&mut a.output, u.output_tokens);
        add_opt(&mut a.reasoning, u.reasoning_tokens);
        add_opt(&mut a.cache_read, u.cache_read_tokens);
        add_opt(&mut a.cache_write, u.cache_write_tokens);
        add_opt(&mut a.total, u.total_tokens());
        if let Some(c) = u.reported_cost_micros {
            a.has_cost = true;
            a.cost = Some(a.cost.unwrap_or(0).saturating_add(c));
        }
    }
    let outcomes = task_terminal_status(store);
    let mut out: Vec<ModelMetric> = acc
        .into_iter()
        .map(|((provider_id, model_id, variant), a)| {
            let mut exclusive_task_runs = 0u64;
            let mut exclusive_success = 0u64;
            let mut mixed_task_runs = 0u64;
            for task in &a.tasks {
                let exclusive = task_models.get(task).is_some_and(|m| m.len() == 1);
                if exclusive {
                    exclusive_task_runs += 1;
                    if outcomes.get(task.as_str()) == Some(&"succeeded") {
                        exclusive_success += 1;
                    }
                } else {
                    mixed_task_runs += 1;
                }
            }
            ModelMetric {
                provider_id,
                model_id,
                variant,
                inference_count: a.count,
                task_runs: a.runs.len() as u64,
                input_tokens: a.input,
                output_tokens: a.output,
                reasoning_tokens: a.reasoning,
                cache_read_tokens: a.cache_read,
                cache_write_tokens: a.cache_write,
                total_tokens: if a.has_any { a.total } else { None },
                reported_cost_micros: if a.has_cost { a.cost } else { None },
                exclusive_task_runs,
                exclusive_success,
                exclusive_success_rate: if exclusive_task_runs > 0 {
                    Some(exclusive_success as f64 / exclusive_task_runs as f64)
                } else {
                    None
                },
                mixed_task_runs,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.total_tokens
            .unwrap_or(0)
            .cmp(&a.total_tokens.unwrap_or(0))
            .then_with(|| a.model_id.cmp(&b.model_id))
    });
    out
}

// ---------------------------------------------------------------------------
// Reliability (Phase 2)
// ---------------------------------------------------------------------------

fn error_in_filters(e: &ErrorRecord, filters: &AnalyticsFilters) -> bool {
    in_window(filters.from_ms, filters.to_ms, e.occurred_at_ms)
        && project_ok(e.project_path.as_deref(), filters)
}

fn filtered_errors<'a>(
    store: &'a AnalyticsStore,
    filters: &AnalyticsFilters,
    allowed: Option<&HashSet<String>>,
) -> Vec<&'a ErrorRecord> {
    store
        .errors
        .iter()
        .filter(|e| {
            if !error_in_filters(e, filters) {
                return false;
            }
            match (allowed, e.task_id.as_deref()) {
                (Some(a), Some(t)) => a.contains(t),
                (Some(_), None) => false,
                (None, _) => true,
            }
        })
        .collect()
}

/// The eventual terminal status of each task, from its latest attempt. Recovery
/// is judged against the task's final outcome, not the failing run alone.
fn task_terminal_status(store: &AnalyticsStore) -> HashMap<&str, &str> {
    let mut best: HashMap<&str, &RunRecord> = HashMap::new();
    for r in &store.runs {
        if !is_terminal_status(&r.status) {
            continue;
        }
        match best.get(r.task_id.as_str()) {
            Some(prev) if prev.attempt >= r.attempt => {}
            _ => {
                best.insert(r.task_id.as_str(), r);
            }
        }
    }
    best.into_iter().map(|(k, r)| (k, r.status.as_str())).collect()
}

fn recovered_of(e: &ErrorRecord, outcomes: &HashMap<&str, &str>) -> Option<bool> {
    let task = e.task_id.as_deref()?;
    match outcomes.get(task) {
        Some(&"succeeded") => Some(true),
        Some(_) => Some(false),
        None => None,
    }
}

fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[idx])
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReliabilitySummary {
    pub has_data: bool,
    pub tasks_failed: u64,
    pub failed_sample: u64,
    pub task_failure_rate: Option<f64>,
    pub runs_with_errors: u64,
    pub observed_runs: u64,
    pub runs_with_errors_rate: Option<f64>,
    pub error_events: u64,
    pub recoverable_errors: u64,
    pub recovered_errors: u64,
    pub recovery_rate: Option<f64>,
    pub retry_tasks: u64,
    pub attempt_tasks: u64,
    pub retry_rate: Option<f64>,
    pub first_pass_success: u64,
    pub first_pass_sample: u64,
    pub first_pass_success_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorCategoryMetric {
    pub category: String,
    pub occurrences: u64,
    pub tasks: u64,
    pub recovered: u64,
    pub recoverable: u64,
    pub recovery_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorGroup {
    pub fingerprint: String,
    pub category: String,
    pub code: String,
    pub stage: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    pub occurrences: u64,
    pub tasks: u64,
    pub recovered: u64,
    pub recoverable: u64,
    pub unresolved: u64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    #[serde(default)]
    pub sample: Option<String>,
    /// A capped list of affected task ids, for drill-down.
    #[serde(default)]
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StageToolMetric {
    pub name: String,
    pub occurrences: u64,
    pub tasks: u64,
    pub recovered: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorTimeSeriesPoint {
    pub bucket_ms: i64,
    pub errors: u64,
    pub tasks_failed: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReliabilityResult {
    pub summary: ReliabilitySummary,
    pub categories: Vec<ErrorCategoryMetric>,
    pub groups: Vec<ErrorGroup>,
    pub stages: Vec<StageToolMetric>,
    pub tools: Vec<StageToolMetric>,
    pub series: Vec<ErrorTimeSeriesPoint>,
}

const MAX_GROUP_TASKS: usize = 20;

pub fn reliability(
    store: &AnalyticsStore,
    filters: &AnalyticsFilters,
) -> ReliabilityResult {
    let allowed = model_allowed_tasks(store, filters);
    let outcomes = task_terminal_status(store);
    let winners = winning_runs(store, filters, allowed.as_ref());
    let errors = filtered_errors(store, filters, allowed.as_ref());

    let failed = winners.iter().filter(|r| r.status == "failed").count() as u64;
    let succeeded = winners.iter().filter(|r| r.status == "succeeded").count() as u64;
    let failed_sample = failed + succeeded;

    let observed: Vec<&RunRecord> = store
        .runs
        .iter()
        .filter(|r| {
            r.started_at_ms.is_some()
                && run_in_filters(r, filters)
                && !allowed.as_ref().is_some_and(|a| !a.contains(&r.task_id))
        })
        .collect();
    let observed_runs = observed.len() as u64;

    let mut run_ids_with_errors: HashSet<&str> = HashSet::new();
    let mut recoverable = 0u64;
    let mut recovered = 0u64;
    for e in &errors {
        if let Some(run) = e.run_id.as_deref() {
            run_ids_with_errors.insert(run);
        }
        if let Some(r) = recovered_of(e, &outcomes) {
            recoverable += 1;
            if r {
                recovered += 1;
            }
        }
    }

    // Retry rate: tasks that needed more than one dispatch.
    let mut attempts_by_task: HashMap<&str, u32> = HashMap::new();
    for r in &observed {
        let entry = attempts_by_task.entry(r.task_id.as_str()).or_insert(0);
        *entry = (*entry).max(r.attempt);
    }
    let attempt_tasks = attempts_by_task.len() as u64;
    let retry_tasks = attempts_by_task.values().filter(|a| **a > 1).count() as u64;

    let first_pass_success = winners
        .iter()
        .filter(|r| r.status == "succeeded" && r.attempt == 1)
        .count() as u64;

    let summary = ReliabilitySummary {
        has_data: !store.runs.is_empty() || !store.errors.is_empty(),
        tasks_failed: failed,
        failed_sample,
        task_failure_rate: if failed_sample > 0 {
            Some(failed as f64 / failed_sample as f64)
        } else {
            None
        },
        runs_with_errors: run_ids_with_errors.len() as u64,
        observed_runs,
        runs_with_errors_rate: if observed_runs > 0 {
            Some(run_ids_with_errors.len() as f64 / observed_runs as f64)
        } else {
            None
        },
        error_events: errors.len() as u64,
        recoverable_errors: recoverable,
        recovered_errors: recovered,
        recovery_rate: if recoverable > 0 {
            Some(recovered as f64 / recoverable as f64)
        } else {
            None
        },
        retry_tasks,
        attempt_tasks,
        retry_rate: if attempt_tasks > 0 {
            Some(retry_tasks as f64 / attempt_tasks as f64)
        } else {
            None
        },
        first_pass_success,
        first_pass_sample: failed_sample,
        first_pass_success_rate: if failed_sample > 0 {
            Some(first_pass_success as f64 / failed_sample as f64)
        } else {
            None
        },
    };

    // Category metrics.
    #[derive(Default)]
    struct CatAcc {
        occurrences: u64,
        tasks: HashSet<String>,
        recovered: u64,
        recoverable: u64,
    }
    let mut cats: HashMap<&str, CatAcc> = HashMap::new();
    for e in &errors {
        let acc = cats.entry(e.category.as_str()).or_default();
        acc.occurrences += 1;
        if let Some(t) = &e.task_id {
            acc.tasks.insert(t.clone());
        }
        if let Some(r) = recovered_of(e, &outcomes) {
            acc.recoverable += 1;
            if r {
                acc.recovered += 1;
            }
        }
    }
    let mut categories: Vec<ErrorCategoryMetric> = cats
        .into_iter()
        .map(|(category, a)| ErrorCategoryMetric {
            category: category.to_string(),
            occurrences: a.occurrences,
            tasks: a.tasks.len() as u64,
            recovered: a.recovered,
            recoverable: a.recoverable,
            recovery_rate: if a.recoverable > 0 {
                Some(a.recovered as f64 / a.recoverable as f64)
            } else {
                None
            },
        })
        .collect();
    categories.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.category.cmp(&b.category))
    });

    // Fingerprint groups.
    #[derive(Default)]
    struct GroupAcc {
        category: String,
        stage: String,
        tool: Option<String>,
        occurrences: u64,
        tasks: HashSet<String>,
        recovered: u64,
        recoverable: u64,
        unresolved: u64,
        first: i64,
        last: i64,
        sample: Option<String>,
        task_ids: Vec<String>,
    }
    let mut groups_acc: HashMap<String, GroupAcc> = HashMap::new();
    for e in &errors {
        let acc = groups_acc
            .entry(e.fingerprint.clone())
            .or_insert_with(|| GroupAcc {
                category: e.category.clone(),
                stage: e.stage.clone(),
                tool: e.tool_name.clone(),
                first: e.occurred_at_ms,
                last: e.occurred_at_ms,
                ..Default::default()
            });
        acc.occurrences += 1;
        acc.first = acc.first.min(e.occurred_at_ms);
        acc.last = acc.last.max(e.occurred_at_ms);
        if let Some(t) = &e.task_id {
            acc.tasks.insert(t.clone());
            if acc.task_ids.len() < MAX_GROUP_TASKS && !acc.task_ids.contains(t) {
                acc.task_ids.push(t.clone());
            }
        }
        if let Some(r) = recovered_of(e, &outcomes) {
            acc.recoverable += 1;
            if r {
                acc.recovered += 1;
            }
        } else {
            acc.unresolved += 1;
        }
        if acc.sample.is_none() {
            acc.sample = e.summary_redacted.clone();
        }
    }
    let mut groups: Vec<ErrorGroup> = groups_acc
        .into_iter()
        .map(|(fingerprint, a)| {
            let code = fingerprint_code(&fingerprint);
            ErrorGroup {
                fingerprint,
                category: a.category,
                code,
                stage: a.stage,
                tool_name: a.tool,
                occurrences: a.occurrences,
                tasks: a.tasks.len() as u64,
                recovered: a.recovered,
                recoverable: a.recoverable,
                unresolved: a.unresolved,
                first_seen_ms: a.first,
                last_seen_ms: a.last,
                sample: a.sample,
                task_ids: a.task_ids,
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| b.last_seen_ms.cmp(&a.last_seen_ms))
    });

    // Stage and tool breakdowns.
    fn breakdown<F: Fn(&ErrorRecord) -> Option<String>>(
        errors: &[&ErrorRecord],
        outcomes: &HashMap<&str, &str>,
        key: F,
    ) -> Vec<StageToolMetric> {
        #[derive(Default)]
        struct Acc {
            occurrences: u64,
            tasks: HashSet<String>,
            recovered: u64,
        }
        let mut acc: HashMap<String, Acc> = HashMap::new();
        for e in errors {
            let Some(name) = key(e) else { continue };
            let a = acc.entry(name).or_default();
            a.occurrences += 1;
            if let Some(t) = &e.task_id {
                a.tasks.insert(t.clone());
            }
            if recovered_of(e, outcomes) == Some(true) {
                a.recovered += 1;
            }
        }
        let mut out: Vec<StageToolMetric> = acc
            .into_iter()
            .map(|(name, a)| StageToolMetric {
                name,
                occurrences: a.occurrences,
                tasks: a.tasks.len() as u64,
                recovered: a.recovered,
            })
            .collect();
        out.sort_by_key(|m| std::cmp::Reverse(m.occurrences));
        out
    }
    let stages = breakdown(&errors, &outcomes, |e| Some(e.stage.clone()));
    let tools = breakdown(&errors, &outcomes, |e| e.tool_name.clone());

    // Daily error series.
    let mut series: BTreeMap<i64, ErrorTimeSeriesPoint> = BTreeMap::new();
    for e in &errors {
        let bucket = local_day_start_ms(e.occurred_at_ms);
        let entry = series.entry(bucket).or_insert_with(|| ErrorTimeSeriesPoint {
            bucket_ms: bucket,
            ..Default::default()
        });
        entry.errors += 1;
    }
    for r in &winners {
        if r.status != "failed" {
            continue;
        }
        let at = r.ended_at_ms.unwrap_or(r.created_at_ms);
        let bucket = local_day_start_ms(at);
        let entry = series.entry(bucket).or_insert_with(|| ErrorTimeSeriesPoint {
            bucket_ms: bucket,
            ..Default::default()
        });
        entry.tasks_failed += 1;
    }

    ReliabilityResult {
        summary,
        categories,
        groups,
        stages,
        tools,
        series: series.into_values().collect(),
    }
}

// ---------------------------------------------------------------------------
// Interactions (Phase 3)
// ---------------------------------------------------------------------------

fn interaction_in_filters(i: &InteractionRecord, filters: &AnalyticsFilters) -> bool {
    in_window(filters.from_ms, filters.to_ms, i.asked_at_ms)
        && project_ok(Some(&i.project_path), filters)
}

fn filtered_interactions<'a>(
    store: &'a AnalyticsStore,
    filters: &AnalyticsFilters,
    allowed: Option<&HashSet<String>>,
) -> Vec<&'a InteractionRecord> {
    store
        .interactions
        .iter()
        .filter(|i| {
            if !interaction_in_filters(i, filters) {
                return false;
            }
            match allowed {
                Some(a) => a.contains(&i.task_id),
                None => true,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionSummary {
    pub has_data: bool,
    pub questions: u64,
    pub permissions: u64,
    pub manual_confirmations: u64,
    pub blocking_total: u64,
    pub resolved: u64,
    pub unresolved: u64,
    pub total_wait_ms: i64,
    pub avg_wait_ms: Option<i64>,
    pub median_wait_ms: Option<i64>,
    pub p95_wait_ms: Option<i64>,
    pub autonomy_rate: Option<f64>,
    pub autonomy_autonomous: u64,
    pub autonomy_sample: u64,
    pub first_pass_success_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionGroup {
    pub fingerprint: String,
    pub kind: String,
    pub category: String,
    pub occurrences: u64,
    pub tasks: u64,
    pub blocking: u64,
    pub unresolved: u64,
    pub avg_wait_ms: Option<i64>,
    pub last_seen_ms: i64,
    #[serde(default)]
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionCategoryMetric {
    pub category: String,
    pub kind: String,
    pub occurrences: u64,
    pub tasks: u64,
    pub unresolved: u64,
    pub avg_wait_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Recommendation {
    pub title: String,
    pub detail: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionResult {
    pub summary: InteractionSummary,
    pub groups: Vec<InteractionGroup>,
    pub categories: Vec<InteractionCategoryMetric>,
    pub recommendations: Vec<Recommendation>,
}

pub fn interactions(
    store: &AnalyticsStore,
    filters: &AnalyticsFilters,
) -> InteractionResult {
    let allowed = model_allowed_tasks(store, filters);
    let items = filtered_interactions(store, filters, allowed.as_ref());

    let count_kind = |k: &str| items.iter().filter(|i| i.kind == k).count() as u64;
    let blocking_total = items.iter().filter(|i| i.blocking).count() as u64;
    let resolved = items.iter().filter(|i| i.resolved_at_ms.is_some()).count() as u64;
    let unresolved = items.len() as u64 - resolved;

    let mut waits: Vec<i64> = items
        .iter()
        .filter(|i| i.blocking)
        .filter_map(|i| i.resolved_at_ms.map(|r| (r - i.asked_at_ms).max(0)))
        .collect();
    waits.sort_unstable();
    let total_wait_ms = waits.iter().sum();
    let avg_wait_ms = if waits.is_empty() {
        None
    } else {
        Some(total_wait_ms / waits.len() as i64)
    };

    // Autonomy: successful agent tasks that needed no blocking intervention.
    let winners = winning_runs(store, filters, allowed.as_ref());
    let blocking_tasks: HashSet<&str> = items
        .iter()
        .filter(|i| i.blocking)
        .map(|i| i.task_id.as_str())
        .collect();
    let successful_agents: Vec<&&RunRecord> = winners
        .iter()
        .filter(|r| r.status == "succeeded" && r.kind == "agent")
        .collect();
    let autonomy_sample = successful_agents.len() as u64;
    let autonomy_autonomous = successful_agents
        .iter()
        .filter(|r| !blocking_tasks.contains(r.task_id.as_str()))
        .count() as u64;

    let succeeded = winners.iter().filter(|r| r.status == "succeeded").count() as u64;
    let failed = winners.iter().filter(|r| r.status == "failed").count() as u64;
    let first_pass = winners
        .iter()
        .filter(|r| r.status == "succeeded" && r.attempt == 1)
        .count() as u64;

    let summary = InteractionSummary {
        has_data: !store.interactions.is_empty() || !store.runs.is_empty(),
        questions: count_kind("question"),
        permissions: count_kind("permission"),
        manual_confirmations: count_kind("manual_confirmation"),
        blocking_total,
        resolved,
        unresolved,
        total_wait_ms,
        avg_wait_ms,
        median_wait_ms: median(&waits),
        p95_wait_ms: percentile(&waits, 95.0),
        autonomy_rate: if autonomy_sample > 0 {
            Some(autonomy_autonomous as f64 / autonomy_sample as f64)
        } else {
            None
        },
        autonomy_autonomous,
        autonomy_sample,
        first_pass_success_rate: if succeeded + failed > 0 {
            Some(first_pass as f64 / (succeeded + failed) as f64)
        } else {
            None
        },
    };

    // Groups by fingerprint.
    #[derive(Default)]
    struct GroupAcc {
        kind: String,
        category: String,
        occurrences: u64,
        tasks: HashSet<String>,
        blocking: u64,
        unresolved: u64,
        waits: Vec<i64>,
        last: i64,
        task_ids: Vec<String>,
    }
    let mut acc: HashMap<String, GroupAcc> = HashMap::new();
    for i in &items {
        let a = acc.entry(i.fingerprint.clone()).or_insert_with(|| GroupAcc {
            kind: i.kind.clone(),
            category: i.category.clone(),
            last: i.asked_at_ms,
            ..Default::default()
        });
        a.occurrences += 1;
        a.tasks.insert(i.task_id.clone());
        a.last = a.last.max(i.asked_at_ms);
        if i.blocking {
            a.blocking += 1;
        }
        match i.resolved_at_ms {
            None => a.unresolved += 1,
            Some(resolved) if i.blocking => {
                a.waits.push((resolved - i.asked_at_ms).max(0))
            }
            Some(_) => {}
        }
        if a.task_ids.len() < MAX_GROUP_TASKS && !a.task_ids.contains(&i.task_id) {
            a.task_ids.push(i.task_id.clone());
        }
    }
    let mut groups: Vec<InteractionGroup> = acc
        .into_iter()
        .map(|(fingerprint, mut a)| {
            a.waits.sort_unstable();
            InteractionGroup {
                fingerprint,
                kind: a.kind,
                category: a.category,
                occurrences: a.occurrences,
                tasks: a.tasks.len() as u64,
                blocking: a.blocking,
                unresolved: a.unresolved,
                avg_wait_ms: if a.waits.is_empty() {
                    None
                } else {
                    Some(a.waits.iter().sum::<i64>() / a.waits.len() as i64)
                },
                last_seen_ms: a.last,
                task_ids: a.task_ids,
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| b.last_seen_ms.cmp(&a.last_seen_ms))
    });

    // Category metrics.
    #[derive(Default)]
    struct CatAcc {
        kind: String,
        occurrences: u64,
        tasks: HashSet<String>,
        unresolved: u64,
        waits: Vec<i64>,
    }
    let mut cats: HashMap<String, CatAcc> = HashMap::new();
    for i in &items {
        let a = cats.entry(i.category.clone()).or_insert_with(|| CatAcc {
            kind: i.kind.clone(),
            ..Default::default()
        });
        a.occurrences += 1;
        a.tasks.insert(i.task_id.clone());
        match i.resolved_at_ms {
            None => a.unresolved += 1,
            Some(resolved) if i.blocking => {
                a.waits.push((resolved - i.asked_at_ms).max(0))
            }
            Some(_) => {}
        }
    }
    let mut categories: Vec<InteractionCategoryMetric> = cats
        .into_iter()
        .map(|(category, mut a)| {
            a.waits.sort_unstable();
            InteractionCategoryMetric {
                category,
                kind: a.kind,
                occurrences: a.occurrences,
                tasks: a.tasks.len() as u64,
                unresolved: a.unresolved,
                avg_wait_ms: if a.waits.is_empty() {
                    None
                } else {
                    Some(a.waits.iter().sum::<i64>() / a.waits.len() as i64)
                },
            }
        })
        .collect();
    categories.sort_by_key(|c| std::cmp::Reverse(c.occurrences));

    // Rule-based, advisory recommendations. Nothing here edits a repo or
    // loosens a permission.
    let mut recommendations = Vec::new();
    let total = items.len() as u64;
    if total >= 3 {
        let top = categories.first();
        if let Some(top) = top {
            let (title, detail) = match top.category.as_str() {
                "missing_repository_instructions" => (
                    "Document build/test commands",
                    "Repeated questions about project commands suggest adding them to AGENTS.md.",
                ),
                "implementation_choice" => (
                    "Record chosen conventions",
                    "Frequent design questions suggest documenting the preferred approach in AGENTS.md.",
                ),
                "environment_setup" => (
                    "Describe the dev environment",
                    "Repeated setup questions suggest documenting versions and install steps.",
                ),
                "credentials" => (
                    "Preconfigure credentials",
                    "Repeated credential requests suggest using project environment variables.",
                ),
                "permissions" => (
                    "Review permission profiles",
                    "Frequent permission prompts may mean a broader profile fits this project.",
                ),
                _ => (
                    "Clarify recurring questions",
                    "The same kind of question keeps recurring; a short project note may help.",
                ),
            };
            recommendations.push(Recommendation {
                title: title.to_string(),
                detail: detail.to_string(),
                count: top.occurrences,
            });
        }
    }

    InteractionResult {
        summary,
        groups,
        categories,
        recommendations,
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunDetail {
    pub run_id: String,
    pub attempt: u32,
    pub kind: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub active_ms: Option<i64>,
    pub total_tokens: Option<u64>,
    pub reported_cost_micros: Option<i64>,
    pub inference_count: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ErrorDetail {
    pub error_id: String,
    pub category: String,
    pub fingerprint: String,
    pub code: String,
    pub stage: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub recovered: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InteractionDetail {
    pub interaction_id: String,
    pub kind: String,
    pub category: String,
    pub asked_at_ms: i64,
    #[serde(default)]
    pub resolved_at_ms: Option<i64>,
    pub blocking: bool,
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TaskAnalyticsDetail {
    pub task_id: String,
    pub title: String,
    pub project_path: String,
    pub status: String,
    pub kind: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub attempts: u32,
    pub review_status: Option<String>,
    pub error: Option<String>,
    pub runs: Vec<RunDetail>,
    #[serde(default)]
    pub errors: Vec<ErrorDetail>,
    #[serde(default)]
    pub interactions: Vec<InteractionDetail>,
    pub total_tokens: Option<u64>,
    pub reported_cost_micros: Option<i64>,
}

/// Usage attributed to a task, across all of its attempts.
pub fn task_detail(
    store: &AnalyticsStore,
    task_id: &str,
    task: Option<&crate::models::Task>,
) -> Option<TaskAnalyticsDetail> {
    let mut runs: Vec<&RunRecord> = store
        .runs
        .iter()
        .filter(|r| r.task_id == task_id)
        .collect();
    if runs.is_empty() && task.is_none() {
        return None;
    }
    runs.sort_by_key(|r| r.attempt);

    let usages: Vec<&UsageRecord> = store
        .usages
        .iter()
        .filter(|u| u.task_id.as_deref() == Some(task_id))
        .collect();
    let sums = sum_usages(&usages);

    let run_details: Vec<RunDetail> = runs
        .iter()
        .map(|r| {
            let run_usages: Vec<&UsageRecord> = usages
                .iter()
                .filter(|u| u.run_id.as_deref() == Some(r.run_id.as_str()))
                .copied()
                .collect();
            let s = sum_usages(&run_usages);
            RunDetail {
                run_id: r.run_id.clone(),
                attempt: r.attempt,
                kind: r.kind.clone(),
                provider: r.provider.clone(),
                model: r.model.clone(),
                status: r.status.clone(),
                started_at_ms: r.started_at_ms,
                ended_at_ms: r.ended_at_ms,
                active_ms: r.active_ms,
                total_tokens: s.total,
                reported_cost_micros: s.cost_micros,
                inference_count: run_usages.len() as u64,
            }
        })
        .collect();

    let outcomes = task_terminal_status(store);
    let error_details: Vec<ErrorDetail> = store
        .errors
        .iter()
        .filter(|e| e.task_id.as_deref() == Some(task_id))
        .map(|e| ErrorDetail {
            error_id: e.error_id.clone(),
            category: e.category.clone(),
            fingerprint: e.fingerprint.clone(),
            code: fingerprint_code(&e.fingerprint),
            stage: e.stage.clone(),
            tool_name: e.tool_name.clone(),
            occurred_at_ms: e.occurred_at_ms,
            recovered: recovered_of(e, &outcomes),
        })
        .collect();
    let interaction_details: Vec<InteractionDetail> = store
        .interactions
        .iter()
        .filter(|i| i.task_id == task_id)
        .map(|i| InteractionDetail {
            interaction_id: i.interaction_id.clone(),
            kind: i.kind.clone(),
            category: i.category.clone(),
            asked_at_ms: i.asked_at_ms,
            resolved_at_ms: i.resolved_at_ms,
            blocking: i.blocking,
            outcome: i.outcome.clone(),
        })
        .collect();

    let first = runs.first();
    Some(TaskAnalyticsDetail {
        task_id: task_id.to_string(),
        title: task.map(|t| t.title.clone()).unwrap_or_default(),
        project_path: task
            .map(|t| t.project_path.clone())
            .or_else(|| first.map(|r| r.project_path.clone()))
            .unwrap_or_default(),
        status: task
            .map(|t| status_str(t.status).to_string())
            .or_else(|| runs.last().map(|r| r.status.clone()))
            .unwrap_or_default(),
        kind: task
            .map(|t| kind_str(t.kind).to_string())
            .or_else(|| first.map(|r| r.kind.clone()))
            .unwrap_or_default(),
        provider: task
            .and_then(|t| t.provider.map(|p| p.command_key().to_string()))
            .or_else(|| runs.last().and_then(|r| r.provider.clone())),
        model: task
            .and_then(|t| t.model.clone())
            .or_else(|| runs.last().and_then(|r| r.model.clone())),
        created_at_ms: task
            .map(|t| t.created_at.saturating_mul(1000))
            .or_else(|| first.map(|r| r.created_at_ms))
            .unwrap_or(0),
        started_at_ms: task
            .and_then(|t| t.started_at.map(|s| s.saturating_mul(1000)))
            .or_else(|| runs.last().and_then(|r| r.started_at_ms)),
        ended_at_ms: task
            .and_then(|t| t.finished_at.map(|s| s.saturating_mul(1000)))
            .or_else(|| runs.last().and_then(|r| r.ended_at_ms)),
        attempts: task.map(|t| t.attempts).unwrap_or_else(|| {
            runs.last().map(|r| r.attempt).unwrap_or(0)
        }),
        review_status: task
            .and_then(|t| t.review.as_ref())
            .map(|r| format!("{:?}", r.status).to_lowercase()),
        error: task.and_then(|t| t.error.clone()),
        runs: run_details,
        errors: error_details,
        interactions: interaction_details,
        total_tokens: sums.total,
        reported_cost_micros: if sums.has_cost { sums.cost_micros } else { None },
    })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CollectionStatus {
    pub enabled: bool,
    pub retention_days: i64,
    pub run_count: u64,
    pub usage_count: u64,
    pub error_count: u64,
    pub interaction_count: u64,
    /// Usage records with both tokens and cost (`complete`) vs not.
    pub usage_complete: u64,
    pub usage_partial: u64,
    /// Whether redacted error samples are being stored (off by default).
    pub capture_error_examples: bool,
    pub oldest_at_ms: Option<i64>,
    pub newest_at_ms: Option<i64>,
}

pub fn collection_status(
    store: &AnalyticsStore,
    enabled: bool,
    retention_days: i64,
    capture_error_examples: bool,
) -> CollectionStatus {
    let (oldest, newest) = {
        let mut oldest: Option<i64> = None;
        let mut newest: Option<i64> = None;
        let mut observe = |at: i64| {
            oldest = Some(oldest.map_or(at, |o| o.min(at)));
            newest = Some(newest.map_or(at, |n| n.max(at)));
        };
        for u in &store.usages {
            observe(u.occurred_at_ms);
        }
        for r in &store.runs {
            if let Some(a) = r.ended_at_ms.or(r.started_at_ms) {
                observe(a);
            }
        }
        for e in &store.errors {
            observe(e.occurred_at_ms);
        }
        for i in &store.interactions {
            observe(i.asked_at_ms);
        }
        (oldest, newest)
    };
    let usage_complete = store
        .usages
        .iter()
        .filter(|u| u.telemetry_quality == "complete")
        .count() as u64;
    CollectionStatus {
        enabled,
        retention_days,
        run_count: store.runs.len() as u64,
        usage_count: store.usages.len() as u64,
        error_count: store.errors.len() as u64,
        interaction_count: store.interactions.len() as u64,
        usage_complete,
        usage_partial: store.usages.len() as u64 - usage_complete,
        capture_error_examples,
        oldest_at_ms: oldest,
        newest_at_ms: newest,
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Escape a CSV cell, mitigating spreadsheet formula injection. A value that
/// starts with `=`, `+`, `-`, `@`, or a control character is prefixed with `'`.
pub fn csv_cell(value: &str) -> String {
    let needs_guard = value
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'));
    let guarded = if needs_guard {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if guarded.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

fn usd(micros: i64) -> String {
    format!("{:.6}", micros as f64 / 1_000_000.0)
}

/// Build a CSV of the project comparison table, including scope and provenance
/// so an exported file is not mistaken for a bill.
pub fn export_csv(
    store: &AnalyticsStore,
    filters: &AnalyticsFilters,
    names: &HashMap<String, String>,
    generated_at_ms: i64,
) -> String {
    let rows = projects(store, filters, names);
    let mut out = String::new();
    out.push_str(&format!(
        "# Solayge analytics export, generated at {generated_at_ms} (unix ms), currency USD, \
         cost basis provider-reported (not a bill)\n"
    ));
    out.push_str(
        "project,project_path,tasks_completed,tasks_failed,success_rate,avg_active_ms,median_active_ms,total_tokens,reported_cost_usd\n",
    );
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_cell(&r.project_name),
            csv_cell(&r.project_path),
            r.tasks_completed,
            r.tasks_failed,
            r.success_rate
                .map(|v| format!("{:.4}", v))
                .unwrap_or_default(),
            r.avg_active_ms.map(|v| v.to_string()).unwrap_or_default(),
            r.median_active_ms
                .map(|v| v.to_string())
                .unwrap_or_default(),
            r.total_tokens.map(|v| v.to_string()).unwrap_or_default(),
            r.reported_cost_micros.map(usd).unwrap_or_default(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn usage_of(id: &str, run: &str, occurred: i64, tokens: u64, cost_micros: i64) -> UsageRecord {
        UsageRecord {
            usage_id: format!("opencode:ses:{id}"),
            source: "opencode".into(),
            session_id: "ses".into(),
            message_id: id.into(),
            task_id: Some("t".into()),
            project_path: Some("/p".into()),
            run_id: Some(run.into()),
            occurred_at_ms: occurred,
            provider_id: Some("opencode-go".into()),
            model_id: Some("m".into()),
            variant: None,
            input_tokens: Some(tokens),
            output_tokens: Some(0),
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            provider_total_tokens: None,
            reported_cost_micros: Some(cost_micros),
            telemetry_quality: "complete".into(),
        }
    }

    fn run(task: &str, attempt: u32, status: &str, started: i64, ended: Option<i64>) -> RunRecord {
        RunRecord {
            run_id: format!("{task}:{attempt}"),
            task_id: task.into(),
            project_path: "/p".into(),
            attempt,
            kind: "agent".into(),
            provider: Some("opencode".into()),
            model: Some("m".into()),
            status: status.into(),
            created_at_ms: started - 1000,
            started_at_ms: Some(started),
            ended_at_ms: ended,
            active_ms: ended.map(|e| e - started),
            telemetry_quality: "partial".into(),
        }
    }

    fn store(runs: Vec<RunRecord>, usages: Vec<UsageRecord>) -> AnalyticsStore {
        AnalyticsStore {
            version: ANALYTICS_VERSION,
            runs,
            usages,
            errors: Vec::new(),
            interactions: Vec::new(),
        }
    }

    fn error_of(id: &str, task: &str, run: &str, text: &str, occurred: i64) -> ErrorRecord {
        let category = error_category(text, Some("shell"), "agent");
        ErrorRecord {
            error_id: id.into(),
            source: "opencode".into(),
            task_id: Some(task.into()),
            project_path: Some("/p".into()),
            run_id: Some(run.into()),
            session_id: Some("ses".into()),
            category: category.into(),
            fingerprint: error_fingerprint(category, text, Some("shell")),
            stage: "tool".into(),
            tool_name: Some("shell".into()),
            summary_redacted: None,
            occurred_at_ms: occurred,
            recovered: None,
        }
    }

    fn interaction_of(
        id: &str,
        task: &str,
        kind: &str,
        category: &str,
        asked: i64,
        resolved: Option<i64>,
    ) -> InteractionRecord {
        InteractionRecord {
            interaction_id: id.into(),
            source_confidence: "structured".into(),
            source: "opencode".into(),
            task_id: task.into(),
            project_path: "/p".into(),
            run_id: Some(format!("{task}:1")),
            session_id: Some("ses".into()),
            kind: kind.into(),
            category: category.into(),
            fingerprint: format!("{kind}:{category}"),
            asked_at_ms: asked,
            resolved_at_ms: resolved,
            blocking: true,
            outcome: resolved.map(|_| "answered".to_string()),
        }
    }

    #[test]
    fn parses_usage_from_a_realistic_opencode_message() {
        let msg = json!({
            "id": "msg_1",
            "type": "assistant",
            "model": {"id": "deepseek-v4.1-flash", "providerID": "opencode-go", "variant": "default"},
            "cost": 0.000777684,
            "tokens": {"input": 2934, "output": 93, "reasoning": 85,
                       "cache": {"read": 76928, "write": 0}},
            "time": {"created": 1791489087290i64, "completed": 1791489090422i64}
        });
        let u = usage_from_opencode_message(Some("t"), Some("/p"), Some("t:1"), "ses_1", &msg)
            .expect("usage");
        assert_eq!(u.provider_id.as_deref(), Some("opencode-go"));
        assert_eq!(u.model_id.as_deref(), Some("deepseek-v4.1-flash"));
        assert_eq!(u.variant.as_deref(), Some("default"));
        assert_eq!(u.input_tokens, Some(2934));
        assert_eq!(u.cache_read_tokens, Some(76928));
        assert_eq!(u.reported_cost_micros, Some(778)); // 0.000777684 * 1e6 rounded
        assert_eq!(u.total_tokens(), Some(2934 + 93 + 85 + 76928));
        assert_eq!(u.occurred_at_ms, 1_791_489_090_422);
        assert_eq!(u.telemetry_quality, "complete");
    }

    #[test]
    fn ignores_non_assistant_and_empty_messages() {
        assert!(usage_from_opencode_message(
            Some("t"),
            Some("/p"),
            None,
            "s",
            &json!({"id": "x", "type": "idle", "outcome": "succeeded"})
        )
        .is_none());
        // Assistant with neither tokens nor cost is not usage.
        assert!(usage_from_opencode_message(
            Some("t"),
            Some("/p"),
            None,
            "s",
            &json!({"id": "x", "type": "assistant", "tokens": {}})
        )
        .is_none());
    }

    #[test]
    fn prefers_a_provider_total_over_summing_categories() {
        let u = usage_from_opencode_message(
            None,
            None,
            None,
            "s",
            &json!({"id": "m", "type": "assistant", "tokens": {
                "total": 500, "input": 10, "output": 5,
                "cache": {"read": 9000, "write": 0}
            }}),
        )
        .expect("usage");
        assert_eq!(u.total_tokens(), Some(500));
    }

    #[test]
    fn upsert_runs_is_idempotent_and_never_downgrades_a_settled_attempt() {
        let mut s = AnalyticsStore::default();
        let r1 = run("t", 1, "succeeded", 1000, Some(2000));
        assert!(upsert_runs(&mut s, vec![r1.clone()]));
        assert!(!upsert_runs(&mut s, vec![r1.clone()]), "same record is a no-op");
        // A rerun reset the task but has not bumped attempts yet: the terminal
        // record must survive.
        let reset = run("t", 1, "ready", 0, None);
        assert!(!upsert_runs(&mut s, vec![reset]));
        assert_eq!(s.runs.len(), 1);
        assert_eq!(s.runs[0].status, "succeeded");
        assert_eq!(s.runs[0].ended_at_ms, Some(2000));
    }

    #[test]
    fn a_second_attempt_is_a_separate_run() {
        let mut s = AnalyticsStore::default();
        upsert_runs(&mut s, vec![run("t", 1, "failed", 1000, Some(2000))]);
        upsert_runs(&mut s, vec![run("t", 2, "succeeded", 3000, Some(4000))]);
        assert_eq!(s.runs.len(), 2);
        let winners = winning_runs(&s, &AnalyticsFilters::default(), None);
        assert_eq!(winners.len(), 1, "one run per task in the winner set");
        assert_eq!(winners[0].status, "succeeded");
    }

    #[test]
    fn overview_counts_terminal_tasks_and_excludes_canceled_from_the_rate() {
        let s = store(
            vec![
                run("a", 1, "succeeded", 1000, Some(2000)),
                run("b", 1, "failed", 1000, Some(2000)),
                run("c", 1, "canceled", 1000, Some(2000)),
                run("d", 1, "running", 1000, None),
            ],
            vec![],
        );
        let o = overview(&s, &AnalyticsFilters::default());
        assert_eq!(o.tasks_completed, 1);
        assert_eq!(o.tasks_failed, 1);
        assert_eq!(o.tasks_canceled, 1);
        assert_eq!(o.tasks_interrupted, 0);
        assert_eq!(o.success_sample, 2);
        assert!((o.success_rate.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(o.tasks_total, 4, "the running task is still a task");
    }

    #[test]
    fn missing_cost_is_unknown_not_zero() {
        let mut u = usage_of("m", "t:1", 1000, 5, 0);
        u.reported_cost_micros = None;
        let s = store(vec![run("t", 1, "succeeded", 1000, Some(2000))], vec![u]);
        let o = overview(&s, &AnalyticsFilters::default());
        assert_eq!(o.reported_cost_micros, None);
        assert_eq!(o.total_tokens, Some(5));
    }

    #[test]
    fn filters_exclude_out_of_range_data() {
        let s = store(
            vec![
                run("a", 1, "succeeded", 1000, Some(2000)),
                run("b", 1, "succeeded", 5000, Some(6000)),
            ],
            vec![],
        );
        let f = AnalyticsFilters {
            from_ms: Some(4000),
            to_ms: None,
            projects: vec![],
            models: vec![],
        };
        let o = overview(&s, &f);
        assert_eq!(o.tasks_completed, 1);
    }

    #[test]
    fn project_filter_limits_runs_and_usage() {
        let mut r = run("a", 1, "succeeded", 1000, Some(2000));
        r.project_path = "/other".into();
        let s = store(
            vec![r, run("b", 1, "succeeded", 1000, Some(2000))],
            vec![],
        );
        let f = AnalyticsFilters {
            projects: vec!["/p".into()],
            ..Default::default()
        };
        let o = overview(&s, &f);
        assert_eq!(o.tasks_completed, 1);
    }

    #[test]
    fn time_series_buckets_by_day_in_order() {
        let day = 86_400_000i64;
        let s = store(
            vec![
                run("a", 1, "succeeded", day, Some(day + 10)),
                run("b", 1, "failed", 2 * day, Some(2 * day + 10)),
            ],
            vec![usage_of("m", "a:1", 2 * day, 100, 1_000)],
        );
        let series = time_series(&s, &AnalyticsFilters::default());
        assert_eq!(series.len(), 2);
        assert!(series[0].bucket_ms < series[1].bucket_ms);
        assert_eq!(series[1].tasks_failed, 1);
        assert_eq!(series[1].total_tokens, Some(100));
    }

    #[test]
    fn duration_stats_average_and_median_ignore_in_progress_runs() {
        let s = store(
            vec![
                run("a", 1, "succeeded", 1000, Some(2000)),
                run("b", 1, "succeeded", 1000, Some(4000)),
                run("c", 1, "running", 1000, None),
            ],
            vec![],
        );
        let o = overview(&s, &AnalyticsFilters::default());
        assert_eq!(o.duration_sample, 2);
        assert_eq!(o.avg_active_ms, Some(2000));
        assert_eq!(o.median_active_ms, Some(2000));
    }

    #[test]
    fn models_group_by_provider_model_variant() {
        let s = store(
            vec![],
            vec![
                usage_of("m1", "t:1", 1000, 10, 1_000),
                usage_of("m2", "t:1", 2000, 20, 2_000),
            ],
        );
        let rows = models(&s, &AnalyticsFilters::default());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].inference_count, 2);
        assert_eq!(rows[0].total_tokens, Some(30));
        assert_eq!(rows[0].reported_cost_micros, Some(3_000));
        assert_eq!(rows[0].task_runs, 1);
    }

    #[test]
    fn csv_export_guards_formula_injection() {
        assert_eq!(csv_cell("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_cell("-5"), "'-5");
        assert_eq!(csv_cell("plain"), "plain");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[cfg(unix)]
    #[test]
    fn saved_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "solayge-analytics-perms-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        save_store(&dir, &AnalyticsStore::default()).unwrap();
        let mode = std::fs::metadata(dir.join("analytics.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "analytics.json must be owner-only");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retain_recent_drops_old_records_and_keeps_forever_on_zero() {
        let mut s = store(
            vec![
                run("old", 1, "succeeded", 1_000, Some(2_000)),
                run("new", 1, "succeeded", 90 * 86_400_000, Some(90 * 86_400_000 + 1)),
            ],
            vec![usage_of("m", "t:1", 2_000, 5, 1_000)],
        );
        // Keep forever: nothing removed.
        assert!(!retain_recent(&mut s, 0, 100 * 86_400_000));
        assert_eq!(s.runs.len(), 2);
        assert_eq!(s.usages.len(), 1);
        // 30-day window drops the old run; the recent one survives.
        assert!(retain_recent(&mut s, 30, 100 * 86_400_000));
        assert_eq!(s.runs.len(), 1);
        assert_eq!(s.runs[0].task_id, "new");
        assert!(s.usages.is_empty());
    }

    #[test]
    fn error_code_token_extracts_stable_signals() {
        assert_eq!(error_code_token("TS2322: Type 'x' is not assignable"), Some("TS2322".into()));
        assert_eq!(error_code_token("error[E0425]: cannot find value"), Some("E0425".into()));
        assert_eq!(error_code_token("request failed with 429"), Some("429".into()));
        assert_eq!(error_code_token("ENOENT: no such file"), Some("ENOENT".into()));
        assert_eq!(error_code_token("just some words here"), None);
    }

    #[test]
    fn error_category_matches_the_taxonomy() {
        assert_eq!(error_category("rate limit exceeded (429)", None, "agent"), "provider");
        assert_eq!(error_category("TS2322: type mismatch", None, "agent"), "build");
        assert_eq!(error_category("assertion failed: left == right", None, "agent"), "test");
        assert_eq!(error_category("ENOENT: no such file or directory", Some("read"), "agent"), "tool");
        assert_eq!(error_category("permission denied", None, "agent"), "security_policy");
        assert_eq!(error_category("merge conflict in src", None, "merge"), "integration");
        assert_eq!(error_category("totally unclear", None, "agent"), "unknown");
    }

    #[test]
    fn redaction_removes_paths_secrets_urls_and_ids() {
        assert_eq!(redact_text("/Users/me/project/src/main.rs"), "<path>");
        assert_eq!(redact_text("token sk-abc123def"), "token <secret>");
        assert_eq!(redact_text("see https://example.test/x"), "see <url>");
        assert_eq!(redact_text("id deadbeefdeadbeef1234"), "id <id>");
        assert_eq!(redact_text("code 1234567"), "code <num>");
        // Sanitized summaries are collapsed and clipped.
        let s = sanitize_summary("a\n  b\tc", 100);
        assert_eq!(s, "a b c");
    }

    #[test]
    fn redaction_covers_windows_and_unc_paths() {
        // A username embedded in a Windows/UNC path must not survive redaction.
        assert_eq!(redact_text(r"C:\Users\jon\.ssh\id_rsa"), "<path>");
        assert_eq!(redact_text(r"c:/Users/jon/secrets.txt"), "<path>");
        assert_eq!(redact_text(r"\\server\share\private.key"), "<path>");
        assert_eq!(redact_text(r"\Users\jon\file.txt"), "<path>");
        // A relative backslash path is still redacted, and plain words survive.
        assert_eq!(redact_text(r"..\..\secrets"), "<path>");
        assert_eq!(redact_text("ordinary words"), "ordinary words");
        assert_eq!(redact_text("version 2.1"), "version 2.1");
        // A single leading backslash (e.g. a regex fragment) is not a path.
        assert_eq!(redact_text(r"invalid regex \d+"), r"invalid regex \d+");
    }

    #[test]
    fn errors_from_opencode_message_extracts_tool_and_message_errors() {
        let msg = json!({
            "id": "msg_1",
            "type": "assistant",
            "time": {"completed": 1_791_489_090_422i64},
            "error": {"message": "rate limit exceeded"},
            "content": [
                {"type": "text", "text": "trying"},
                {"type": "tool", "name": "shell",
                 "state": {"status": "error", "error": "ENOENT: no such file /Users/x/y"}},
                {"type": "tool", "name": "read",
                 "state": {"status": "completed", "input": {"path": "a"}}}
            ]
        });
        let errs = errors_from_opencode_message(true, Some("t"), Some("/p"), Some("t:1"), "ses_1", &msg);
        assert_eq!(errs.len(), 2, "one message error and one tool error");
        let message_err = errs.iter().find(|e| e.error_id.ends_with(":message")).unwrap();
        assert_eq!(message_err.category, "provider");
        let tool_err = errs.iter().find(|e| e.tool_name.as_deref() == Some("shell")).unwrap();
        assert_eq!(tool_err.category, "tool");
        assert_eq!(tool_err.stage, "tool");
        assert_eq!(tool_err.occurred_at_ms, 1_791_489_090_422);
        // The captured sample is redacted at ingestion.
        assert!(tool_err.summary_redacted.as_deref().unwrap().contains("<path>"));
        // Same message polls produce identical ids, so upserts never duplicate.
        let again = errors_from_opencode_message(false, Some("t"), Some("/p"), Some("t:1"), "ses_1", &msg);
        let ids: Vec<_> = errs.iter().map(|e| &e.error_id).collect();
        let ids2: Vec<_> = again.iter().map(|e| &e.error_id).collect();
        assert_eq!(ids, ids2);
        // Examples are omitted unless opted in.
        assert!(again[0].summary_redacted.is_none());
    }

    #[test]
    fn upsert_errors_is_idempotent() {
        let mut s = AnalyticsStore::default();
        let e = error_of("e1", "t", "t:1", "ENOENT: no such file", 1000);
        assert!(upsert_errors(&mut s, vec![e.clone()]));
        assert!(!upsert_errors(&mut s, vec![e]));
        assert_eq!(s.errors.len(), 1);
    }

    #[test]
    fn errors_from_task_uses_a_deterministic_id_and_category() {
        let mut t: crate::models::Task = serde_json::from_value(json!({
            "id": "t1", "project_path": "/p", "title": "t", "prompt": "p",
            "status": "failed", "created_at": 1, "started_at": 2, "finished_at": 3,
            "attempts": 2, "kind": "shell", "error": "cargo build failed: error[E0425]"
        }))
        .expect("task");
        let e = errors_from_task(false, &t).expect("error");
        assert_eq!(e.error_id, "task:t1:2");
        assert_eq!(e.category, "build");
        assert_eq!(e.run_id.as_deref(), Some("t1:2"));
        assert_eq!(e.occurred_at_ms, 3_000);
        assert!(e.summary_redacted.is_none(), "examples off by default");
        // A non-terminal task never produces an error record.
        t.status = crate::models::TaskStatus::Running;
        assert!(errors_from_task(false, &t).is_none());
    }

    #[test]
    fn task_detail_includes_errors_and_interactions() {
        let s = AnalyticsStore {
            errors: vec![error_of("e1", "t", "t:1", "TS2322: type mismatch", 1_500)],
            interactions: vec![interaction_of("t:q1", "t", "question", "other", 1_600, Some(2_600))],
            ..store(vec![run("t", 1, "succeeded", 1_000, Some(2_000))], vec![])
        };
        let d = task_detail(&s, "t", None).expect("detail");
        assert_eq!(d.errors.len(), 1);
        assert_eq!(d.errors[0].category, "build");
        assert_eq!(d.errors[0].recovered, Some(true));
        assert_eq!(d.interactions.len(), 1);
        assert_eq!(d.interactions[0].outcome.as_deref(), Some("answered"));
    }

    #[test]
    fn model_outcomes_exclude_mixed_model_tasks() {
        // Task a used only model m; task b used both m and n.
        let mut ua = usage_of("m1", "a:1", 1_000, 10, 1_000);
        ua.model_id = Some("m".into());
        ua.task_id = Some("a".into());
        let mut ub = usage_of("m2", "b:1", 1_500, 10, 1_000);
        ub.model_id = Some("m".into());
        ub.task_id = Some("b".into());
        let mut uc = usage_of("m3", "b:1", 1_600, 10, 1_000);
        uc.model_id = Some("n".into());
        uc.task_id = Some("b".into());
        let s = store(
            vec![
                run("a", 1, "succeeded", 1_000, Some(2_000)),
                run("b", 1, "succeeded", 1_000, Some(2_000)),
            ],
            vec![ua, ub, uc],
        );
        let rows = models(&s, &AnalyticsFilters::default());
        let m = rows.iter().find(|r| r.model_id == "m").expect("m");
        assert_eq!(m.exclusive_task_runs, 1, "only task a is exclusively m");
        assert_eq!(m.exclusive_success, 1);
        assert_eq!(m.exclusive_success_rate, Some(1.0));
        assert_eq!(m.mixed_task_runs, 1, "task b is mixed");
        let n = rows.iter().find(|r| r.model_id == "n").expect("n");
        assert_eq!(n.exclusive_task_runs, 0);
        assert_eq!(n.mixed_task_runs, 1);
    }

    #[test]
    fn reliability_separates_error_occurrence_from_task_failure() {
        let s = AnalyticsStore {
            errors: vec![
                error_of("e1", "a", "a:1", "ENOENT: no such file", 1_500),
                error_of("e2", "c", "c:1", "assertion failed", 1_500),
            ],
            ..store(
                vec![
                    run("a", 1, "failed", 1_000, Some(2_000)),
                    run("a", 2, "succeeded", 3_000, Some(4_000)),
                    run("b", 1, "succeeded", 1_000, Some(2_000)),
                    run("c", 1, "failed", 1_000, Some(2_000)),
                ],
                vec![],
            )
        };
        let r = reliability(&s, &AnalyticsFilters::default());
        assert_eq!(r.summary.tasks_failed, 1, "only c's winner failed");
        assert!((r.summary.task_failure_rate.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(r.summary.runs_with_errors, 2);
        assert_eq!(r.summary.observed_runs, 4);
        assert_eq!(r.summary.error_events, 2);
        // a:1's error was recovered because a eventually succeeded; c's was not.
        assert_eq!(r.summary.recoverable_errors, 2);
        assert_eq!(r.summary.recovered_errors, 1);
        assert!((r.summary.recovery_rate.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(r.summary.retry_tasks, 1);
        assert_eq!(r.summary.attempt_tasks, 3);
        assert!((r.summary.retry_rate.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(r.summary.first_pass_success, 1);
        assert_eq!(r.summary.first_pass_sample, 3);
        assert!(r.groups.iter().any(|g| g.category == "tool"));
    }

    #[test]
    fn interactions_measure_human_wait_and_autonomy() {
        let s = AnalyticsStore {
            interactions: vec![
                interaction_of("a:q1", "a", "question", "implementation_choice", 1_000, Some(61_000)),
                interaction_of("b:p1", "b", "permission", "permissions", 2_000, None),
            ],
            ..store(
                vec![
                    run("a", 1, "succeeded", 1_000, Some(2_000)),
                    run("b", 1, "succeeded", 1_000, Some(2_000)),
                    run("c", 1, "succeeded", 1_000, Some(2_000)),
                ],
                vec![],
            )
        };
        let r = interactions(&s, &AnalyticsFilters::default());
        assert_eq!(r.summary.questions, 1);
        assert_eq!(r.summary.permissions, 1);
        assert_eq!(r.summary.resolved, 1);
        assert_eq!(r.summary.unresolved, 1);
        assert_eq!(r.summary.total_wait_ms, 60_000);
        assert_eq!(r.summary.avg_wait_ms, Some(60_000));
        // Only task c had no blocking interaction.
        assert_eq!(r.summary.autonomy_sample, 3);
        assert_eq!(r.summary.autonomy_autonomous, 1);
        assert!((r.summary.autonomy_rate.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert!(r.groups.iter().any(|g| g.fingerprint == "question:implementation_choice"));
    }

    #[test]
    fn question_classification_and_fingerprint_are_structural() {
        assert_eq!(
            question_category("Which database should I use?", None, &[], "question"),
            "implementation_choice"
        );
        assert_eq!(
            question_category("Permission: bash", None, &[], "permission"),
            "permissions"
        );
        assert_eq!(
            question_category("Need API key", None, &[], "permission"),
            "credentials"
        );
        // Fingerprint uses field keys, not their (agent-supplied) labels.
        let fp = interaction_fingerprint(
            "question",
            None,
            &["b".into(), "a".into(), "a".into()],
        );
        assert_eq!(fp, "question:a+b");
        assert_eq!(
            interaction_fingerprint("permission", Some("bash"), &[]),
            "permission:bash"
        );
    }

    #[test]
    fn reconcile_closes_open_interactions_for_ended_tasks() {
        let mut s = AnalyticsStore {
            interactions: vec![
                interaction_of("a:q1", "a", "question", "other", 1_000, None),
                interaction_of("b:q1", "b", "question", "other", 1_000, None),
            ],
            ..store(vec![run("a", 1, "failed", 1_000, Some(2_000))], vec![])
        };
        let tasks: HashMap<String, Option<i64>> =
            [("a".to_string(), Some(2_000)), ("b".to_string(), None)]
                .into_iter()
                .collect();
        assert!(reconcile_interactions_locked(&mut s, &tasks, 9_000));
        assert_eq!(s.interactions[0].resolved_at_ms, Some(2_000));
        assert_eq!(s.interactions[0].outcome.as_deref(), Some("canceled"));
        // b is still running: left open.
        assert_eq!(s.interactions[1].resolved_at_ms, None);
        // A second pass is a no-op.
        assert!(!reconcile_interactions_locked(&mut s, &tasks, 9_000));
        // A task no longer present closes at "now".
        let gone: HashMap<String, Option<i64>> = HashMap::new();
        assert!(reconcile_interactions_locked(&mut s, &gone, 9_000));
        assert_eq!(s.interactions[1].resolved_at_ms, Some(9_000));
    }

    #[test]
    fn retention_prunes_errors_and_interactions_too() {
        let day = 86_400_000i64;
        let mut s = AnalyticsStore {
            errors: vec![error_of("old", "t", "t:1", "ENOENT", 1_000)],
            interactions: vec![interaction_of("t:q", "t", "question", "other", 1_000, None)],
            ..store(vec![], vec![])
        };
        assert!(retain_recent(&mut s, 30, 100 * day));
        assert!(s.errors.is_empty());
        assert!(s.interactions.is_empty());
    }

    #[test]
    fn task_detail_aggregates_usage_across_attempts() {
        let s = store(
            vec![
                run("t", 1, "failed", 1_000, Some(2_000)),
                run("t", 2, "succeeded", 3_000, Some(4_000)),
            ],
            vec![
                usage_of("m1", "t:1", 1_500, 10, 1_000),
                usage_of("m2", "t:2", 3_500, 20, 2_000),
            ],
        );
        let d = task_detail(&s, "t", None).expect("detail");
        assert_eq!(d.attempts, 2);
        assert_eq!(d.runs.len(), 2);
        assert_eq!(d.total_tokens, Some(30));
        assert_eq!(d.reported_cost_micros, Some(3_000));
        assert_eq!(d.runs[0].total_tokens, Some(10));
        assert_eq!(d.runs[1].total_tokens, Some(20));
    }
}
