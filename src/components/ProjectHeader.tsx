import type {
  EnvironmentStatus,
  GitStatus,
  PermissionProfile,
  Project,
  ResolvedConfig,
  Task,
} from "../types";
import { editorLabel, providerLabel, reviewModeLabel } from "../lib/providers";
import { Icon } from "./Icons";

export function ProjectHeader({
  project,
  status,
  tasks,
  running,
  resolved,
  remote,
  tools,
  onNewTask,
  onPlan,
  onExecute,
  onRefresh,
  onReveal,
  onSetProfile,
  onOpenRemote,
  onOpenEditor,
  onBranchDiff,
  onShip,
  onProjectSettings,
  onCheckTools,
}: {
  project: Project;
  status: GitStatus | null;
  tasks: Task[];
  running: number;
  resolved: ResolvedConfig;
  remote: string | null;
  tools: EnvironmentStatus | null;
  onNewTask: () => void;
  onPlan: () => void;
  onExecute: () => void;
  onRefresh: () => void;
  onReveal: () => void;
  onSetProfile: (p: PermissionProfile) => void;
  onOpenRemote: () => void;
  onOpenEditor: () => void;
  onBranchDiff: () => void;
  onShip: () => void;
  onProjectSettings: () => void;
  onCheckTools: () => void;
}) {
  const total = tasks.length;
  const succeeded = tasks.filter((t) => t.status === "succeeded").length;
  const failed = tasks.filter((t) => t.status === "failed").length;
  const interrupted = tasks.filter((t) => t.status === "interrupted").length;
  const drafts = tasks.filter((t) => t.status === "draft").length;
  const retryable = tasks.filter((t) =>
    ["failed", "canceled", "blocked", "interrupted"].includes(t.status),
  ).length;
  const runnable = drafts + retryable;
  const finished = tasks.filter((t) =>
    ["succeeded", "failed", "canceled", "blocked", "interrupted"].includes(
      t.status,
    ),
  ).length;
  const pct = total === 0 ? 0 : Math.round((finished / total) * 100);

  const providerTool = tools?.providers.find(
    (p) => p.provider === resolved.provider,
  );
  let warning: string | null = null;
  if (tools && !tools.git.found) {
    warning =
      "Git was not found on PATH — tasks can't run. Install git and restart the app.";
  } else if (providerTool && !providerTool.found) {
    warning = `The "${providerTool.command}" CLI for ${providerLabel(
      resolved.provider,
    )} was not found on PATH. Tasks will fail to launch with a "No such file or directory" error. Install it, or switch this project's provider (Project settings → Agent).`;
  } else if (providerTool?.note) {
    warning = providerTool.note;
  }

  return (
    <div
      className="panel titlebar-pad rounded-none border-x-0 border-t-0 px-6 pb-4"
      data-tauri-drag-region="deep"
    >
      <div className="drag-region flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="truncate text-lg font-semibold text-ink">
              {project.name}
            </h1>
            {status?.branch && (
              <span className="mono flex items-center gap-1 rounded-md border border-line bg-well px-2 py-0.5 text-[11px] text-ink-muted">
                <Icon name="branch" className="h-3 w-3" />
                {status.branch}
              </span>
            )}
            {status?.dirty && (
              <span className="rounded-md border border-warning-line bg-warning-soft px-2 py-0.5 text-[11px] text-warning">
                {status.changed_files.length} changed
              </span>
            )}
            <span
              className="flex items-center gap-1 rounded-md border border-line bg-well px-2 py-0.5 text-[11px] text-ink-muted"
              title="Agent used for new tasks in this project"
            >
              <Icon name="layers" className="h-3 w-3" />
              {providerLabel(resolved.provider)}
              {resolved.model ? ` · ${resolved.model}` : ""}
            </span>
            {resolved.review_mode !== "off" && (
              <span
                className="flex items-center gap-1 rounded-md border border-accent-line bg-accent-soft px-2 py-0.5 text-[11px] text-accent-text"
                title="Auto code review"
              >
                <Icon name="check" className="h-3 w-3" />
                {reviewModeLabel(resolved.review_mode)}
              </span>
            )}
            <select
              className="no-drag rounded-md border border-line bg-well px-2 py-1 text-[11px] text-ink-muted outline-none"
              value={project.default_profile ?? "autonomous"}
              onChange={(e) => onSetProfile(e.target.value as PermissionProfile)}
              title="Default permission profile for new tasks"
            >
              <option value="autonomous">Default: Autonomous</option>
              <option value="supervised">Default: Supervised</option>
              <option value="readonly">Default: Read-only</option>
            </select>
          </div>
          <button
            onClick={onReveal}
            className="no-drag mt-1 flex items-center gap-1 text-[11px] text-ink-subtle hover:text-ink-muted"
            title="Reveal in the file manager"
          >
            <span className="mono truncate">{project.path}</span>
            <Icon name="external" className="h-3 w-3 shrink-0" />
          </button>
        </div>

        <div className="no-drag flex shrink-0 flex-col items-end gap-2">
          <div className="flex items-center gap-1.5">
            <button className="btn btn-ghost" onClick={onNewTask} title="New task">
              <Icon name="plus" className="h-3.5 w-3.5" />
              Task
            </button>
            <button
              className="btn btn-ghost"
              onClick={onShip}
              title="Commit, push, open a PR, merge it, and sync"
            >
              <Icon name="zap" className="h-3.5 w-3.5" />
              Ship
            </button>
            <button
              className={runnable > 0 ? "btn btn-primary" : "btn btn-ghost"}
              onClick={onExecute}
              disabled={runnable === 0}
              title={
                runnable > 0
                  ? `Run ${drafts} draft${drafts === 1 ? "" : "s"} and retry ${retryable} failed/blocked task${retryable === 1 ? "" : "s"}`
                  : "Nothing to run"
              }
            >
              <Icon name="play" className="h-3.5 w-3.5" />
              Execute{runnable > 0 ? ` (${runnable})` : ""}
            </button>
            <button className="btn btn-primary" onClick={onPlan}>
              <Icon name="sparkles" className="h-3.5 w-3.5" />
              Plan with AI
            </button>
          </div>
          <div className="flex items-center gap-1">
            <IconButton
              name="diff"
              title="Diff local changes against the default branch"
              onClick={onBranchDiff}
            />
            {remote && <IconButton name="github" title={remote} onClick={onOpenRemote} />}
            <IconButton
              name="code"
              title={`Open in ${editorLabel(resolved.editor)}`}
              onClick={onOpenEditor}
            />
            <IconButton name="refresh" title="Refresh" onClick={onRefresh} />
            <IconButton
              name="settings"
              title="Project settings"
              onClick={onProjectSettings}
            />
          </div>
        </div>
      </div>

      {warning && (
        <div className="mt-3 flex items-start gap-2 rounded-lg border border-warning-line bg-warning-soft px-3 py-2 text-[11.5px] leading-relaxed text-warning">
          <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div className="min-w-0 flex-1">{warning}</div>
          <button
            className="no-drag shrink-0 rounded p-0.5 transition hover:bg-ink-subtle-soft"
            onClick={onCheckTools}
            title="Re-check installed tools"
            aria-label="Re-check installed tools"
          >
            <Icon name="refresh" className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      <div className="mt-4 flex items-center gap-4">
        <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-well">
          <div
            className="h-full rounded-full bg-gradient-to-r from-accent to-accent-2 transition-all duration-500"
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="flex items-center gap-3 text-[11px] text-ink-muted">
          <span className="mono">{pct}%</span>
          {drafts > 0 && <span className="text-ink-subtle">{drafts} draft</span>}
          <span className="text-success">{succeeded} done</span>
          {running > 0 && (
            <span className="text-warning">{running} running</span>
          )}
          {interrupted > 0 && (
            <span className="text-warning">{interrupted} interrupted</span>
          )}
          {failed > 0 && <span className="text-danger">{failed} failed</span>}
          <span className="text-ink-subtle">{total} total</span>
        </div>
      </div>
    </div>
  );
}

function IconButton({
  name,
  title,
  onClick,
}: {
  name: Parameters<typeof Icon>[0]["name"];
  title: string;
  onClick: () => void;
}) {
  return (
    <button
      className="no-drag rounded-lg border border-line p-1.5 text-ink-muted transition hover:bg-hover hover:text-ink"
      onClick={onClick}
      title={title}
      aria-label={title}
    >
      <Icon name={name} className="h-3.5 w-3.5" />
    </button>
  );
}
