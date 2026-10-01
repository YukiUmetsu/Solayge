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
}

export interface TaskReview {
  mode: ReviewMode;
  status: ReviewStatus;
  provider?: Provider | null;
  model?: string | null;
  summary?: string | null;
  started_at?: number | null;
  finished_at?: number | null;
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
