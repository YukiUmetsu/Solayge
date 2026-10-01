import { useState } from "react";
import type { Snapshot, Task, TaskAsk } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { ErrorNote } from "./Field";
import { DECISION_HINTS, DECISION_LABELS, PermissionDetails } from "./PermissionDetails";

const ORDER = ["once", "always", "reject"];

function buttonClass(decision: string): string {
  if (decision === "once") return "btn btn-primary";
  if (decision === "reject") return "btn btn-danger";
  return "btn btn-ghost";
}

/**
 * A focused, app-level popup for an agent permission request. It shows which
 * task wants access, the exact target(s), the provider's details, and why — so
 * the decision is made here in the app (with a desktop notification) instead of
 * through an opaque OS prompt.
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

  const decisions = ORDER.filter((d) => ask.options.length === 0 || ask.options.includes(d));

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
          {decisions.map((d) => (
            <button
              key={d}
              className={buttonClass(d)}
              disabled={busy}
              onClick={() => void decide(d)}
              title={DECISION_HINTS[d]}
            >
              {DECISION_LABELS[d] ?? d}
            </button>
          ))}
        </>
      }
    >
      <div className="space-y-3">
        <PermissionDetails ask={ask} />

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
