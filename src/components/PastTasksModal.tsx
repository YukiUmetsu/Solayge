import { lazy, Suspense, useState, type ReactNode } from "react";
import type { DeletedTask } from "../types";
import { STATUS_META, clock, relTime, shortId } from "../lib/format";
import { separationMeta, taskKindLabel, taskSeparation } from "../lib/providers";
import { Modal } from "./Modal";
import { Icon } from "./Icons";

const ResultView = lazy(() => import("./ResultView"));

/**
 * A project's past task history: the soft-deleted tasks that no longer appear
 * in the graph. Tasks that depended on each other are grouped together and
 * listed dependencies-first. Read-only — restoring lives in Recently deleted.
 */
export function PastTasksModal({
  deleted,
  projectName,
  onClose,
}: {
  deleted: DeletedTask[];
  projectName: string;
  onClose: () => void;
}) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const groups = groupDeleted(deleted);
  const flat = groups.flat();
  const selected =
    flat.find((d) => d.task.id === selectedId) ?? flat[0] ?? null;
  const byId = new Map(deleted.map((d) => [d.task.id, d]));

  return (
    <Modal
      title="Past tasks"
      subtitle={`A read-only history of removed tasks from ${projectName}. Restore one from Recently deleted.`}
      onClose={onClose}
      width="max-w-6xl"
    >
      {deleted.length === 0 ? (
        <p className="py-6 text-center text-xs text-ink-subtle">
          No past tasks for this project yet.
        </p>
      ) : (
        <div className="-mx-5 -my-4 flex h-[calc(100%+2rem)]">
          <div className="w-64 shrink-0 overflow-y-auto border-r border-line p-3">
            {groups.map((group, i) => (
              <div key={i} className={i > 0 ? "mt-3" : ""}>
                {group.length > 1 && (
                  <div className="px-1 pb-1 text-[10px] font-semibold uppercase tracking-wider text-ink-subtle">
                    Dependency group · {group.length}
                  </div>
                )}
                <div className="space-y-1.5">
                  {group.map((d) => (
                    <TaskTab
                      key={d.task.id}
                      deleted={d}
                      selected={d.task.id === selected?.task.id}
                      onSelect={() => setSelectedId(d.task.id)}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>

          <div className="min-w-0 flex-1 overflow-y-auto p-4">
            {selected && (
              <PastTaskDetail deleted={selected} byId={byId} />
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}

/** Newest activity time: deletion, finish, start, or creation, whichever is last. */
function updated(d: DeletedTask): number {
  const t = d.task;
  return Math.max(
    d.deleted_at,
    t.finished_at ?? 0,
    t.started_at ?? 0,
    t.created_at,
  );
}

/**
 * Group tasks that reference each other through `depends_on`. Only ids present
 * in the deleted set count as edges; anything else (a still-live dependency or
 * one that never existed) is ignored. Groups are ordered by their newest
 * member, and each group is topologically ordered dependencies-first.
 */
function groupDeleted(deleted: DeletedTask[]): DeletedTask[][] {
  const byId = new Map(deleted.map((d) => [d.task.id, d]));
  const parent = new Map<string, string>();
  for (const d of deleted) parent.set(d.task.id, d.task.id);

  const find = (id: string): string => {
    let root = id;
    while (parent.get(root) !== root) root = parent.get(root)!;
    let cur = id;
    while (parent.get(cur) !== root) {
      const next = parent.get(cur)!;
      parent.set(cur, root);
      cur = next;
    }
    return root;
  };
  const union = (a: string, b: string) => {
    const ra = find(a);
    const rb = find(b);
    if (ra !== rb) parent.set(ra, rb);
  };

  for (const d of deleted) {
    for (const dep of d.task.depends_on) {
      if (byId.has(dep)) union(d.task.id, dep);
    }
  }

  const components = new Map<string, DeletedTask[]>();
  for (const d of deleted) {
    const root = find(d.task.id);
    const arr = components.get(root) ?? [];
    arr.push(d);
    components.set(root, arr);
  }

  return [...components.values()]
    .map(topoOrder)
    .sort(
      (a, b) =>
        Math.max(...b.map(updated)) - Math.max(...a.map(updated)),
    );
}

/** Kahn's algorithm over the group's internal edges; cycles fall back to input order. */
function topoOrder(group: DeletedTask[]): DeletedTask[] {
  const ids = new Set(group.map((d) => d.task.id));
  const byId = new Map(group.map((d) => [d.task.id, d]));
  const indegree = new Map<string, number>();
  const children = new Map<string, string[]>();
  for (const d of group) indegree.set(d.task.id, 0);

  for (const d of group) {
    for (const dep of d.task.depends_on) {
      if (dep === d.task.id || !ids.has(dep)) continue;
      indegree.set(d.task.id, (indegree.get(d.task.id) ?? 0) + 1);
      const arr = children.get(dep) ?? [];
      arr.push(d.task.id);
      children.set(dep, arr);
    }
  }

  const queue = group
    .filter((d) => indegree.get(d.task.id) === 0)
    .sort((a, b) => updated(a) - updated(b))
    .map((d) => d.task.id);
  const out: DeletedTask[] = [];
  const seen = new Set<string>();

  while (queue.length > 0) {
    const id = queue.shift()!;
    if (seen.has(id)) continue;
    seen.add(id);
    out.push(byId.get(id)!);
    for (const child of children.get(id) ?? []) {
      const next = (indegree.get(child) ?? 0) - 1;
      indegree.set(child, next);
      if (next === 0) queue.push(child);
    }
  }

  for (const d of group) if (!seen.has(d.task.id)) out.push(d);
  return out;
}

function TaskTab({
  deleted,
  selected,
  onSelect,
}: {
  deleted: DeletedTask;
  selected: boolean;
  onSelect: () => void;
}) {
  const meta = STATUS_META[deleted.task.status];
  return (
    <button
      onClick={onSelect}
      className={`block w-full rounded-lg border px-2.5 py-2 text-left transition ${
        selected
          ? "border-accent-line bg-accent-soft"
          : "border-line bg-well hover:border-accent-line"
      }`}
    >
      <div className="flex items-start gap-2">
        <span
          className={`mt-0.5 h-2 w-2 shrink-0 rounded-full ${meta.dot}`}
        />
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-ink">
          {deleted.task.title || "Untitled task"}
        </span>
      </div>
      <div className="mt-1.5 flex items-center gap-2 pl-4">
        <span
          className={`shrink-0 rounded-md border px-1.5 py-0.5 text-[10px] font-medium ${meta.chip} ${meta.text}`}
        >
          {meta.label}
        </span>
        <span className="flex items-center gap-1 text-[10px] text-ink-subtle">
          <Icon name="clock" className="h-3 w-3" />
          {relTime(updated(deleted))}
        </span>
      </div>
    </button>
  );
}

function PastTaskDetail({
  deleted,
  byId,
}: {
  deleted: DeletedTask;
  byId: Map<string, DeletedTask>;
}) {
  const task = deleted.task;
  const meta = STATUS_META[task.status];
  const kindLabel = taskKindLabel(task);
  const separation = separationMeta(taskSeparation(task));

  return (
    <div className="space-y-4 text-[12px]">
      <div>
        <div className="flex flex-wrap items-center gap-2">
          <span className={`h-2 w-2 rounded-full ${meta.dot}`} />
          <span className={`font-medium ${meta.text}`}>{meta.label}</span>
          {kindLabel && (
            <span className="rounded border border-accent-line bg-accent-soft px-1.5 py-0.5 text-[10px] text-accent-text">
              {kindLabel}
            </span>
          )}
          <span className="ml-auto text-[10.5px] text-ink-subtle">
            updated {relTime(updated(deleted))}
          </span>
        </div>
        <h2 className="mt-2 text-[15px] font-semibold text-ink">
          {task.title || "Untitled task"}
        </h2>
        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-ink-subtle">
          <span className="mono text-ink-faint">{shortId(task.id)}</span>
          <span className="flex items-center gap-1">
            <Icon name="clock" className="h-3 w-3" />
            deleted {clock(deleted.deleted_at)}
          </span>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Separation">{separation.short}</Field>
        <Field label="Branch">{task.branch ?? "—"}</Field>
        <Field label="Created">{clock(task.created_at)}</Field>
        <Field label="Started">{clock(task.started_at)}</Field>
        <Field label="Finished">{clock(task.finished_at)}</Field>
        <Field label="Exit code">
          {task.exit_code === null || task.exit_code === undefined
            ? "—"
            : task.exit_code}
        </Field>
      </div>

      {task.error && (
        <Field label="Error">
          <div className="rounded-lg border border-danger-line bg-danger-soft p-2.5 text-[11.5px] text-danger">
            {task.error}
          </div>
        </Field>
      )}

      {task.depends_on.length > 0 && (
        <Field label="Dependencies">
          <div className="space-y-1">
            {task.depends_on.map((id) => {
              const dep = byId.get(id);
              return (
                <div
                  key={id}
                  className="flex items-center gap-1.5 text-[11.5px] text-ink-muted"
                >
                  <Icon name="layers" className="h-3 w-3 shrink-0 text-ink-subtle" />
                  <span className="truncate">
                    after: {dep ? dep.task.title || "Untitled task" : shortId(id)}
                  </span>
                </div>
              );
            })}
          </div>
        </Field>
      )}

      <Field label="Prompt">
        <pre className="mono whitespace-pre-wrap rounded-lg border border-line bg-well-strong p-2.5 text-[11.5px] leading-relaxed text-ink-muted">
          {task.prompt || "—"}
        </pre>
      </Field>

      {deleted.summary && (
        <Field label="Output summary">
          <pre className="mono max-h-72 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-well p-2.5 text-[11px] leading-relaxed text-ink-muted">
            {deleted.summary}
          </pre>
        </Field>
      )}

      {deleted.diff && (
        <Field label="Diff">
          <pre className="mono max-h-72 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-well p-2.5 text-[11px] leading-relaxed text-ink-muted">
            {deleted.diff}
          </pre>
        </Field>
      )}

      {task.result && (
        <Field label="Result">
          <div className="rounded-lg border border-line bg-well">
            <Suspense
              fallback={
                <div className="p-3 text-xs text-ink-subtle">Loading…</div>
              }
            >
              <ResultView task={task} />
            </Suspense>
          </div>
        </Field>
      )}
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div>
      <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
        {label}
      </div>
      <div className="text-ink-muted">{children}</div>
    </div>
  );
}
