export type TaskStatus =
  | "draft"
  | "waiting"
  | "ready"
  | "running"
  | "succeeded"
  | "failed"
  | "canceled"
  | "blocked"
  | "interrupted";

export type Isolation = "worktree" | "shared";

export type PermissionProfile = "autonomous" | "supervised" | "readonly";

export type Provider = "opencode" | "codex" | "claude" | "cursor";

export type ReviewMode = "off" | "report" | "autofix" | "pause";

export type ReviewStatus =
  | "none"
  | "pending"
  | "running"
  | "passed"
  | "issues"
  | "failed";

/** Whether an agent is asking a question or requesting permission. */
export type AskKind = "question" | "permission";

/** The widget type for one question field. */
export type AskFieldKind =
  | "string"
  | "number"
  | "integer"
  | "boolean"
  | "multiselect"
  | "external";

export interface AskOption {
  value: string;
  label: string;
  description?: string | null;
}

export interface AskField {
  key: string;
  label: string;
  description?: string | null;
  kind: AskFieldKind;
  required: boolean;
  /** Whether a value outside `options` may be typed (opencode's `custom`). */
  custom: boolean;
  options: AskOption[];
  default?: unknown;
  placeholder?: string | null;
  /** `email` | `uri` | `date` | `date-time`. */
  format?: string | null;
  min?: number | null;
  max?: number | null;
  min_length?: number | null;
  max_length?: number | null;
  pattern?: string | null;
  min_items?: number | null;
  max_items?: number | null;
  /** For an `external` field: where it is completed. */
  url?: string | null;
}

/** A pending question or permission request from an agent. */
export interface TaskAsk {
  id: string;
  kind: AskKind;
  title: string;
  message?: string | null;
  /** For a permission ask: the provider action, e.g. `external_directory`. */
  action?: string | null;
  /** For a permission ask: the exact directories/commands/URLs requested. */
  resources: string[];
  /** For a permission ask: what an `always` reply would remember. */
  save: string[];
  /** For a permission ask: provider-supplied details (tool, command, …). */
  metadata?: unknown;
  /** For a permission ask: the tool call that triggered the request. */
  source?: unknown;
  /** For a permission ask: a plain-language reason. */
  purpose?: string | null;
  fields: AskField[];
  options: string[];
  session_id?: string | null;
  created_at?: number | null;
  /** Unix seconds after which the agent stops waiting, when it supplied one. */
  deadline?: number | null;
  /** A provider-supplied relative wait in seconds instead of a deadline. */
  timeout_secs?: number | null;
}

export interface TaskReview {
  mode: ReviewMode;
  status: ReviewStatus;
  provider?: Provider | null;
  model?: string | null;
  summary?: string | null;
  started_at?: number | null;
  finished_at?: number | null;
  /** How many times the review has been started (automatic retries included). */
  attempts?: number;
  /** Per-run reviewer instructions set when the user edits the prompt before a
   *  re-run; `null` uses the project/account review prompt. */
  prompt?: string | null;
}

export type SystemPromptPosition = "prefix" | "suffix";

/** A project environment variable. The value lives in the encrypted store. */
export interface ProjectEnvVar {
  key: string;
  secret: boolean;
}

/** A decrypted env var value, returned for editing in project settings. */
export interface EnvValue extends ProjectEnvVar {
  value: string;
}

/** A named command that belongs to a project and can be turned into a task. */
export interface ProjectSkill {
  id: string;
  name: string;
  command: string;
  description: string;
}

/** Text injected before or after every task prompt for a project. */
export interface SystemPrompt {
  position: SystemPromptPosition;
  text: string;
  enabled: boolean;
}

/** Where encrypted secrets are stored. */
export type SecretStore = "keychain" | "file";

/** Preflight: whether a tool is available on PATH. */
export interface ToolStatus {
  name: string;
  found: boolean;
  path?: string | null;
  note?: string | null;
}

export interface ProviderTool {
  provider: Provider;
  /** The binary named first in the provider's command template. */
  command: string;
  found: boolean;
  path?: string | null;
  note?: string | null;
}

export interface EnvironmentStatus {
  git: ToolStatus;
  gh: ToolStatus;
  providers: ProviderTool[];
}

/** What a task does when it runs. */
export type TaskKind = "agent" | "shell" | "git" | "merge";

/** A built-in git operation. */
export type GitOp =
  | "add_commit"
  | "push"
  | "pr_create"
  | "pr_merge"
  | "checkout"
  | "pull";

/** How an integration task combines its sources. */
export type MergeStrategy = "merge" | "rebase" | "octopus";

/** How merge conflicts are handled for a project. */
export type ConflictMode = "user" | "agent_review" | "agent_auto";

/** For non-worktree tasks: run on the current branch or create a new one. */
export type BranchMode = "current" | "new";

/**
 * The user-facing separation choice for a task: an isolated worktree, a fresh
 * branch in the project folder, or the project's current branch. The mapping to
 * `isolation` + `branch_mode` lives in `lib/providers.ts`.
 */
export type Separation = "worktree" | "branch" | "current";

/** The configuration of a `merge` task. */
export interface MergeSpec {
  /** Source task ids or branch names, combined in order. */
  sources: string[];
  target?: string | null;
  strategy: MergeStrategy;
  /** Command run after a clean merge (e.g. a test skill). */
  test_command?: string | null;
  fix_on_failure: boolean;
  push_target: boolean;
  /** Commit uncommitted work in each source worktree before combining. */
  commit_sources: boolean;
}

/** The pre-combine state of one source selected for a merge. */
export interface MergeSourceStatus {
  /** The source as given (a task id or a branch name). */
  source: string;
  branch?: string | null;
  worktree?: string | null;
  /** Whether the worktree has uncommitted work the merge would otherwise miss. */
  dirty: boolean;
  changed: number;
}

export interface CommandTemplates {
  opencode: string;
  codex: string;
  claude: string;
  cursor: string;
}

/** The event kinds the backend can raise a notification for. */
export type NotifyKind =
  | "task_complete"
  | "task_failed"
  | "task_review"
  | "needs_attention"
  | "system";

/** Desktop notification + sound preferences. */
export interface NotificationSettings {
  /** Master switch for every notification. */
  enabled: boolean;
  on_task_complete: boolean;
  on_task_failed: boolean;
  on_task_review: boolean;
  on_needs_attention: boolean;
  /** Play a sound alongside a notification. */
  sound_enabled: boolean;
  /** Playback volume, 0..1. */
  volume: number;
  /** A preset id or a `file:<absolute path>` reference; null is silent. */
  complete_sound: string | null;
  failed_sound: string | null;
  review_sound: string | null;
  attention_sound: string | null;
}

/** The payload of the backend `app://notify` event. */
export interface NotifyEvent {
  kind: NotifyKind;
  title: string;
  body: string;
}

/** The agent configuration fields, shared by project config and settings. */
export interface AgentConfig {
  provider?: Provider | null;
  model?: string | null;
  fallback_provider?: Provider | null;
  fallback_model?: string | null;
  review_provider?: Provider | null;
  review_model?: string | null;
  review_mode?: ReviewMode | null;
  /** Custom instructions for the auto reviewer; blank uses the built-in prompt. */
  review_prompt?: string | null;
  editor?: string | null;
}

export interface ResolvedConfig {
  provider: Provider;
  model?: string | null;
  fallback_provider?: Provider | null;
  fallback_model?: string | null;
  review_provider: Provider;
  review_model?: string | null;
  review_mode: ReviewMode;
  review_prompt?: string | null;
  editor?: string | null;
}

export interface Task {
  id: string;
  project_path: string;
  title: string;
  prompt: string;
  /** Absolute paths of files attached to the prompt. */
  attachments?: string[];
  isolation: Isolation;
  profile: PermissionProfile;
  last_permission?: string | null;
  /** The live opencode session while the task runs (for messaging the agent). */
  session_id?: string | null;
  base_ref?: string | null;
  branch?: string | null;
  worktree_path?: string | null;
  not_before?: number | null;
  depends_on: string[];
  status: TaskStatus;
  exit_code?: number | null;
  error?: string | null;
  created_at: number;
  started_at?: number | null;
  finished_at?: number | null;
  /** How many times the task has been dispatched; `> 1` marks a re-run. */
  attempts?: number;
  provider?: Provider | null;
  model?: string | null;
  fallback_provider?: Provider | null;
  fallback_model?: string | null;
  used_fallback?: boolean;
  review?: TaskReview | null;
  /** What this task does; defaults to `agent`. */
  kind?: TaskKind | null;
  git_op?: GitOp | null;
  /** Shell command, or the commit/PR text for a git task. */
  command?: string | null;
  merge?: MergeSpec | null;
  branch_mode?: BranchMode | null;
  new_branch?: string | null;
  /** A question/permission the agent is waiting on, if any. */
  ask?: TaskAsk | null;
  /** The agent's final markdown summary, rendered in the Result tab. */
  result?: string | null;
}

export interface Project {
  path: string;
  name: string;
  added_at: number;
  default_base_ref?: string | null;
  default_profile?: PermissionProfile | null;
  provider?: Provider | null;
  model?: string | null;
  fallback_provider?: Provider | null;
  fallback_model?: string | null;
  review_provider?: Provider | null;
  review_model?: string | null;
  review_mode?: ReviewMode | null;
  review_prompt?: string | null;
  editor?: string | null;
  env_vars: ProjectEnvVar[];
  skills: ProjectSkill[];
  system_prompt?: SystemPrompt | null;
  conflict_mode?: ConflictMode | null;
}

/** The project config payload sent to `update_project_config`. */
export interface ProjectConfigInput extends AgentConfig {
  env_vars?: EnvValue[];
  skills?: ProjectSkill[];
  system_prompt?: SystemPrompt | null;
  conflict_mode?: ConflictMode | null;
}

/** A soft-deleted task: the task record plus the pieces kept for restore. */
export interface DeletedTask {
  task: Task;
  deleted_at: number;
  /** Tail of the task's log when it was deleted. */
  summary?: string | null;
  /** Working-tree diff captured when it was deleted. */
  diff?: string | null;
}

export interface Snapshot {
  projects: Project[];
  tasks: Task[];
  concurrency: number;
  running: number;
  settings: Settings;
  /** Soft-deleted tasks across all projects; filter by `task.project_path`. */
  deleted_tasks: DeletedTask[];
}

export interface Settings {
  /** Cache retention in days; `0` keeps forever. */
  cache_retention_days: number;
  /** Fallback permission profile for projects without their own default. */
  default_profile?: PermissionProfile | null;
  provider?: Provider | null;
  model?: string | null;
  fallback_provider?: Provider | null;
  fallback_model?: string | null;
  review_provider?: Provider | null;
  review_model?: string | null;
  review_mode: ReviewMode;
  review_prompt?: string | null;
  editor?: string | null;
  command_templates: CommandTemplates;
  secret_store?: SecretStore | null;
  notifications: NotificationSettings;
  /** Collect local, content-free analytics. On by default. */
  analytics_enabled: boolean;
  /** How long analytics records are kept, in days. `0` keeps forever. */
  analytics_retention_days: number;
  /**
   * Store a redacted, length-bounded sample next to each error event. Off by
   * default: error summaries can contain sensitive fragments.
   */
  analytics_capture_error_examples: boolean;
}

export interface PromptEntry {
  id: string;
  project_path?: string | null;
  title: string;
  prompt: string;
  profile?: PermissionProfile | null;
  isolation?: Isolation | null;
  created_at: number;
  uses: number;
}

// ---- connected devices (mobile pairing) ----

/** The desktop's own public identity, for the Add-phone dialog. */
export interface DesktopIdentity {
  desktop_id: string;
  fingerprint: string;
}

/** A trusted phone as shown in Settings → Connected devices. */
export interface ConnectedDevice {
  device_id: string;
  name: string;
  platform: string;
  /** Granted scopes, e.g. `["sessions.read", "sessions.message"]`. */
  scopes: string[];
  paired_at: number;
  last_seen: number | null;
  online: boolean;
  fingerprint: string;
}

/** A freshly created pairing challenge: a QR payload plus a manual code. */
export interface PairingOffer {
  pairing_id: string;
  manual_code: string;
  expires_at: number;
  /** The JSON string to render as a QR code. */
  qr_string: string;
  desktop_id: string;
  desktop_fingerprint: string;
  /** Whether the Iroh listener advertised a dialable address. */
  reachable: boolean;
}

/** The live state of a pairing challenge, polled by the Add-phone dialog. */
export interface PairingStatus {
  state: string;
  /** Present only once the phone has begun pairing (both nonces are bound). */
  verification_code?: string;
  device_name?: string;
  device_platform?: string;
}

// ---- remote access (WebSocket listener + Tailscale Serve) ----

/** Persisted remote-access preferences. */
export interface RemoteAccessPreferences {
  enabled: boolean;
  tailscale_backup: boolean;
  https_port: number;
  local_port: number;
}

/** The installed Tailscale CLI's status, as the desktop sees it. */
export interface TailscaleStatus {
  installed: boolean;
  exe?: string;
  backend_state: string;
  authenticated: boolean;
  online: boolean;
  dns_name?: string;
  tailnet?: string;
  version?: string;
  message?: string;
}

/** The desktop's remote-access status for Settings. */
export interface RemoteStatus {
  enabled: boolean;
  /** `disabled` | `listening` | `active` | `degraded` | `error`. */
  state: string;
  listener_running: boolean;
  listener_port: number | null;
  connected_clients: number;
  tailscale_backup: boolean;
  tailscale: TailscaleStatus;
  serve_enabled: boolean;
  serve_owned: boolean;
  serve_url?: string;
  serve_port?: number;
  https_port: number;
  local_port: number;
  last_error?: string;
}

/** The embedded push gateway's status, shown in Settings → Devices. */
export interface PushGatewayStatus {
  /** Whether the user asked the gateway to run. */
  enabled: boolean;
  /** Whether it is listening right now. */
  running: boolean;
  /** The loopback `POST /v1/push` URL, when running. */
  url?: string;
  port: number;
  /** The configured Firebase service-account path (not a secret). */
  service_account?: string;
  /** Authorized paired devices that registered an FCM token. */
  registered_devices: number;
  last_error?: string;
}

export interface CacheStats {
  prompt_count: number;
  oldest_prompt?: number | null;
  log_count: number;
  log_bytes: number;
  retention_days: number;
}

export interface ChangedFile {
  path: string;
  status: string;
}

export interface GitStatus {
  is_repo: boolean;
  branch?: string | null;
  dirty: boolean;
  changed_files: ChangedFile[];
  ahead: number;
  behind: number;
}

export interface Worktree {
  path: string;
  branch?: string | null;
  head?: string | null;
  is_main: boolean;
}

/** A branch's landing state against the project's default branch. */
export interface BranchInfo {
  name: string;
  /** A remote-tracking branch with no local counterpart. */
  is_remote: boolean;
  is_default: boolean;
  is_current: boolean;
  /** The default branch already contains every commit here. */
  merged: boolean;
  ahead: number;
  behind: number;
  worktree?: string | null;
  has_remote: boolean;
  /** The branch's worktree (or the project folder) has uncommitted changes. */
  dirty: boolean;
}

/** One commit in a worktree's history, summarized for the Git view. */
export interface CommitSummary {
  id: string;
  short_id: string;
  subject: string;
  author: string;
  committed_at: number;
  files_changed: number;
  additions: number;
  deletions: number;
}

/** One file touched by a commit, with its change counts. */
export interface CommitFile {
  path: string;
  status: string;
  additions: number;
  deletions: number;
}

/** Landing detail for a worktree, computed on demand when expanded. */
export interface WorktreeDetail {
  path: string;
  branch?: string | null;
  base: string;
  head?: string | null;
  commit_count: number;
  files_changed: number;
  additions: number;
  deletions: number;
  last_commit_at?: number | null;
  last_commit_subject?: string | null;
  dirty: boolean;
  commits: CommitSummary[];
}

export interface FileDiff {
  path: string;
  status: string;
  diff: string;
}

export interface DiffResult {
  stat: string;
  files: FileDiff[];
}

export interface PlanTask {
  id?: string | null;
  title: string;
  prompt: string;
  isolation?: string | null;
  after: string[];
  delay_seconds?: number | null;
}

export interface PlanResult {
  summary: string;
  tasks: PlanTask[];
  raw: string;
}

export interface NewTask {
  local_id?: string | null;
  title: string;
  prompt: string;
  /** Absolute paths of files to attach to the prompt. */
  attachments?: string[];
  isolation?: Isolation | null;
  profile?: PermissionProfile | null;
  after?: string[];
  delay_seconds?: number | null;
  base_ref?: string | null;
  kind?: TaskKind | null;
  git_op?: GitOp | null;
  command?: string | null;
  merge?: MergeSpec | null;
  branch_mode?: BranchMode | null;
  new_branch?: string | null;
}

export interface TaskPatch {
  title?: string;
  prompt?: string;
  /** Absolute paths of files attached to the prompt; `[]` clears them. */
  attachments?: string[];
  isolation?: Isolation;
  profile?: PermissionProfile;
  base_ref?: string;
  delay_seconds?: number;
  depends_on?: string[];
  command?: string;
  branch_mode?: BranchMode;
  new_branch?: string;
  kind?: TaskKind;
}

/** A coarse kind for a task-log line, used to color and filter the log. */
export type LogKind =
  | "text"
  | "tool"
  | "note"
  | "error"
  | "warn"
  | "success"
  | "heading";

/** One classified line in a task log. */
export interface LogEntry {
  text: string;
  kind: LogKind;
}

export interface LogEvent {
  task_id: string;
  stream: string;
  line: string;
  /** Coarse kind from the backend (`text` | `tool` | `note`); may be absent. */
  kind?: string | null;
}

// ---- analytics ----

/**
 * Query filters for the analytics views. `null` bounds mean unbounded; `0` means
 * observed zero. Empty `projects`/`models` means "all".
 */
export interface AnalyticsFilters {
  from_ms: number | null;
  to_ms: number | null;
  projects: string[];
  models: string[];
}

export interface AnalyticsOverview {
  has_data: boolean;
  tasks_total: number;
  tasks_completed: number;
  tasks_failed: number;
  tasks_canceled: number;
  tasks_blocked: number;
  tasks_interrupted: number;
  success_rate: number | null;
  success_sample: number;
  avg_active_ms: number | null;
  median_active_ms: number | null;
  duration_sample: number;
  avg_wait_ms: number | null;
  total_tokens: number | null;
  input_tokens: number | null;
  output_tokens: number | null;
  reasoning_tokens: number | null;
  cache_read_tokens: number | null;
  cache_write_tokens: number | null;
  reported_cost_micros: number | null;
  inference_count: number;
  usage_coverage_runs: number;
  usage_runs_with_data: number;
  currency: string;
  cost_basis: string;
  /** Runs that recorded at least one categorized error. */
  runs_with_errors: number;
  /** Categorized error occurrences (not task failures). */
  error_events: number;
  /** Tasks that needed more than one dispatch. */
  retry_rate: number | null;
  /** Successful terminal tasks that needed no retry. */
  first_pass_success_rate: number | null;
  /** Successful agent tasks with no blocking human interaction. */
  autonomy_rate: number | null;
  autonomy_sample: number;
  generated_at_ms: number;
}

export interface AnalyticsTimeSeriesPoint {
  bucket_ms: number;
  tasks_completed: number;
  tasks_failed: number;
  tasks_terminal: number;
  total_tokens: number | null;
  reported_cost_micros: number | null;
  inference_count: number;
}

export interface AnalyticsProjectMetric {
  project_path: string;
  project_name: string;
  tasks_total: number;
  tasks_completed: number;
  tasks_failed: number;
  success_rate: number | null;
  success_sample: number;
  avg_active_ms: number | null;
  median_active_ms: number | null;
  total_tokens: number | null;
  reported_cost_micros: number | null;
  inference_count: number;
}

export interface AnalyticsModelMetric {
  provider_id: string;
  model_id: string;
  variant: string | null;
  inference_count: number;
  task_runs: number;
  input_tokens: number | null;
  output_tokens: number | null;
  reasoning_tokens: number | null;
  cache_read_tokens: number | null;
  cache_write_tokens: number | null;
  total_tokens: number | null;
  reported_cost_micros: number | null;
  /** Task runs whose usage was exclusively this model. */
  exclusive_task_runs: number;
  exclusive_success: number;
  exclusive_success_rate: number | null;
  /** Task runs that used this model and another (`mixed`). */
  mixed_task_runs: number;
}

export interface AnalyticsRunDetail {
  run_id: string;
  attempt: number;
  kind: string;
  provider: string | null;
  model: string | null;
  status: string;
  started_at_ms: number | null;
  ended_at_ms: number | null;
  active_ms: number | null;
  total_tokens: number | null;
  reported_cost_micros: number | null;
  inference_count: number;
}

export interface AnalyticsErrorDetail {
  error_id: string;
  category: string;
  fingerprint: string;
  code: string;
  stage: string;
  tool_name: string | null;
  occurred_at_ms: number;
  recovered: boolean | null;
}

export interface AnalyticsInteractionDetail {
  interaction_id: string;
  kind: string;
  category: string;
  asked_at_ms: number;
  resolved_at_ms: number | null;
  blocking: boolean;
  outcome: string | null;
}

export interface AnalyticsTaskDetail {
  task_id: string;
  title: string;
  project_path: string;
  status: string;
  kind: string;
  provider: string | null;
  model: string | null;
  created_at_ms: number;
  started_at_ms: number | null;
  ended_at_ms: number | null;
  attempts: number;
  review_status: string | null;
  error: string | null;
  runs: AnalyticsRunDetail[];
  errors: AnalyticsErrorDetail[];
  interactions: AnalyticsInteractionDetail[];
  total_tokens: number | null;
  reported_cost_micros: number | null;
}

export interface AnalyticsCollectionStatus {
  enabled: boolean;
  retention_days: number;
  run_count: number;
  usage_count: number;
  error_count: number;
  interaction_count: number;
  /** Usage records with both tokens and cost (`complete`) vs not. */
  usage_complete: number;
  usage_partial: number;
  /** Whether redacted error samples are being stored (off by default). */
  capture_error_examples: boolean;
  oldest_at_ms: number | null;
  newest_at_ms: number | null;
}

// ---- analytics: reliability (Phase 2) ----

export interface ReliabilitySummary {
  has_data: boolean;
  tasks_failed: number;
  failed_sample: number;
  task_failure_rate: number | null;
  runs_with_errors: number;
  observed_runs: number;
  runs_with_errors_rate: number | null;
  error_events: number;
  recoverable_errors: number;
  recovered_errors: number;
  recovery_rate: number | null;
  retry_tasks: number;
  attempt_tasks: number;
  retry_rate: number | null;
  first_pass_success: number;
  first_pass_sample: number;
  first_pass_success_rate: number | null;
}

export interface ErrorCategoryMetric {
  category: string;
  occurrences: number;
  tasks: number;
  recovered: number;
  recoverable: number;
  recovery_rate: number | null;
}

export interface ErrorGroup {
  fingerprint: string;
  category: string;
  code: string;
  stage: string;
  tool_name: string | null;
  occurrences: number;
  tasks: number;
  recovered: number;
  recoverable: number;
  unresolved: number;
  first_seen_ms: number;
  last_seen_ms: number;
  sample: string | null;
  task_ids: string[];
}

export interface StageToolMetric {
  name: string;
  occurrences: number;
  tasks: number;
  recovered: number;
}

export interface ErrorTimeSeriesPoint {
  bucket_ms: number;
  errors: number;
  tasks_failed: number;
}

export interface ReliabilityResult {
  summary: ReliabilitySummary;
  categories: ErrorCategoryMetric[];
  groups: ErrorGroup[];
  stages: StageToolMetric[];
  tools: StageToolMetric[];
  series: ErrorTimeSeriesPoint[];
}

// ---- analytics: interactions (Phase 3) ----

export interface InteractionSummary {
  has_data: boolean;
  questions: number;
  permissions: number;
  manual_confirmations: number;
  blocking_total: number;
  resolved: number;
  unresolved: number;
  total_wait_ms: number;
  avg_wait_ms: number | null;
  median_wait_ms: number | null;
  p95_wait_ms: number | null;
  autonomy_rate: number | null;
  autonomy_autonomous: number;
  autonomy_sample: number;
  first_pass_success_rate: number | null;
}

export interface InteractionGroup {
  fingerprint: string;
  kind: string;
  category: string;
  occurrences: number;
  tasks: number;
  blocking: number;
  unresolved: number;
  avg_wait_ms: number | null;
  last_seen_ms: number;
  task_ids: string[];
}

export interface InteractionCategoryMetric {
  category: string;
  kind: string;
  occurrences: number;
  tasks: number;
  unresolved: number;
  avg_wait_ms: number | null;
}

export interface Recommendation {
  title: string;
  detail: string;
  count: number;
}

export interface InteractionResult {
  summary: InteractionSummary;
  groups: InteractionGroup[];
  categories: InteractionCategoryMetric[];
  recommendations: Recommendation[];
}
