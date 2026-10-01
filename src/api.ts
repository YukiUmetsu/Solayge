import { invoke } from "@tauri-apps/api/core";
import type {
  BranchInfo,
  CacheStats,
  DiffResult,
  EnvValue,
  EnvironmentStatus,
  GitStatus,
  Isolation,
  MergeSourceStatus,
  NewTask,
  PermissionProfile,
  PlanResult,
  ProjectConfigInput,
  PromptEntry,
  Provider,
  ResolvedConfig,
  Settings,
  Snapshot,
  TaskPatch,
  Worktree,
} from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  addProject: (path: string) => invoke<Snapshot>("add_project", { path }),
  removeProject: (path: string) => invoke<Snapshot>("remove_project", { path }),
  projectStatus: (path: string) => invoke<GitStatus>("project_status", { path }),
  projectWorktrees: (path: string) =>
    invoke<Worktree[]>("project_worktrees", { path }),
  projectBranches: (path: string) =>
    invoke<BranchInfo[]>("project_branches", { path }),
  gitMergeBranch: (path: string, branch: string, target?: string | null) =>
    invoke<string>("git_merge_branch", { path, branch, target: target ?? null }),
  gitDeleteBranch: (path: string, branch: string) =>
    invoke<string>("git_delete_branch", { path, branch }),
  gitDeleteRemoteBranch: (path: string, branch: string) =>
    invoke<string>("git_delete_remote_branch", { path, branch }),
  gitRemoveWorktree: (path: string, worktree: string) =>
    invoke<string>("git_remove_worktree", { path, worktree }),
  gitDiff: (path: string, target?: string | null) =>
    invoke<DiffResult>("git_diff", { path, target: target ?? null }),
  projectDefaultBranch: (path: string) =>
    invoke<string>("project_default_branch", { path }),
  branchDiff: (path: string, includeLocal = false) =>
    invoke<DiffResult>("project_branch_diff", { path, includeLocal }),
  gitStage: (path: string, files?: string[] | null) =>
    invoke<string>("git_stage", { path, files: files ?? null }),
  gitCommit: (path: string, message: string) =>
    invoke<string>("git_commit", { path, message }),
  gitPush: (path: string) => invoke<string>("git_push", { path }),
  gitCreatePr: (path: string, title?: string | null) =>
    invoke<string>("git_create_pr", { path, title: title ?? null }),
  gitMergePr: (path: string, method?: string | null) =>
    invoke<string>("git_merge_pr", { path, method: method ?? null }),
  gitCheckoutPull: (path: string) =>
    invoke<string>("git_checkout_pull", { path }),
  mergePreflight: (path: string, sources: string[]) =>
    invoke<MergeSourceStatus[]>("merge_preflight", { path, sources }),
  commitWorktrees: (worktrees: string[], message?: string | null) =>
    invoke<string>("commit_worktrees", {
      worktrees,
      message: message ?? null,
    }),
  executeProject: (projectPath: string) =>
    invoke<Snapshot>("execute_project", { projectPath }),
  createTasks: (
    projectPath: string,
    tasks: NewTask[],
    defaultIsolation?: Isolation | null,
  ) =>
    invoke<Snapshot>("create_tasks", {
      projectPath,
      tasks,
      defaultIsolation: defaultIsolation ?? null,
    }),
  updateTask: (taskId: string, patch: TaskPatch) =>
    invoke<Snapshot>("update_task", { taskId, patch }),
  deleteTask: (taskId: string) =>
    invoke<Snapshot>("delete_task", { taskId }),
  restoreTask: (taskId: string) =>
    invoke<Snapshot>("restore_task", { taskId }),
  startNow: (taskId: string) =>
    invoke<Snapshot>("start_task_now", { taskId }),
  cancel: (taskId: string) => invoke<Snapshot>("cancel_task", { taskId }),
  retry: (taskId: string) => invoke<Snapshot>("retry_task", { taskId }),
  retryReview: (taskId: string) =>
    invoke<Snapshot>("retry_review", { taskId }),
  answerTask: (taskId: string, answer: Record<string, unknown>) =>
    invoke<Snapshot>("answer_task", { taskId, answer }),
  removeWorktree: (taskId: string) =>
    invoke<Snapshot>("remove_task_worktree", { taskId }),
  clearFinished: (projectPath: string) =>
    invoke<Snapshot>("clear_finished", { projectPath }),
  taskLog: (taskId: string) =>
    invoke<string>("get_task_log", { taskId }),
  setConcurrency: (value: number) =>
    invoke<Snapshot>("set_concurrency", { value }),
  setProjectDefaultProfile: (path: string, profile: PermissionProfile | null) =>
    invoke<Snapshot>("set_project_default_profile", { path, profile }),
  plan: (
    projectPath: string,
    goal: string,
    maxTasks?: number,
    timeoutSecs?: number,
  ) =>
    invoke<PlanResult>("plan_with_opencode", {
      projectPath,
      goal,
      maxTasks: maxTasks ?? null,
      timeoutSecs: timeoutSecs ?? null,
    }),

  // ---- settings & cache ----
  updateSettings: (settings: Settings) =>
    invoke<Snapshot>("update_settings", { settings }),
  promptHistory: (projectPath?: string | null, limit?: number) =>
    invoke<PromptEntry[]>("get_prompt_history", {
      projectPath: projectPath ?? null,
      limit: limit ?? null,
    }),
  cacheStats: () => invoke<CacheStats>("get_cache_stats"),
  clearCache: (prompts: boolean, logs: boolean) =>
    invoke<CacheStats>("clear_cache", { prompts, logs }),
  errorLog: () => invoke<string>("get_error_log"),
  clearErrorLog: () => invoke<void>("clear_error_log"),

  // ---- projects ----
  reorderProjects: (paths: string[]) =>
    invoke<Snapshot>("reorder_projects", { paths }),
  projectRemote: (path: string) =>
    invoke<string | null>("project_remote", { path }),
  openExternal: (target: string) =>
    invoke<void>("open_external", { target }),
  openInEditor: (path: string, editor?: string | null) =>
    invoke<string>("open_in_editor", { path, editor: editor ?? null }),
  updateProjectConfig: (path: string, config: ProjectConfigInput) =>
    invoke<Snapshot>("update_project_config", { path, config }),
  projectSecrets: (path: string) =>
    invoke<EnvValue[]>("get_project_secrets", { path }),
  resolvedConfig: (path: string) =>
    invoke<ResolvedConfig>("get_resolved_config", { path }),

  // ---- auto code review ----
  reviewLog: (taskId: string) =>
    invoke<string>("get_review_log", { taskId }),

  // ---- provider models ----
  listModels: (provider: Provider, force = false) =>
    invoke<string[]>("list_models", { provider, force }),

  // ---- preflight ----
  environmentCheck: () => invoke<EnvironmentStatus>("environment_check"),
};
