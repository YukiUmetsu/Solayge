import { invoke } from "@tauri-apps/api/core";
import type {
  CacheStats,
  DiffResult,
  EnvValue,
  EnvironmentStatus,
  GitStatus,
  Isolation,
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
  gitDiff: (path: string, target?: string | null) =>
    invoke<DiffResult>("git_diff", { path, target: target ?? null }),
  projectDefaultBranch: (path: string) =>
    invoke<string>("project_default_branch", { path }),
  branchDiff: (path: string) =>
    invoke<DiffResult>("project_branch_diff", { path }),
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
  startNow: (taskId: string) =>
    invoke<Snapshot>("start_task_now", { taskId }),
  cancel: (taskId: string) => invoke<Snapshot>("cancel_task", { taskId }),
  retry: (taskId: string) => invoke<Snapshot>("retry_task", { taskId }),
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
