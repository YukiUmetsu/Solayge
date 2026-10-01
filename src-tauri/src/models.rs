use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Created but not released; the scheduler ignores it until Execute.
    Draft,
    Waiting,
    Ready,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Blocked,
    /// The task was mid-run when the app stopped (crash, forced quit, out of
    /// disk, or the scheduler stalled). Not a real failure of the work, but it
    /// did not finish; retrying re-queues it.
    Interrupted,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Canceled
                | TaskStatus::Blocked
                | TaskStatus::Interrupted
        )
    }
}

/// What a task actually does when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Run the agent CLI with a prompt (the original behaviour).
    #[default]
    Agent,
    /// Run a shell command, with no agent.
    Shell,
    /// A built-in git operation (commit, push, PR, merge, checkout, pull).
    Git,
    /// Combine branches/worktrees, resolve conflicts, test, and land.
    Merge,
}

/// A built-in git operation, run against the project or a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOp {
    AddCommit,
    Push,
    PrCreate,
    PrMerge,
    Checkout,
    Pull,
}

/// How an integration task combines its sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    #[default]
    Merge,
    Rebase,
    Octopus,
}

/// What to do when combining branches hits a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictMode {
    /// Stop and wait for the user to resolve it.
    #[default]
    User,
    /// Let an agent resolve, then pause for the user to review.
    AgentReview,
    /// Let an agent resolve and continue the workflow.
    AgentAuto,
}

/// For a task that runs in the project directory (not an isolated worktree),
/// whether it uses the current branch or creates a new one first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BranchMode {
    /// Run on whatever branch the project is on.
    #[default]
    Current,
    /// Create and check out a new branch before running.
    New,
}

/// The configuration of a `Merge` task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MergeSpec {
    /// Source task ids or branch names, combined in order.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Branch to land the result on (defaults to the project's default branch).
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub strategy: MergeStrategy,
    /// Command run after a clean merge (e.g. a test skill).
    #[serde(default)]
    pub test_command: Option<String>,
    /// On test failure, hand to an agent to fix, then re-run the tests.
    #[serde(default)]
    pub fix_on_failure: bool,
    /// Push the target branch after landing.
    #[serde(default)]
    pub push_target: bool,
    /// Stage and commit uncommitted work in each source's worktree before
    /// combining, so it is actually part of the branch that gets merged.
    #[serde(default)]
    pub commit_sources: bool,
}

/// The pre-combine state of one source: the branch and worktree it refers to,
/// and whether that worktree holds uncommitted work the merge would otherwise
/// leave out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSourceStatus {
    /// The source as given (a task id or a branch name).
    pub source: String,
    /// The branch the source resolves to, when known.
    #[serde(default)]
    pub branch: Option<String>,
    /// The worktree checked out to that branch, when one exists.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Whether the worktree has staged, unstaged, or untracked changes.
    pub dirty: bool,
    /// How many files are changed in the worktree.
    pub changed: usize,
}

/// One branch's landing state relative to the repository's default branch.
/// Powers the Git management view, so it carries the extra bits the list needs
/// (default/current flags, remote existence, and the worktree using it) rather
/// than making the UI re-derive them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Short name: `foo` for both `refs/heads/foo` and `origin/foo`.
    pub name: String,
    /// True for a remote-tracking branch with no local counterpart.
    pub is_remote: bool,
    /// True for the repository's resolved default branch.
    pub is_default: bool,
    /// True when this is the branch currently checked out in the project folder.
    pub is_current: bool,
    /// True when the default branch already contains every commit here, so the
    /// branch has nothing left to land.
    pub merged: bool,
    /// Commits on this branch that the default branch does not have.
    pub ahead: i64,
    /// Commits on the default branch that this branch does not have.
    pub behind: i64,
    /// Worktree where this branch is checked out, when one exists.
    #[serde(default)]
    pub worktree: Option<String>,
    /// A remote-tracking branch `origin/<name>` exists.
    pub has_remote: bool,
    /// The branch's worktree (or the project folder, for the checked-out
    /// branch) has uncommitted changes that are not on the branch. Surfaced so a
    /// branch is never shown as cleanly "merged" while its work sits uncommitted.
    #[serde(default)]
    pub dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    #[default]
    Worktree,
    Shared,
}

/// Controls how much an agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    /// Pre-grant broadly and run with `--auto` (rails still apply).
    #[default]
    Autonomous,
    /// Pre-grant reads/edits; ask for shell/network. Without an approver these
    /// are auto-rejected and surfaced as notifications.
    Supervised,
    /// Read/analyze only; no edits or shell.
    Readonly,
}

/// Coding-agent CLI that runs the tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Opencode,
    Codex,
    Claude,
    Cursor,
}

impl Provider {
    /// Every provider, in display order. Kept next to the enum so adding a
    /// variant and forgetting a list is a compile error, not a silent omission.
    pub const ALL: [Provider; 4] = [
        Provider::Opencode,
        Provider::Codex,
        Provider::Claude,
        Provider::Cursor,
    ];

    pub fn command_key(self) -> &'static str {
        match self {
            Provider::Opencode => "opencode",
            Provider::Codex => "codex",
            Provider::Claude => "claude",
            Provider::Cursor => "cursor",
        }
    }
}

/// What to do after a task succeeds and auto-review is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    /// No review.
    #[default]
    Off,
    /// Review and record the result; never blocks.
    Report,
    /// Review and let the reviewer fix issues, then continue.
    Autofix,
    /// Review; if issues are found, stop the task and ask for attention.
    Pause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    #[default]
    None,
    Pending,
    Running,
    Passed,
    Issues,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReview {
    pub mode: ReviewMode,
    #[serde(default)]
    pub status: ReviewStatus,
    #[serde(default)]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub finished_at: Option<i64>,
}

/// Whether an agent is asking a question or requesting permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// The agent asked the user a question (opencode's `question` tool).
    Question,
    /// The agent wants to run a tool/command and needs approval.
    Permission,
}

/// The widget type for one question field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskFieldKind {
    String,
    Number,
    Integer,
    Boolean,
    Multiselect,
    /// Answered outside the form (e.g. an OAuth URL the agent hands over).
    External,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskOption {
    pub value: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// One field of a question, mapped from opencode's form schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskField {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    pub kind: AskFieldKind,
    #[serde(default)]
    pub required: bool,
    /// Whether the user may answer with something other than `options` (opencode's
    /// `custom` flag). The UI shows a free-text input alongside the choices.
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// String format hint: `email`, `uri`, `date`, or `date-time`.
    #[serde(default)]
    pub format: Option<String>,
    /// Numeric bounds for `number`/`integer` fields.
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    /// String length bounds.
    #[serde(default)]
    pub min_length: Option<u64>,
    #[serde(default)]
    pub max_length: Option<u64>,
    #[serde(default)]
    pub pattern: Option<String>,
    /// Bounds on the number of multiselect entries.
    #[serde(default)]
    pub min_items: Option<u64>,
    #[serde(default)]
    pub max_items: Option<u64>,
    /// For an `external` field: where the user completes it.
    #[serde(default)]
    pub url: Option<String>,
}

/// A pending question or permission request from an agent. While one is set the
/// task is `blocked` (waiting for the user), not failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAsk {
    /// Provider-side id (opencode form id / permission request id).
    pub id: String,
    pub kind: AskKind,
    pub title: String,
    #[serde(default)]
    pub message: Option<String>,
    /// For a permission ask: the provider action (e.g. `external_directory`).
    #[serde(default)]
    pub action: Option<String>,
    /// For a permission ask: the concrete targets being requested (a directory,
    /// command, URL, …), one per entry so the UI can list them on their own lines.
    #[serde(default)]
    pub resources: Vec<String>,
    /// For a permission ask: what an `always` reply would remember.
    #[serde(default)]
    pub save: Vec<String>,
    /// For a permission ask: provider-supplied details (tool, command, args, …).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// For a permission ask: the tool call that triggered the request.
    #[serde(default)]
    pub source: Option<serde_json::Value>,
    /// For a permission ask: a plain-language reason the agent gave.
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub fields: Vec<AskField>,
    /// Permission decisions offered by the provider (`once`, `always`, `reject`).
    #[serde(default)]
    pub options: Vec<String>,
    /// The provider session this ask belongs to (for opencode, `ses_…`).
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_path: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub isolation: Isolation,
    #[serde(default)]
    pub profile: PermissionProfile,
    #[serde(default)]
    pub last_permission: Option<String>,
    /// The live opencode session backing this task while it runs. The UI uses it
    /// to let the user send the agent messages; cleared when the task stops.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
    /// Unix seconds before which this task must not start.
    #[serde(default)]
    pub not_before: Option<i64>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub finished_at: Option<i64>,

    // ---- agent configuration, resolved when the task is created ----
    #[serde(default)]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<Provider>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// Whether the backup provider/model has already been tried.
    #[serde(default)]
    pub used_fallback: bool,
    /// Auto code review state, when enabled for this task.
    #[serde(default)]
    pub review: Option<TaskReview>,

    // ---- non-agent execution ----
    #[serde(default)]
    pub kind: TaskKind,
    /// The git operation, when `kind` is `Git`.
    #[serde(default)]
    pub git_op: Option<GitOp>,
    /// A shell command (for `Shell`), or the commit/PR text for a git task.
    #[serde(default)]
    pub command: Option<String>,
    /// The integration configuration, when `kind` is `Merge`.
    #[serde(default)]
    pub merge: Option<MergeSpec>,
    /// For non-worktree tasks: current branch or a new one.
    #[serde(default)]
    pub branch_mode: BranchMode,
    /// Requested new-branch name; empty lets the agent choose one.
    #[serde(default)]
    pub new_branch: Option<String>,
    /// A question or permission request the agent is waiting on, if any. While
    /// set, the task is `blocked`.
    #[serde(default)]
    pub ask: Option<TaskAsk>,
    /// The agent's final markdown summary, captured when its run ends. Rendered
    /// as the task's Result tab and persisted so it can be re-read later.
    #[serde(default)]
    pub result: Option<String>,
}

impl Task {
    /// Whether the pending ask (if any) can still be answered. An ask is only
    /// real while the task is `running`: once the task leaves `running` its
    /// provider session is gone, so the prompt must not be shown or answered.
    pub fn can_answer_ask(&self) -> bool {
        self.status == TaskStatus::Running && self.ask.is_some()
    }

    /// Drop an ask that is no longer answerable because the task stopped.
    /// Returns `true` when a stale ask was cleared.
    ///
    /// Every transition out of `running` must leave the task in this shape.
    /// Enforcing it here gives the UI a single rule to rely on.
    pub fn drop_orphaned_ask(&mut self) -> bool {
        if self.ask.is_some() && !self.can_answer_ask() {
            self.ask = None;
            true
        } else {
            false
        }
    }
}

/// A project environment variable. The value is never stored here — it lives
/// encrypted in the secret store (OS keychain or an encrypted local file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEnvVar {
    pub key: String,
    /// Whether the value is masked in the UI. All values are encrypted at rest
    /// regardless; this only controls display.
    #[serde(default)]
    pub secret: bool,
}

/// A named command that belongs to a project and can be turned into a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSkill {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptPosition {
    #[default]
    Prefix,
    Suffix,
}

/// Text injected before or after every task prompt for a project. Supports
/// `{{project_name}}`, `{{project_path}}`, `{{current_branch}}` and
/// `{{env.NAME}}` (project env vars, then the process environment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPrompt {
    #[serde(default)]
    pub position: SystemPromptPosition,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub path: String,
    pub name: String,
    pub added_at: i64,
    #[serde(default)]
    pub default_base_ref: Option<String>,
    #[serde(default)]
    pub default_profile: Option<PermissionProfile>,

    // ---- per-project agent configuration (falls back to Settings) ----
    #[serde(default)]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<Provider>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default)]
    pub review_provider: Option<Provider>,
    #[serde(default)]
    pub review_model: Option<String>,
    #[serde(default)]
    pub review_mode: Option<ReviewMode>,
    /// Custom instructions for the auto code reviewer. `None` uses the built-in
    /// skeptical prompt.
    #[serde(default)]
    pub review_prompt: Option<String>,
    /// Editor id used by the "open project" button (vscode, cursor, zed, …).
    #[serde(default)]
    pub editor: Option<String>,

    // ---- per-project context applied to every agent run ----
    /// Environment variable names; values live in the encrypted secret store.
    #[serde(default)]
    pub env_vars: Vec<ProjectEnvVar>,
    /// Named commands the project can run as tasks.
    #[serde(default)]
    pub skills: Vec<ProjectSkill>,
    /// Optional text prepended or appended to every task prompt.
    #[serde(default)]
    pub system_prompt: Option<SystemPrompt>,
    /// How merge conflicts are handled (defaults to waiting for the user).
    #[serde(default)]
    pub conflict_mode: Option<ConflictMode>,
}

/// A project env var as sent by the UI, including its (decrypted) value.
#[derive(Debug, Clone, Deserialize)]
pub struct EnvVarInput {
    pub key: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub value: String,
}

/// A decrypted env var value returned to the UI for editing.
#[derive(Debug, Clone, Serialize)]
pub struct EnvValue {
    pub key: String,
    pub value: String,
    pub secret: bool,
}

/// The full per-project agent configuration, as sent by the UI.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<Provider>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default)]
    pub review_provider: Option<Provider>,
    #[serde(default)]
    pub review_model: Option<String>,
    #[serde(default)]
    pub review_mode: Option<ReviewMode>,
    #[serde(default)]
    pub review_prompt: Option<String>,
    #[serde(default)]
    pub editor: Option<String>,
    /// Omitted fields leave the existing value untouched.
    #[serde(default)]
    pub env_vars: Option<Vec<EnvVarInput>>,
    #[serde(default)]
    pub skills: Option<Vec<ProjectSkill>>,
    #[serde(default)]
    pub system_prompt: Option<SystemPrompt>,
    #[serde(default)]
    pub conflict_mode: Option<ConflictMode>,
}

fn opencode_command() -> String {
    "opencode run --standalone {auto} [--model {model}] {prompt}".to_string()
}
fn codex_command() -> String {
    "codex exec [--model {model}] {prompt}".to_string()
}
fn claude_command() -> String {
    "claude -p {prompt} [--model {model}]".to_string()
}
fn cursor_command() -> String {
    "cursor-agent -p {prompt} [--model {model}]".to_string()
}

/// How each provider CLI is invoked. `{prompt}` is one argument, `{model}` and
/// `{auto}` are substituted where present, and `[ … ]` groups are dropped when
/// their placeholders are empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandTemplates {
    #[serde(default = "opencode_command")]
    pub opencode: String,
    #[serde(default = "codex_command")]
    pub codex: String,
    #[serde(default = "claude_command")]
    pub claude: String,
    #[serde(default = "cursor_command")]
    pub cursor: String,
}

impl Default for CommandTemplates {
    fn default() -> Self {
        CommandTemplates {
            opencode: opencode_command(),
            codex: codex_command(),
            claude: claude_command(),
            cursor: cursor_command(),
        }
    }
}

impl CommandTemplates {
    pub fn for_provider(&self, provider: Provider) -> &str {
        match provider {
            Provider::Opencode => &self.opencode,
            Provider::Codex => &self.codex,
            Provider::Claude => &self.claude,
            Provider::Cursor => &self.cursor,
        }
    }
}

fn default_concurrency() -> usize {
    3
}

fn default_retention_days() -> i64 {
    30
}

fn default_true() -> bool {
    true
}

fn default_volume() -> f32 {
    0.8
}

fn default_sound_complete() -> Option<String> {
    Some("complete".to_string())
}

fn default_sound_failed() -> Option<String> {
    Some("failed".to_string())
}

fn default_sound_review() -> Option<String> {
    Some("review".to_string())
}

fn default_sound_attention() -> Option<String> {
    Some("attention".to_string())
}

/// The kinds of app event that can raise a notification. Serialized
/// snake_case so the frontend can switch on the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    /// A task finished successfully.
    TaskComplete,
    /// A task failed.
    TaskFailed,
    /// A task entered (or finished) automatic review.
    TaskReview,
    /// Something needs the user: a question, permission, block, or interruption.
    NeedsAttention,
    /// An app-level problem (scheduler, launch). Only gated by the master toggle.
    System,
}

/// The payload sent to the frontend over `app://notify`, where the in-app toast
/// and the notification sound are produced.
#[derive(Debug, Clone, Serialize)]
pub struct NotifyEvent {
    pub kind: NotifyKind,
    pub title: String,
    pub body: String,
}

/// Which events notify, how they sound, and how loud. All on by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// Master switch for every notification.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub on_task_complete: bool,
    #[serde(default = "default_true")]
    pub on_task_failed: bool,
    #[serde(default = "default_true")]
    pub on_task_review: bool,
    #[serde(default = "default_true")]
    pub on_needs_attention: bool,
    /// Play a sound alongside a notification.
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    /// Playback volume, `0.0`..=`1.0`.
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// A preset id (`complete`, `failed`, `review`, `attention`) or a
    /// `file:<absolute path>` reference. `None` is silent.
    #[serde(default = "default_sound_complete")]
    pub complete_sound: Option<String>,
    #[serde(default = "default_sound_failed")]
    pub failed_sound: Option<String>,
    #[serde(default = "default_sound_review")]
    pub review_sound: Option<String>,
    #[serde(default = "default_sound_attention")]
    pub attention_sound: Option<String>,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        NotificationSettings {
            enabled: true,
            on_task_complete: true,
            on_task_failed: true,
            on_task_review: true,
            on_needs_attention: true,
            sound_enabled: true,
            volume: default_volume(),
            complete_sound: default_sound_complete(),
            failed_sound: default_sound_failed(),
            review_sound: default_sound_review(),
            attention_sound: default_sound_attention(),
        }
    }
}

impl NotificationSettings {
    /// Whether an event of this kind should notify at all.
    pub fn allows(&self, kind: NotifyKind) -> bool {
        if !self.enabled {
            return false;
        }
        match kind {
            NotifyKind::TaskComplete => self.on_task_complete,
            NotifyKind::TaskFailed => self.on_task_failed,
            NotifyKind::TaskReview => self.on_task_review,
            NotifyKind::NeedsAttention => self.on_needs_attention,
            NotifyKind::System => true,
        }
    }
}

/// App-wide preferences, persisted with the rest of the state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// How long cached prompt history / task logs are kept, in days.
    /// `0` means keep forever.
    #[serde(default = "default_retention_days")]
    pub cache_retention_days: i64,
    /// Fallback permission profile for projects with no default of their own.
    #[serde(default)]
    pub default_profile: Option<PermissionProfile>,

    // ---- account-wide agent defaults (projects override these) ----
    #[serde(default)]
    pub provider: Option<Provider>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<Provider>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    #[serde(default)]
    pub review_provider: Option<Provider>,
    #[serde(default)]
    pub review_model: Option<String>,
    #[serde(default)]
    pub review_mode: ReviewMode,
    /// Account-wide default instructions for the auto code reviewer. Projects
    /// override it; `None` uses the built-in skeptical prompt.
    #[serde(default)]
    pub review_prompt: Option<String>,
    /// Default editor id for the "open project" button.
    #[serde(default)]
    pub editor: Option<String>,
    #[serde(default)]
    pub command_templates: CommandTemplates,
    /// Where encrypted secrets live: `"keychain"` (OS-native) or `"file"`
    /// (an AES-encrypted local file). `None` picks the OS-native store when one
    /// is available, otherwise the file store.
    #[serde(default)]
    pub secret_store: Option<String>,
    /// Desktop notification + sound preferences.
    #[serde(default)]
    pub notifications: NotificationSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            cache_retention_days: default_retention_days(),
            default_profile: None,
            provider: None,
            model: None,
            fallback_provider: None,
            fallback_model: None,
            review_provider: None,
            review_model: None,
            review_mode: ReviewMode::Off,
            review_prompt: None,
            editor: None,
            command_templates: CommandTemplates::default(),
            secret_store: None,
            notifications: NotificationSettings::default(),
        }
    }
}

/// The agent configuration actually in effect for a task.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedConfig {
    pub provider: Provider,
    pub model: Option<String>,
    pub fallback_provider: Option<Provider>,
    pub fallback_model: Option<String>,
    pub review_provider: Provider,
    pub review_model: Option<String>,
    pub review_mode: ReviewMode,
    pub review_prompt: Option<String>,
    pub editor: Option<String>,
}

// ---- preflight environment check ----

/// Whether a tool is available on `PATH`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    pub path: Option<String>,
    /// Extra warning, e.g. a Windows `.cmd`/`.bat` shim needing `cmd /C`.
    pub note: Option<String>,
}

/// A provider and whether its configured CLI is available.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderTool {
    pub provider: Provider,
    /// The binary named first in the provider's command template.
    pub command: String,
    pub found: bool,
    pub path: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentStatus {
    pub git: ToolStatus,
    pub gh: ToolStatus,
    pub providers: Vec<ProviderTool>,
}

/// A previously used prompt (task prompt or planner goal), kept as cache so it
/// can be re-used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptEntry {
    pub id: String,
    #[serde(default)]
    pub project_path: Option<String>,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub profile: Option<PermissionProfile>,
    #[serde(default)]
    pub isolation: Option<Isolation>,
    pub created_at: i64,
    #[serde(default = "default_uses")]
    pub uses: u64,
}

fn default_uses() -> u64 {
    1
}

/// Sizes of the on-disk cache, shown in Settings.
#[derive(Debug, Clone, Serialize)]
pub struct CacheStats {
    pub prompt_count: usize,
    pub oldest_prompt: Option<i64>,
    pub log_count: usize,
    pub log_bytes: u64,
    pub retention_days: i64,
}

/// A soft-deleted task: the task record (prompt, details, dependencies), plus
/// the small pieces worth keeping once the full log and worktree are gone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedTask {
    pub task: Task,
    pub deleted_at: i64,
    /// Tail of the task's log when it was deleted (its "output summary").
    #[serde(default)]
    pub summary: Option<String>,
    /// Working-tree diff captured when it was deleted.
    #[serde(default)]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub prompts: Vec<PromptEntry>,
    /// Soft-deleted tasks, kept so they can be restored. Capped per project.
    #[serde(default)]
    pub deleted_tasks: Vec<DeletedTask>,
}

impl Default for PersistedState {
    fn default() -> Self {
        PersistedState {
            projects: Vec::new(),
            tasks: Vec::new(),
            concurrency: default_concurrency(),
            settings: Settings::default(),
            prompts: Vec::new(),
            deleted_tasks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub concurrency: usize,
    pub running: usize,
    pub settings: Settings,
    /// Soft-deleted tasks (all projects; the UI filters by project).
    pub deleted_tasks: Vec<DeletedTask>,
}

// ---- git DTOs ----

#[derive(Debug, Clone, Serialize)]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatus {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub dirty: bool,
    pub changed_files: Vec<ChangedFile>,
    pub ahead: i64,
    pub behind: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub is_main: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDiff {
    pub path: String,
    pub status: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    pub stat: String,
    pub files: Vec<FileDiff>,
}

// ---- planning DTOs ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub delay_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDraft {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tasks: Vec<PlanTask>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanResult {
    pub summary: String,
    pub tasks: Vec<PlanTask>,
    pub raw: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewTask {
    #[serde(default)]
    pub local_id: Option<String>,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub isolation: Option<Isolation>,
    #[serde(default)]
    pub profile: Option<PermissionProfile>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub delay_seconds: Option<i64>,
    #[serde(default)]
    pub base_ref: Option<String>,
    // ---- non-agent execution ----
    #[serde(default)]
    pub kind: Option<TaskKind>,
    #[serde(default)]
    pub git_op: Option<GitOp>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub merge: Option<MergeSpec>,
    #[serde(default)]
    pub branch_mode: Option<BranchMode>,
    #[serde(default)]
    pub new_branch: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub isolation: Option<Isolation>,
    #[serde(default)]
    pub profile: Option<PermissionProfile>,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub delay_seconds: Option<i64>,
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    #[serde(default)]
    pub command: Option<String>,
    /// For non-worktree tasks: run on the current branch, or create one first.
    #[serde(default)]
    pub branch_mode: Option<BranchMode>,
    /// Requested new-branch name (empty lets the agent choose one).
    #[serde(default)]
    pub new_branch: Option<String>,
    /// What the task does (agent prompt, shell command, git op, merge).
    #[serde(default)]
    pub kind: Option<TaskKind>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub task_id: String,
    pub stream: String,
    pub line: String,
    /// Coarse kind for the UI (`"text"`, `"tool"`, or `"note"`), so the log can
    /// be grouped/filtered without re-parsing the line.
    pub kind: String,
}

#[cfg(test)]
mod tests {
    use super::{NotificationSettings, NotifyKind, Settings, Task, TaskStatus};

    /// Every status a task can have. Used to check rules that must hold for all
    /// of them; add any new status here and the invariant tests below cover it.
    const ALL_STATUSES: [TaskStatus; 9] = [
        TaskStatus::Draft,
        TaskStatus::Waiting,
        TaskStatus::Ready,
        TaskStatus::Running,
        TaskStatus::Succeeded,
        TaskStatus::Failed,
        TaskStatus::Canceled,
        TaskStatus::Blocked,
        TaskStatus::Interrupted,
    ];

    fn task_with_ask(status: TaskStatus) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": "t",
            "project_path": "/p",
            "title": "t",
            "prompt": "p",
            "status": status,
            "created_at": 1,
            "ask": {
                "id": "frm_1",
                "kind": "question",
                "title": "Which environment?",
                "session_id": "ses_1"
            }
        }))
        .expect("a valid task")
    }

    #[test]
    fn terminal_statuses_are_the_six_finished_ones() {
        for s in [
            TaskStatus::Succeeded,
            TaskStatus::Failed,
            TaskStatus::Canceled,
            TaskStatus::Blocked,
            TaskStatus::Interrupted,
        ] {
            assert!(s.is_terminal(), "{s:?} should be terminal");
        }
        for s in [
            TaskStatus::Draft,
            TaskStatus::Waiting,
            TaskStatus::Ready,
            TaskStatus::Running,
        ] {
            assert!(!s.is_terminal(), "{s:?} should not be terminal");
        }
    }

    #[test]
    fn every_status_is_covered_by_the_invariant_tests() {
        // Guards against adding a status and forgetting to extend the tests:
        // `is_terminal` and the ask rule are checked against the same set.
        assert_eq!(ALL_STATUSES.len(), 9);
        assert!(ALL_STATUSES.iter().any(|s| s.is_terminal()));
        assert!(ALL_STATUSES.iter().any(|s| !s.is_terminal()));
    }

    #[test]
    fn an_ask_is_only_answerable_while_running() {
        for status in ALL_STATUSES {
            let t = task_with_ask(status);
            assert_eq!(
                t.can_answer_ask(),
                status == TaskStatus::Running,
                "can_answer_ask must be false for {status:?}"
            );
        }
    }

    #[test]
    fn drop_orphaned_ask_clears_it_for_every_stopped_status() {
        for status in ALL_STATUSES {
            let mut t = task_with_ask(status);
            let cleared = t.drop_orphaned_ask();

            if status == TaskStatus::Running {
                assert!(!cleared, "a running task's ask must be kept");
                assert!(t.ask.is_some());
            } else {
                assert!(cleared, "{status:?} should have dropped its ask");
                assert!(t.ask.is_none(), "{status:?} kept an orphaned ask");
            }
        }
    }

    #[test]
    fn drop_orphaned_ask_is_a_no_op_without_an_ask() {
        for status in ALL_STATUSES {
            let mut t = task_with_ask(status);
            t.ask = None;
            assert!(!t.drop_orphaned_ask());
            assert!(t.ask.is_none());
        }
    }

    /// The event kinds the frontend switches on. If one is added, the default
    /// and gating tests below must cover it too.
    const ALL_KINDS: [NotifyKind; 5] = [
        NotifyKind::TaskComplete,
        NotifyKind::TaskFailed,
        NotifyKind::TaskReview,
        NotifyKind::NeedsAttention,
        NotifyKind::System,
    ];

    #[test]
    fn notifications_are_allowed_by_default() {
        let n = NotificationSettings::default();
        for kind in ALL_KINDS {
            assert!(n.allows(kind), "{kind:?} should notify by default");
        }
    }

    #[test]
    fn the_master_switch_silences_every_kind() {
        let n = NotificationSettings {
            enabled: false,
            ..Default::default()
        };
        for kind in ALL_KINDS {
            assert!(!n.allows(kind), "{kind:?} must be silenced by the master switch");
        }
    }

    #[test]
    fn each_event_toggle_gates_only_its_own_kind() {
        let complete = NotificationSettings {
            on_task_complete: false,
            ..Default::default()
        };
        assert!(!complete.allows(NotifyKind::TaskComplete));
        assert!(complete.allows(NotifyKind::TaskFailed));

        let failed = NotificationSettings {
            on_task_failed: false,
            ..Default::default()
        };
        assert!(!failed.allows(NotifyKind::TaskFailed));
        assert!(failed.allows(NotifyKind::TaskComplete));

        let review = NotificationSettings {
            on_task_review: false,
            ..Default::default()
        };
        assert!(!review.allows(NotifyKind::TaskReview));
        assert!(review.allows(NotifyKind::TaskComplete));

        let attention = NotificationSettings {
            on_needs_attention: false,
            ..Default::default()
        };
        assert!(!attention.allows(NotifyKind::NeedsAttention));
        assert!(attention.allows(NotifyKind::TaskComplete));
    }

    #[test]
    fn system_notifications_ignore_the_per_event_toggles() {
        // App-level problems have no dedicated toggle: only the master switch
        // controls them, so they still surface when everything else is muted.
        let n = NotificationSettings {
            on_task_complete: false,
            on_task_failed: false,
            on_task_review: false,
            on_needs_attention: false,
            ..Default::default()
        };
        assert!(n.allows(NotifyKind::System));
    }

    #[test]
    fn missing_notification_fields_fall_back_to_defaults() {
        let n: NotificationSettings =
            serde_json::from_value(serde_json::json!({})).expect("empty object is valid");
        assert!(n.enabled);
        assert!(n.on_task_complete);
        assert!(n.sound_enabled);
        assert!((n.volume - 0.8).abs() < f32::EPSILON);
        assert_eq!(n.complete_sound.as_deref(), Some("complete"));
        assert_eq!(n.failed_sound.as_deref(), Some("failed"));
        assert_eq!(n.review_sound.as_deref(), Some("review"));
        assert_eq!(n.attention_sound.as_deref(), Some("attention"));

        let partial: NotificationSettings =
            serde_json::from_value(serde_json::json!({ "enabled": false }))
                .expect("a partial object is valid");
        assert!(!partial.enabled);
        assert!(partial.on_task_complete, "unspecified toggles keep their default");
        assert_eq!(partial.complete_sound.as_deref(), Some("complete"));
    }

    #[test]
    fn settings_from_older_state_gains_notification_defaults() {
        // State written before the notifications feature has no `notifications`
        // key; it must load with defaults rather than failing.
        let s: Settings = serde_json::from_value(serde_json::json!({})).expect("valid settings");
        assert!(s.notifications.enabled);
        let round_tripped = serde_json::to_value(&s).expect("serializes");
        assert!(round_tripped.get("notifications").is_some());
    }

    #[test]
    fn notify_kind_matches_the_frontend_wire_contract() {
        let cases = [
            (NotifyKind::TaskComplete, "task_complete"),
            (NotifyKind::TaskFailed, "task_failed"),
            (NotifyKind::TaskReview, "task_review"),
            (NotifyKind::NeedsAttention, "needs_attention"),
            (NotifyKind::System, "system"),
        ];
        for (kind, wire) in cases {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(wire));
        }
    }
}
