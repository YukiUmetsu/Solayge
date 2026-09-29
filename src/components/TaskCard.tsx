import type { Task } from "../types";
import {
  STATUS_META,
  PROFILE_META,
  isLive,
  runDuration,
  shortId,
} from "../lib/format";
import { providerLabel, reviewStatusMeta, taskKindLabel, taskSeparationDisplay } from "../lib/providers";
import { Icon } from "./Icons";

export function TaskCard({
  task,
  now,
  selected,
  onSelect,
  onStartNow,
  onCancel,
  onRetry,
  onDelete,
  onShowDiff,
}: {
  task: Task;
  now: number;
  selected: boolean;
  onSelect: () => void;
  onStartNow: () => void;
  onCancel: () => void;
  onRetry: () => void;
  onDelete: () => void;
  onShowDiff: () => void;
}) {
  const meta = STATUS_META[task.status];
  const pm = PROFILE_META[task.profile] ?? PROFILE_META.autonomous;
  const isTerminal = ["succeeded", "failed", "canceled", "blocked", "interrupted"].includes(
    task.status,
  );
  const canRun = ["draft", "waiting", "ready", "blocked"].includes(task.status);
  const canCancel = ["waiting", "ready", "running"].includes(task.status);
  const live = isLive(task);
  const review =
    task.review && task.review.mode !== "off" ? task.review : null;
  const reviewMeta = review ? reviewStatusMeta(review.status) : null;
  const kind = task.kind ?? "agent";
  const kindLabel = taskKindLabel(task);
  const kindIcon = kind === "merge" ? "diff" : kind === "git" ? "branch" : "terminal";
  const separation = taskSeparationDisplay(task);

  return (
    <div
      onClick={onSelect}
      className={`card group cursor-pointer rounded-xl px-3.5 py-3 transition ${
        selected ? "ring-1 ring-accent-line" : ""
      }`}
    >
      <div className="flex items-start gap-2.5">
        <span
          className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${meta.dot}`}
          title={meta.label}
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <div className="truncate text-[13.5px] font-medium text-ink">
              {task.title || "Untitled task"}
            </div>
            <span
              className={`shrink-0 rounded-md border px-1.5 py-0.5 text-[10.5px] font-medium ${meta.chip} ${meta.text}`}
            >
              {meta.label}
              {task.status === "succeeded" && task.exit_code === 0 ? "" : ""}
            </span>
          </div>

          <p className="mt-1 line-clamp-2 text-[11.5px] leading-relaxed text-ink-muted">
            {task.prompt}
          </p>

          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-ink-subtle">
            {kindLabel && (
              <span className="flex items-center gap-1 rounded border border-accent-line bg-accent-soft px-1.5 py-0.5 text-[10px] text-accent-text">
                <Icon name={kindIcon} className="h-3 w-3" />
                {kindLabel}
              </span>
            )}
            {kind === "agent" && (
              <span
                className={`flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] ${pm.chip} ${pm.text}`}
                title={pm.desc}
              >
                <span className={`h-1.5 w-1.5 rounded-full ${pm.dot}`} />
                {pm.label}
              </span>
            )}
            <span className="flex items-center gap-1">
              <Icon name={separation.icon} className="h-3 w-3" />
              {separation.label}
            </span>
            {task.started_at && (
              <span
                className={`flex items-center gap-1 ${live ? "text-warning" : ""}`}
                title={live ? "Running for" : "Ran for"}
              >
                <Icon name="clock" className="h-3 w-3" />
                {runDuration(task, now)}
                {live && (
                  <span className="h-1.5 w-1.5 rounded-full bg-warning running-dot" />
                )}
              </span>
            )}
            {task.provider && kind === "agent" && (
              <span
                className="flex items-center gap-1"
                title={
                  task.fallback_provider
                    ? `Backup: ${providerLabel(task.fallback_provider)}${
                        task.used_fallback ? " (already used)" : ""
                      }`
                    : "No backup configured"
                }
              >
                <Icon name="layers" className="h-3 w-3" />
                {providerLabel(task.provider)}
                {task.model ? ` · ${task.model}` : ""}
                {task.used_fallback && (
                  <span className="rounded bg-warning-soft px-1 text-[9.5px] text-warning">
                    backup
                  </span>
                )}
              </span>
            )}
            {task.depends_on.length > 0 && (
              <span className="flex items-center gap-1">
                <Icon name="layers" className="h-3 w-3" />
                after {task.depends_on.length}
              </span>
            )}
            {reviewMeta && (
              <span
                className={`flex items-center gap-1 ${reviewMeta.text}`}
                title={review?.summary ?? reviewMeta.label}
              >
                <span className={`h-1.5 w-1.5 rounded-full ${reviewMeta.dot}`} />
                {reviewMeta.label}
              </span>
            )}
            {task.exit_code !== null && task.exit_code !== undefined && (
              <span className="mono">exit {task.exit_code}</span>
            )}
            <span className="mono text-ink-faint">{shortId(task.id)}</span>
          </div>

          {task.error && (
            <div className="mt-2 flex items-start gap-1.5 rounded-md border border-danger-line bg-danger-soft px-2 py-1 text-[11px] text-danger">
              <Icon name="alert" className="mt-0.5 h-3 w-3 shrink-0" />
              <span className="line-clamp-2">{task.error}</span>
            </div>
          )}
        </div>

        <div className="flex shrink-0 flex-col gap-1 opacity-0 transition group-hover:opacity-100">
          {canRun && (
            <IconBtn name="play" title="Start now" onClick={onStartNow} />
          )}
          {canCancel && (
            <IconBtn name="stop" title="Cancel" onClick={onCancel} danger />
          )}
          {isTerminal && <IconBtn name="retry" title="Retry" onClick={onRetry} />}
          {(task.worktree_path || task.branch) && (
            <IconBtn name="diff" title="View diff" onClick={onShowDiff} />
          )}
          <IconBtn name="trash" title="Delete" onClick={onDelete} danger />
        </div>
      </div>
    </div>
  );
}

function IconBtn({
  name,
  title,
  onClick,
  danger,
}: {
  name: Parameters<typeof Icon>[0]["name"];
  title: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      title={title}
      className={`rounded-md border border-line bg-well p-1.5 transition ${
        danger
          ? "text-ink-muted hover:border-danger-line hover:text-danger"
          : "text-ink-muted hover:border-accent-line hover:text-accent-text"
      }`}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
    >
      <Icon name={name} className="h-3.5 w-3.5" />
    </button>
  );
}

export function buildTree(tasks: Task[]): { task: Task; depth: number }[] {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const children = new Map<string, Task[]>();
  const roots: Task[] = [];
  for (const t of tasks) {
    const deps = t.depends_on.filter((d) => byId.has(d));
    if (deps.length === 0) {
      roots.push(t);
    } else {
      const parent = deps[0];
      const arr = children.get(parent) ?? [];
      arr.push(t);
      children.set(parent, arr);
    }
  }
  const out: { task: Task; depth: number }[] = [];
  const seen = new Set<string>();
  const walk = (t: Task, depth: number) => {
    if (seen.has(t.id)) return;
    seen.add(t.id);
    out.push({ task: t, depth });
    const kids = (children.get(t.id) ?? []).sort(
      (a, b) => a.created_at - b.created_at,
    );
    for (const c of kids) walk(c, depth + 1);
  };
  for (const r of roots.sort((a, b) => a.created_at - b.created_at))
    walk(r, 0);
  for (const t of tasks) if (!seen.has(t.id)) walk(t, 0);
  return out;
}
