import type { Task } from "../types";
import { useNow } from "../lib/useNow";
import { isClearable } from "../lib/tasks";
import { TaskCard, buildTree } from "./TaskCard";
import { Icon } from "./Icons";

export function TaskTree({
  tasks,
  selectedId,
  onSelect,
  onEdit,
  onStartNow,
  onCancel,
  onRetry,
  onDelete,
  onShowDiff,
  onCombine,
  onClearFinished,
  deletedCount,
  onShowDeleted,
  onShowPast,
}: {
  tasks: Task[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onEdit: (id: string) => void;
  onStartNow: (id: string) => void;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
  onDelete: (id: string) => void;
  onShowDiff: (id: string) => void;
  onCombine: () => void;
  onClearFinished: () => void;
  deletedCount: number;
  onShowDeleted: () => void;
  onShowPast: () => void;
}) {
  const rows = buildTree(tasks);
  const now = useNow();
  // Mirrors the backend `is_clearable`; the rule lives in `lib/tasks`.
  const clearable = tasks.filter(isClearable).length;

  if (tasks.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-10 text-center">
        <img src="/app-icon.png" alt="" className="h-14 w-14 rounded-2xl shadow-lg" />
        <div className="text-sm font-medium text-ink">
          Plan a chain or a parallel tree
        </div>
        <p className="max-w-sm text-xs leading-relaxed text-ink-subtle">
          Use <span className="text-accent-text">Plan with AI</span> to turn a goal
          into a dependency graph, or add tasks manually. Independent tasks run in
          parallel, each in its own git worktree. Nothing runs until you press{" "}
          <span className="text-ink-muted">Execute</span>.
        </p>
        <div className="mt-1 flex gap-2">
          <button className="btn btn-ghost" onClick={onCombine}>
            <Icon name="diff" className="h-3.5 w-3.5" />
            Combine branches
          </button>
          <button className="btn btn-ghost" onClick={onShowPast}>
            <Icon name="clock" className="h-3.5 w-3.5" />
            Show past tasks
          </button>
          {deletedCount > 0 && (
            <button className="btn btn-ghost" onClick={onShowDeleted}>
              <Icon name="retry" className="h-3.5 w-3.5" />
              Recently deleted · {deletedCount}
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="mb-3 flex items-center justify-between gap-3">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-subtle">
          Graph · {tasks.length} task{tasks.length === 1 ? "" : "s"}
        </span>
        <div className="flex items-center gap-2">
          {deletedCount > 0 && (
            <button className="btn btn-ghost" onClick={onShowDeleted}>
              <Icon name="retry" className="h-3.5 w-3.5" />
              Recently deleted · {deletedCount}
            </button>
          )}
          {clearable > 0 && (
            <button className="btn btn-ghost" onClick={onClearFinished}>
              <Icon name="trash" className="h-3.5 w-3.5" />
              Clear finished
            </button>
          )}
        </div>
      </div>
      {rows.map(({ task, depth }) => (
        <div
          key={task.id}
          className="relative"
          style={{ marginLeft: depth * 22 }}
        >
          {depth > 0 && (
            <>
              <span
                className="absolute -left-[11px] top-0 h-6 w-[11px] rounded-bl-lg border-b border-l border-line-strong"
                aria-hidden
              />
              <span
                className="absolute -left-[11px] bottom-0 top-6 w-px bg-line-strong"
                aria-hidden
              />
            </>
          )}
          <TaskCard
            task={task}
            now={now}
            selected={task.id === selectedId}
            onSelect={() => onSelect(task.id)}
            onEdit={() => onEdit(task.id)}
            onStartNow={() => onStartNow(task.id)}
            onCancel={() => onCancel(task.id)}
            onRetry={() => onRetry(task.id)}
            onDelete={() => onDelete(task.id)}
            onShowDiff={() => onShowDiff(task.id)}
          />
        </div>
      ))}

      <button
        onClick={onCombine}
        className="mt-1 flex w-full items-center justify-center gap-2 rounded-xl border border-dashed border-line-strong px-3 py-3 text-[12px] text-ink-subtle transition hover:border-accent-line hover:text-accent-text"
      >
        <Icon name="diff" className="h-3.5 w-3.5" />
        Combine branches into one
      </button>
    </div>
  );
}
