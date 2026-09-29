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
}

export interface CommandTemplates {
  opencode: string;
  codex: string;
  claude: string;
  cursor: string;
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

export interface Snapshot {
  projects: Project[];
  tasks: Task[];
  concurrency: number;
  running: number;
  settings: Settings;
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
  editor?: string | null;
  command_templates: CommandTemplates;
  secret_store?: SecretStore | null;
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
}

export interface LogEvent {
  task_id: string;
  stream: string;
  line: string;
}
