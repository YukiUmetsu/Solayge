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
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Canceled | TaskStatus::Blocked
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
            editor: None,
            command_templates: CommandTemplates::default(),
            secret_store: None,
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
    pub editor: Option<String>,
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
}

impl Default for PersistedState {
    fn default() -> Self {
        PersistedState {
            projects: Vec::new(),
            tasks: Vec::new(),
            concurrency: default_concurrency(),
            settings: Settings::default(),
            prompts: Vec::new(),
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
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub task_id: String,
    pub stream: String,
    pub line: String,
}
