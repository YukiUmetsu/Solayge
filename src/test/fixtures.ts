import type {
  EnvironmentStatus,
  Project,
  Settings,
  Snapshot,
  Task,
  TaskStatus,
} from "../types";

export function makeSettings(overrides: Partial<Settings> = {}): Settings {
  return {
    cache_retention_days: 30,
    review_mode: "off",
    command_templates: {
      opencode: "opencode run --standalone {auto} [--model {model}] {prompt}",
      codex: "codex exec [--model {model}] {prompt}",
      claude: "claude -p {prompt} [--model {model}]",
      cursor: "cursor-agent -p {prompt} [--model {model}]",
    },
    notifications: {
      enabled: false,
      on_task_complete: true,
      on_task_failed: true,
      on_task_review: true,
      on_needs_attention: true,
      sound_enabled: false,
      volume: 0.5,
      complete_sound: null,
      failed_sound: null,
      review_sound: null,
      attention_sound: null,
    },
    analytics_enabled: true,
    analytics_retention_days: 90,
    analytics_capture_error_examples: false,
    ...overrides,
  };
}

export function makeProject(overrides: Partial<Project> = {}): Project {
  return {
    path: "/tmp/solayge-project",
    name: "solayge-project",
    added_at: 1_700_000_000,
    env_vars: [],
    skills: [],
    ...overrides,
  };
}

export function makeTask(overrides: Partial<Task> = {}): Task {
  const status: TaskStatus = overrides.status ?? "draft";
  return {
    id: overrides.id ?? "task-1",
    project_path: overrides.project_path ?? "/tmp/solayge-project",
    title: overrides.title ?? "A task",
    prompt: overrides.prompt ?? "Do the thing",
    attachments: overrides.attachments ?? [],
    isolation: overrides.isolation ?? "worktree",
    profile: overrides.profile ?? "autonomous",
    depends_on: overrides.depends_on ?? [],
    status,
    created_at: overrides.created_at ?? 1_700_000_000,
    ...overrides,
  } as Task;
}

export function makeSnapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    projects: overrides.projects ?? [makeProject()],
    tasks: overrides.tasks ?? [],
    concurrency: overrides.concurrency ?? 2,
    running: overrides.running ?? 0,
    settings: overrides.settings ?? makeSettings(),
    deleted_tasks: overrides.deleted_tasks ?? [],
  };
}

export function makeTools(overrides: Partial<EnvironmentStatus> = {}): EnvironmentStatus {
  return {
    git: { name: "git", found: true, path: "/usr/bin/git" },
    gh: { name: "gh", found: true, path: "/usr/bin/gh" },
    providers: [
      { provider: "opencode", command: "opencode", found: true, path: "/usr/bin/opencode" },
      { provider: "codex", command: "codex", found: true },
      { provider: "claude", command: "claude", found: true },
      { provider: "cursor", command: "cursor-agent", found: true },
    ],
    ...overrides,
  };
}
