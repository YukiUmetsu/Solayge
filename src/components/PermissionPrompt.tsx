import { useState } from "react";
import type { Snapshot, Task, TaskAsk } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { ErrorNote } from "./Field";

const DECISIONS: { id: string; label: string; primary?: boolean; danger?: boolean }[] = [
  { id: "once", label: "Allow once", primary: true },
  { id: "always", label: "Always allow" },
  { id: "reject", label: "Reject", danger: true },
];

/**
 * A focused, app-level popup for an agent permission request. It shows which
 * task wants access, the exact directory/command/URL, and why — so the decision
 * is made here in the app (with a desktop notification) instead of through an
 * opaque OS prompt.
 */
export function PermissionPrompt({
  task,
  projectName,
  onAnswered,
  onDismiss,
  onOpenTask,
}: {
  task: Task;
  projectName: string | null;
  onAnswered: (s: Snapshot) => void;
  onDismiss: () => void;
  onOpenTask: () => void;
}) {
  const ask = task.ask as TaskAsk;
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function decide(decision: string) {
    setBusy(true);
    setErr(null);
    try {
      onAnswered(await api.answerTask(task.id, { decision }));
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Permission needed"
      subtitle={[projectName, task.title].filter(Boolean).join(" · ")}
      onClose={onDismiss}
      width="max-w-lg"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onOpenTask} disabled={busy}>
            <Icon name="eye" className="h-3.5 w-3.5" />
            View task
          </button>
          <div className="flex-1" />
          {DECISIONS.map((d) => (
            <button
              key={d.id}
              className={
                d.primary ? "btn btn-primary" : d.danger ? "btn btn-danger" : "btn btn-ghost"
              }
              disabled={busy}
              onClick={() => void decide(d.id)}
              title={
                d.id === "always"
                  ? `Always allow this for ${task.title}`
                  : d.id === "once"
                    ? "Allow just this time"
                    : "Deny this request"
              }
            >
              {d.label}
            </button>
          ))}
        </>
      }
    >
      <div className="space-y-3">
        <div className="flex items-start gap-2 rounded-lg border border-warning-line bg-warning-soft p-3 text-[12px] text-warning">
          <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div>
            <div className="font-medium">
              {ask.purpose ?? "This task is asking for permission"}
            </div>
            <div className="text-[11px] text-ink-muted">
              It is paused until you answer.
            </div>
          </div>
        </div>

        {ask.resource && (
          <div>
            <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
              Directory / target
            </div>
            <pre className="mono whitespace-pre-wrap break-all rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink">
              {ask.resource}
            </pre>
          </div>
        )}

        {ask.message && (
          <div>
            <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
              Details
            </div>
            <pre className="mono whitespace-pre-wrap break-all rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink-muted">
              {ask.message}
            </pre>
          </div>
        )}

        <div className="text-[11px] text-ink-subtle">
          Provider: <span className="mono text-ink-muted">{task.provider ?? "default"}</span>
          {task.model ? (
            <>
              {" · "}
              <span className="mono text-ink-muted">{task.model}</span>
            </>
          ) : null}
        </div>

        <ErrorNote error={err} />
      </div>
    </Modal>
  );
}
