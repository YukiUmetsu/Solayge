import type { Project, Snapshot, Task } from "../types";
import { useTheme } from "../theme";
import { useNow } from "../lib/useNow";
import { Icon } from "./Icons";

export function Sidebar({
  snapshot,
  selected,
  width,
  onSelect,
  onAdd,
  onRemove,
  onConcurrency,
  onSettings,
  onReorder,
  onCollapse,
}: {
  snapshot: Snapshot;
  selected: string | null;
  width: number;
  onSelect: (path: string) => void;
  onAdd: () => void;
  onRemove: (path: string) => void;
  onConcurrency: (n: number) => void;
  onSettings: () => void;
  onReorder: (paths: string[]) => void;
  onCollapse: () => void;
}) {
  const total = snapshot.tasks.length;
  const running = snapshot.tasks.filter((t) => t.status === "running").length;
  const done = snapshot.tasks.filter((t) => t.status === "succeeded").length;
  const failed = snapshot.tasks.filter((t) => t.status === "failed").length;
  const { theme, toggle, pref } = useTheme();
  const now = useNow();

  /** Move a project one slot up (-1) or down (+1). */
  function move(i: number, delta: number) {
    const j = i + delta;
    const paths = snapshot.projects.map((p) => p.path);
    if (j < 0 || j >= paths.length) return;
    [paths[i], paths[j]] = [paths[j], paths[i]];
    onReorder(paths);
  }

  return (
    <aside
      style={{ width }}
      className="panel flex shrink-0 flex-col rounded-none border-y-0 border-l-0"
    >
      <div
        className="drag-region titlebar-pad flex items-center gap-2.5 px-4 pb-4"
        data-tauri-drag-region="deep"
      >
        <img
          src="/app-icon.png"
          alt=""
          className="h-8 w-8 shrink-0 rounded-lg shadow-lg"
        />
        <div className="min-w-0 flex-1 no-drag">
          <div className="truncate text-[13px] font-semibold text-ink">
            Solayge
          </div>
          <div className="truncate text-[10.5px] text-ink-subtle">
            sequential · parallel · worktrees
          </div>
        </div>
        <button
          className="no-drag rounded-lg border border-line p-1.5 text-ink-muted transition hover:bg-hover hover:text-ink"
          onClick={toggle}
          title={
            (pref === "system" ? "System theme — " : "") +
            (theme === "dark" ? "switch to light" : "switch to dark")
          }
          aria-label="Toggle theme"
        >
          <Icon name={theme === "dark" ? "sun" : "moon"} className="h-4 w-4" />
        </button>
        <button
          className="no-drag rounded-lg border border-line p-1.5 text-ink-muted transition hover:bg-hover hover:text-ink"
          onClick={onSettings}
          title="Settings"
          aria-label="Settings"
        >
          <Icon name="settings" className="h-4 w-4" />
        </button>
        <button
          className="no-drag rounded-lg border border-line p-1.5 text-ink-muted transition hover:bg-hover hover:text-ink"
          onClick={onCollapse}
          title="Hide projects"
          aria-label="Hide projects"
        >
          <Icon name="chevron" className="h-4 w-4 rotate-180" />
        </button>
      </div>

      <div className="flex items-center justify-between px-4 pb-2 pt-1">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-subtle">
          Projects
        </span>
        <button
          className="btn btn-ghost !px-2 !py-1 no-drag"
          onClick={onAdd}
          title="Add a project folder"
        >
          <Icon name="plus" className="h-3.5 w-3.5" />
          Add
        </button>
      </div>

      <div className="scroll flex-1 space-y-1 px-2">
        {snapshot.projects.length === 0 && (
          <p className="px-2 py-4 text-xs leading-relaxed text-ink-subtle">
            No projects yet. Add a git repository folder to start planning tasks.
          </p>
        )}
        {snapshot.projects.map((p: Project, i: number) => {
          const active = p.path === selected;
          const ptasks = snapshot.tasks.filter((t) => t.project_path === p.path);
          return (
            <div
              key={p.path}
              onClick={() => onSelect(p.path)}
              className={`group flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition ${
                active
                  ? "bg-accent-soft text-ink ring-1 ring-accent-line"
                  : "text-ink-muted hover:bg-hover"
              }`}
            >
              <Icon
                name="folder"
                className={`h-4 w-4 shrink-0 ${active ? "text-accent-text" : "text-ink-subtle"}`}
              />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] font-medium">{p.name}</div>
                <div className="truncate text-[10.5px] text-ink-subtle">
                  {ptasks.length} task{ptasks.length === 1 ? "" : "s"}
                </div>
              </div>
              <ProjectStatus tasks={ptasks} now={now} />
              <div className="flex shrink-0 items-center opacity-0 transition group-hover:opacity-100">
                <button
                  className="rounded p-1 text-ink-subtle transition hover:bg-hover hover:text-ink disabled:opacity-30 disabled:hover:bg-transparent"
                  disabled={i === 0}
                  onClick={(e) => {
                    e.stopPropagation();
                    move(i, -1);
                  }}
                  title="Move up"
                  aria-label="Move up"
                >
                  <Icon name="chevron" className="h-3.5 w-3.5 -rotate-90" />
                </button>
                <button
                  className="rounded p-1 text-ink-subtle transition hover:bg-hover hover:text-ink disabled:opacity-30 disabled:hover:bg-transparent"
                  disabled={i === snapshot.projects.length - 1}
                  onClick={(e) => {
                    e.stopPropagation();
                    move(i, 1);
                  }}
                  title="Move down"
                  aria-label="Move down"
                >
                  <Icon name="chevron" className="h-3.5 w-3.5 rotate-90" />
                </button>
                <button
                  className="rounded p-1 text-ink-subtle transition hover:bg-hover hover:text-danger"
                  onClick={(e) => {
                    e.stopPropagation();
                    onRemove(p.path);
                  }}
                  title="Remove project"
                  aria-label="Remove project"
                >
                  <Icon name="x" className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          );
        })}
      </div>

      <div className="space-y-3 border-t border-line px-4 py-4">
        <div>
          <div className="mb-2 flex items-center justify-between text-[11px] text-ink-muted">
            <span className="flex items-center gap-1.5">
              <Icon name="layers" className="h-3.5 w-3.5" /> Max parallel
            </span>
            <span className="mono text-ink">{snapshot.concurrency}</span>
          </div>
          <input
            type="range"
            min={1}
            max={8}
            value={snapshot.concurrency}
            onChange={(e) => onConcurrency(Number(e.target.value))}
            className="w-full accent-accent"
          />
        </div>
        <div className="grid grid-cols-3 gap-2 text-center">
          <Stat label="Tasks" value={total} />
          <Stat label="Running" value={running} tone="text-warning" />
          <Stat label="Done" value={done} tone="text-success" />
        </div>
        {failed > 0 && (
          <div className="flex items-center gap-1.5 text-[11px] text-danger">
            <Icon name="alert" className="h-3.5 w-3.5" />
            {failed} failed
          </div>
        )}
      </div>
    </aside>
  );
}

function Stat({
  label,
  value,
  tone = "text-ink",
}: {
  label: string;
  value: number;
  tone?: string;
}) {
  return (
    <div className="rounded-lg border border-line bg-well px-2 py-1.5">
      <div className={`mono text-base font-semibold ${tone}`}>{value}</div>
      <div className="text-[10px] uppercase tracking-wide text-ink-subtle">
        {label}
      </div>
    </div>
  );
}

/** Compact "how long" label: seconds under a minute, then minutes, then hours. */
function elapsedLabel(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/**
 * A small always-visible status chip for a project: a spinner and elapsed time
 * while anything runs, then failures, blocked, pending, or a done check.
 */
function ProjectStatus({ tasks, now }: { tasks: Task[]; now: number }) {
  const chip =
    "flex shrink-0 items-center gap-1 rounded-md border px-1.5 py-0.5 text-[10px] font-medium";

  const running = tasks.filter((t) => t.status === "running");
  if (running.length > 0) {
    const started = running
      .map((t) => t.started_at)
      .filter((s): s is number => s != null);
    const elapsed = started.length ? now - Math.min(...started) : 0;
    return (
      <span
        className={`${chip} border-warning-line bg-warning-soft text-warning`}
        title={`${running.length} running`}
      >
        <Icon name="refresh" className="h-3 w-3 animate-spin" />
        {elapsedLabel(elapsed)}
      </span>
    );
  }

  const failed = tasks.filter((t) => t.status === "failed").length;
  if (failed > 0) {
    return (
      <span
        className={`${chip} border-danger-line bg-danger-soft text-danger`}
        title={`${failed} failed`}
      >
        <Icon name="alert" className="h-3 w-3" />
        {failed}
      </span>
    );
  }

  const blocked = tasks.filter((t) => t.status === "blocked").length;
  if (blocked > 0) {
    return (
      <span
        className={`${chip} border-violet-line bg-violet-soft text-violet`}
        title={`${blocked} blocked, waiting for you`}
      >
        <Icon name="alert" className="h-3 w-3" />
        {blocked}
      </span>
    );
  }

  const pending = tasks.filter((t) =>
    ["draft", "waiting", "ready"].includes(t.status),
  ).length;
  if (pending > 0) {
    return (
      <span
        className={`${chip} border-line bg-well text-ink-subtle`}
        title={`${pending} not finished`}
      >
        <Icon name="clock" className="h-3 w-3" />
        {pending}
      </span>
    );
  }

  const done = tasks.filter((t) => t.status === "succeeded").length;
  if (done > 0) {
    return (
      <span
        className={`${chip} border-success-line bg-success-soft text-success`}
        title={`${done} done`}
      >
        <Icon name="check" className="h-3 w-3" />
        {done}
      </span>
    );
  }

  return null;
}
