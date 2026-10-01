import type {
  AgentConfig,
  BranchMode,
  ConflictMode,
  GitOp,
  Isolation,
  Provider,
  ResolvedConfig,
  ReviewMode,
  ReviewStatus,
  SecretStore,
  Separation,
  TaskKind,
} from "../types";
import type { IconName } from "../components/Icons";

export const PROVIDERS: { id: Provider; label: string }[] = [
  { id: "opencode", label: "opencode" },
  { id: "codex", label: "Codex" },
  { id: "claude", label: "Claude Code" },
  { id: "cursor", label: "Cursor Agent" },
];

export function providerLabel(id?: Provider | null): string {
  return PROVIDERS.find((p) => p.id === id)?.label ?? "—";
}

/**
 * Model suggestions per provider. The field is free text, so anything the
 * provider accepts can be typed; these are just convenient defaults.
 */
export const MODEL_SUGGESTIONS: Record<Provider, string[]> = {
  opencode: [
    "anthropic/claude-sonnet-4-5",
    "anthropic/claude-opus-4-1",
    "openai/gpt-5",
    "google/gemini-2.5-pro",
  ],
  codex: ["gpt-5-codex", "gpt-5", "o3"],
  claude: ["claude-sonnet-4-5", "claude-opus-4-1", "claude-haiku-4-5"],
  cursor: ["auto", "gpt-5", "claude-sonnet-4-5"],
};

export const EDITORS: { id: string; label: string }[] = [
  { id: "system", label: "System default" },
  { id: "vscode", label: "VS Code" },
  { id: "cursor", label: "Cursor" },
  { id: "zed", label: "Zed" },
  { id: "windsurf", label: "Windsurf" },
  { id: "sublime", label: "Sublime Text" },
];

export function editorLabel(id?: string | null): string {
  return EDITORS.find((e) => e.id === id)?.label ?? "System default";
}

export const REVIEW_MODES: {
  id: ReviewMode;
  label: string;
  desc: string;
}[] = [
  { id: "off", label: "Off", desc: "No automatic review." },
  {
    id: "report",
    label: "Review only",
    desc: "Review the changes and record a verdict. Never blocks the run.",
  },
  {
    id: "autofix",
    label: "Review + auto-fix",
    desc: "Review, fix problems it finds, then continue.",
  },
  {
    id: "pause",
    label: "Review, then stop on issues",
    desc: "Review; if problems are found, stop the task for your attention.",
  },
];

export function reviewModeLabel(id?: ReviewMode | null): string {
  return REVIEW_MODES.find((m) => m.id === id)?.label ?? "Off";
}

/** Where encrypted project secrets are stored. `null` = automatic. */
export const SECRET_STORES: {
  id: SecretStore | "auto";
  label: string;
  desc: string;
}[] = [
  {
    id: "auto",
    label: "Automatic",
    desc: "Use the OS keychain when this platform has one, otherwise an encrypted local file.",
  },
  {
    id: "keychain",
    label: "OS keychain",
    desc: "macOS Keychain or Windows DPAPI. Not available on Linux — use the encrypted file there.",
  },
  {
    id: "file",
    label: "Encrypted local file",
    desc: "AES-256-GCM with a key kept in the app data directory (0600).",
  },
];

export function secretStoreLabel(store?: SecretStore | null): string {
  if (store === "file") return "an encrypted local file";
  if (store === "keychain") return "the OS keychain";
  return "encrypted storage";
}

/** How a project handles merge conflicts. */
export const CONFLICT_MODES: {
  id: ConflictMode;
  label: string;
  desc: string;
}[] = [
  {
    id: "user",
    label: "Stop and wait for me",
    desc: "The integration stops at the conflict and blocks the task. Resolve it yourself, then retry.",
  },
  {
    id: "agent_review",
    label: "Agent resolves, then wait for me",
    desc: "An agent resolves the conflict, then the task pauses so you can review before continuing.",
  },
  {
    id: "agent_auto",
    label: "Agent resolves and continues",
    desc: "An agent resolves the conflict and the workflow carries on automatically.",
  },
];

export function conflictModeLabel(id?: ConflictMode | null): string {
  return CONFLICT_MODES.find((m) => m.id === id)?.label ?? CONFLICT_MODES[0].label;
}

export const GIT_OP_LABELS: Record<GitOp, string> = {
  add_commit: "commit",
  push: "push",
  pr_create: "create PR",
  pr_merge: "merge PR",
  checkout: "checkout",
  pull: "pull",
};

/** A short label for a non-agent task, or `null` for ordinary agent tasks. */
export function taskKindLabel(task: {
  kind?: TaskKind | null;
  git_op?: GitOp | null;
}): string | null {
  switch (task.kind ?? "agent") {
    case "agent":
      return null;
    case "shell":
      return "shell";
    case "git":
      return GIT_OP_LABELS[task.git_op ?? "add_commit"];
    case "merge":
      return "combine";
  }
}

/**
 * The task separation choices, shared by New Task, Plan with AI, task cards,
 * and the detail panel. Add or rename options here only.
 */
export const SEPARATIONS: {
  id: Separation;
  /** Full label for select menus. */
  label: string;
  /** Short label for chips and detail rows. */
  short: string;
  /** One-line description shown under the control. */
  desc: string;
}[] = [
  {
    id: "worktree",
    label: "Worktree (isolated copy)",
    short: "worktree",
    desc: "Each task runs in its own git worktree, so parallel tasks can't clash.",
  },
  {
    id: "branch",
    label: "New branch",
    short: "new branch",
    desc: "The task creates a branch in the project folder first (agent-named when blank); these run one at a time.",
  },
  {
    id: "current",
    label: "Current branch",
    short: "current branch",
    desc: "Tasks run in the project folder on its current branch, one at a time.",
  },
];

export const DEFAULT_SEPARATION: Separation = "worktree";

export function separationMeta(id: Separation) {
  return SEPARATIONS.find((s) => s.id === id) ?? SEPARATIONS[0];
}

/** The `isolation` + `branch_mode` a separation maps to (for the backend). */
export function separationConfig(sep: Separation): {
  isolation: Isolation;
  branch_mode: BranchMode;
} {
  return {
    isolation: sep === "worktree" ? "worktree" : "shared",
    branch_mode: sep === "branch" ? "new" : "current",
  };
}

/** Map a plan task's own isolation suggestion onto a separation choice. */
export function separationFromIsolation(
  planIsolation: string | null | undefined,
  fallback: Separation,
): Separation {
  if (planIsolation === "worktree") return "worktree";
  if (planIsolation === "shared") return "current";
  return fallback;
}

/** The separation an existing task was created with. */
export function taskSeparation(task: {
  isolation?: Isolation | null;
  branch_mode?: BranchMode | null;
}): Separation {
  if (task.isolation === "worktree") return "worktree";
  return task.branch_mode === "new" ? "branch" : "current";
}

/** A task's separation as displayed (label + icon), including its branch. */
export function taskSeparationDisplay(task: {
  isolation?: Isolation | null;
  branch_mode?: BranchMode | null;
  branch?: string | null;
}): { label: string; icon: IconName } {
  const sep = taskSeparation(task);
  if (sep === "worktree") {
    return { label: task.branch ?? "worktree", icon: "branch" };
  }
  if (sep === "branch") {
    return {
      label: task.branch ? `new branch · ${task.branch}` : "new branch",
      icon: "branch",
    };
  }
  return { label: "current branch", icon: "folder" };
}

export const REVIEW_STATUS_META: Record<
  ReviewStatus,
  { label: string; text: string; dot: string }
> = {
  none: { label: "Not reviewed", text: "text-ink-subtle", dot: "bg-ink-faint" },
  pending: { label: "Review queued", text: "text-ink-muted", dot: "bg-neutral" },
  running: {
    label: "Reviewing",
    text: "text-warning",
    dot: "bg-warning running-dot",
  },
  passed: { label: "Review passed", text: "text-success", dot: "bg-success" },
  issues: { label: "Review: issues", text: "text-danger", dot: "bg-danger" },
  failed: { label: "Review failed", text: "text-danger", dot: "bg-danger" },
};

export function reviewStatusMeta(status?: ReviewStatus | null) {
  return REVIEW_STATUS_META[status ?? "none"];
}

/** Effective config for display: project value, else account default. */
export function effectiveConfig(
  project: AgentConfig,
  settings: AgentConfig,
): ResolvedConfig {
  const provider = project.provider ?? settings.provider ?? "opencode";
  const reviewProvider =
    project.review_provider ?? settings.review_provider ?? provider;
  return {
    provider,
    model: project.model ?? settings.model ?? null,
    fallback_provider:
      project.fallback_provider ?? settings.fallback_provider ?? null,
    fallback_model: project.fallback_model ?? settings.fallback_model ?? null,
    review_provider: reviewProvider,
    review_model:
      project.review_model ??
      settings.review_model ??
      project.model ??
      settings.model ??
      null,
    review_mode: project.review_mode ?? settings.review_mode ?? "off",
    review_prompt:
      project.review_prompt ?? settings.review_prompt ?? null,
    editor: project.editor ?? settings.editor ?? null,
  };
}
