import { useState } from "react";
import type { DeletedTask } from "../types";
import { STATUS_META, clock, shortId } from "../lib/format";
import { Modal } from "./Modal";
import { Icon } from "./Icons";

/**
 * The project's recently deleted tasks. A deleted task keeps its prompt,
 * details, dependencies, a diff, and the tail of its log, so it can be restored.
 */
export function DeletedTasksModal({
  deleted,
  onClose,
  onRestore,
}: {
  deleted: DeletedTask[];
  onClose: () => void;
  onRestore: (id: string) => void;
}) {
  const [openId, setOpenId] = useState<string | null>(null);
  const rows = [...deleted].sort((a, b) => b.deleted_at - a.deleted_at);

  return (
    <Modal
      title="Recently deleted"
      subtitle="Deleted tasks keep their prompt, details, dependencies, a diff, and the tail of their log. Restore one to put it back."
      onClose={onClose}
      width="max-w-3xl"
    >
      {rows.length === 0 ? (
        <p className="py-6 text-center text-xs text-ink-subtle">
          Nothing has been deleted in this project.
        </p>
      ) : (
        <div className="space-y-2">
          {rows.map((d) => {
            const meta = STATUS_META[d.task.status];
            const open = openId === d.task.id;
            return (
              <div key={d.task.id} className="card rounded-xl px-3.5 py-3">
                <div className="flex items-start gap-2.5">
                  <span
                    className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${meta.dot}`}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13px] font-medium text-ink">
                      {d.task.title || "Untitled task"}
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-ink-subtle">
                      <span
                        className={`rounded-md border px-1.5 py-0.5 text-[10px] font-medium ${meta.chip} ${meta.text}`}
                      >
                        {meta.label}
                      </span>
                      <span className="flex items-center gap-1">
                        <Icon name="clock" className="h-3 w-3" />
                        deleted {clock(d.deleted_at)}
                      </span>
                      {d.task.depends_on.length > 0 && (
                        <span className="flex items-center gap-1">
                          <Icon name="layers" className="h-3 w-3" />
                          after {d.task.depends_on.length}
                        </span>
                      )}
                      <span className="mono text-ink-faint">
                        {shortId(d.task.id)}
                      </span>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      className="btn btn-ghost !px-2 !py-1"
                      onClick={() => setOpenId(open ? null : d.task.id)}
                      aria-expanded={open}
                    >
                      <Icon name="eye" className="h-3 w-3" />
                      {open ? "Hide" : "Details"}
                    </button>
                    <button
                      className="btn btn-primary !px-2 !py-1"
                      onClick={() => onRestore(d.task.id)}
                    >
                      <Icon name="retry" className="h-3 w-3" />
                      Restore
                    </button>
                  </div>
                </div>

                {open && (
                  <div className="mt-3 space-y-3 border-t border-line pt-3">
                    <div>
                      <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                        Prompt
                      </div>
                      <pre className="mono whitespace-pre-wrap rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink-muted">
                        {d.task.prompt}
                      </pre>
                    </div>
                    {d.diff && (
                      <div>
                        <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                          Diff
                        </div>
                        <pre className="mono max-h-64 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink-muted">
                          {d.diff}
                        </pre>
                      </div>
                    )}
                    {d.summary && (
                      <div>
                        <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                          Output summary
                        </div>
                        <pre className="mono max-h-64 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink-muted">
                          {d.summary}
                        </pre>
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
