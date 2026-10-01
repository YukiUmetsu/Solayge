import { useState } from "react";
import type { LogEntry, Project, Snapshot, Task, TaskAsk } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { AskForm } from "./AskForm";
import { LogBody } from "./LogView";
import { askDeadline, clock, compactDuration, shortId } from "../lib/format";
import { useNow } from "../lib/useNow";

/** Quick snooze choices offered in the attention prompt. */
const SNOOZE_OPTIONS: { label: string; ms: number }[] = [
  { label: "5 minutes", ms: 5 * 60_000 },
  { label: "15 minutes", ms: 15 * 60_000 },
  { label: "1 hour", ms: 60 * 60_000 },
  { label: "4 hours", ms: 4 * 60 * 60_000 },
];

/** Read a rough duration ("30 minutes", "2h", "45") as milliseconds. */
function parseDurationMs(text: string): number | null {
  const m = text.trim().match(/(\d+(?:\.\d+)?)\s*(h|hr|hrs|hour|hours|m|min|mins|minute|minutes)?/i);
  if (!m) return null;
  const n = Number(m[1]);
  if (!Number.isFinite(n) || n <= 0) return null;
  const unit = (m[2] ?? "m").toLowerCase();
  const ms = unit.startsWith("h") ? n * 3600_000 : n * 60_000;
  return Math.min(ms, 24 * 3600_000);
}

/**
 * A focused, transient in-app popup shown whenever a running task is waiting on
 * the agent — a question or a permission request. It names the project and task,
 * shows the task's prompt, recent log, and the answer deadline (when the agent
 * supplied one), and lets a multi-part question be answered one tab at a time.
 * It also offers to defer ("remind me later") or tell the agent how long you
 * need. A desktop notification fires alongside it.
 */
export function AttentionPrompt({
  task,
  project,
  logs,
  onAnswered,
  onDismiss,
  onSnooze,
  onOpenTask,
}: {
  task: Task;
  project: Project | null;
  logs: LogEntry[];
  onAnswered: (s: Snapshot) => void;
  onDismiss: () => void;
  onSnooze: (ms: number) => void;
  onOpenTask: () => void;
}) {
  const ask = task.ask as TaskAsk;
  const isPermission = ask.kind === "permission";
  const [moreOpen, setMoreOpen] = useState(false);
  const [estimate, setEstimate] = useState("15 minutes");
  const [sending, setSending] = useState(false);
  const [hideTools, setHideTools] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const now = useNow();

  const deadline = askDeadline(ask);
  const remaining = deadline ? deadline - now : 0;

  async function tellAgentMoreTime() {
    const text = estimate.trim();
    if (!text) return;
    setSending(true);
    setErr(null);
    try {
      await api.sendTaskMessage(
        task.id,
        `I need more time before I can answer — please give me about ${text}. ` +
          `If you can, keep working on parts that do not depend on my answer; otherwise wait for me.`,
      );
      const ms = parseDurationMs(text) ?? 15 * 60_000;
      onSnooze(ms);
    } catch (e) {
      setErr(String(e));
    } finally {
      setSending(false);
    }
  }

  return (
    <Modal
      title={isPermission ? "Permission needed" : "The agent needs your input"}
      subtitle={[project?.name, task.title].filter(Boolean).join(" · ")}
      onClose={onDismiss}
      width="max-w-3xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onOpenTask}>
            <Icon name="eye" className="h-3.5 w-3.5" />
            View task &amp; logs
          </button>
          <div className="flex-1" />
          <button
            className={`btn ${moreOpen ? "btn-primary" : "btn-ghost"}`}
            onClick={() => setMoreOpen((v) => !v)}
            aria-expanded={moreOpen}
          >
            <Icon name="clock" className="h-3.5 w-3.5" />
            More time…
          </button>
          <button className="btn btn-ghost" onClick={onDismiss}>
            Dismiss
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="flex items-start gap-2 rounded-lg border border-warning-line bg-warning-soft p-3 text-[12px] text-warning">
          <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div className="min-w-0 flex-1">
            <div className="font-medium">
              {isPermission ? "This task is paused until you decide" : ask.title}
            </div>
            <div className="mt-0.5 text-[11px] text-ink-muted">
              {task.provider && (
                <span>
                  {task.provider}
                  {task.model ? ` ${task.model}` : ""} ·{" "}
                </span>
              )}
              {isPermission
                ? "permission request"
                : `${ask.fields.length} question${ask.fields.length === 1 ? "" : "s"}`}
            </div>
          </div>
        </div>

        {/* Which project and task this belongs to, so the popup is self-contained. */}
        <dl className="grid grid-cols-[5.5rem_1fr] gap-x-3 gap-y-1.5 rounded-lg border border-line bg-well px-3 py-2.5 text-[11.5px]">
          <dt className="flex items-center gap-1.5 text-ink-subtle">
            <Icon name="folder" className="h-3.5 w-3.5" />
            Project
          </dt>
          <dd className="min-w-0">
            <div className="truncate text-ink">{project?.name ?? "Unknown project"}</div>
            <div className="mono truncate text-[10.5px] text-ink-subtle">
              {project?.path ?? task.project_path}
            </div>
          </dd>

          <dt className="flex items-center gap-1.5 text-ink-subtle">
            <Icon name="layers" className="h-3.5 w-3.5" />
            Task
          </dt>
          <dd className="min-w-0">
            <div className="truncate text-ink" title={task.title}>
              {task.title}
            </div>
            <div className="mono text-[10.5px] text-ink-subtle">
              {shortId(task.id)} · {task.status}
            </div>
          </dd>
        </dl>

        {deadline && (
          <div
            className={`flex items-center gap-2 rounded-lg border p-2.5 text-[11.5px] ${
              remaining > 0
                ? "border-warning-line bg-warning-soft text-warning"
                : "border-line bg-well text-ink-muted"
            }`}
          >
            <Icon name="clock" className="h-3.5 w-3.5 shrink-0" />
            {remaining > 0 ? (
              <span>
                <span className="font-medium">
                  Answer within {compactDuration(remaining)}
                </span>
                <span className="text-ink-muted"> · by {clock(deadline)}</span>
              </span>
            ) : (
              <span>
                The agent's answer window has passed, but it is still waiting for
                you.
              </span>
            )}
          </div>
        )}

        <AskForm task={task} onAnswered={onAnswered} onError={setErr} />

        <details open className="rounded-lg border border-line bg-well">
          <summary className="flex cursor-pointer items-center gap-1.5 px-3 py-2 text-[11.5px] font-medium text-ink">
            <Icon name="code" className="h-3.5 w-3.5 text-ink-subtle" />
            Task prompt
          </summary>
          <pre className="mono scroll max-h-44 overflow-auto whitespace-pre-wrap break-words border-t border-line px-3 py-2 text-[11.5px] leading-relaxed text-ink-muted">
            {task.prompt?.trim() ? task.prompt : "No prompt."}
          </pre>
        </details>

        <div className="rounded-lg border border-line bg-well">
          <div className="flex items-center justify-between gap-2 border-b border-line px-3 py-2">
            <div className="flex items-center gap-1.5 text-[11.5px] font-medium text-ink">
              <Icon name="terminal" className="h-3.5 w-3.5 text-ink-subtle" />
              Task log
            </div>
            <button
              className="btn btn-ghost !px-2 !py-0.5 !text-[11px]"
              onClick={() => setHideTools((v) => !v)}
              title={hideTools ? "Show tool-call lines" : "Hide tool-call lines"}
              aria-pressed={hideTools}
            >
              <Icon name="eye" className="h-3 w-3" />
              {hideTools ? "Show tools" : "Hide tools"}
            </button>
          </div>
          <div className="mono scroll max-h-52 overflow-auto whitespace-pre-wrap break-words px-3 py-2 text-[11.5px] leading-relaxed">
            <LogBody entries={logs} hideTools={hideTools} />
          </div>
        </div>

        {moreOpen && (
          <div className="space-y-3 rounded-lg border border-line bg-well p-3">
            <div>
              <div className="text-[11.5px] font-medium text-ink">Remind me later</div>
              <div className="mt-1.5 flex flex-wrap gap-1.5">
                {SNOOZE_OPTIONS.map((o) => (
                  <button
                    key={o.label}
                    className="rounded-lg border border-line bg-panel px-2.5 py-1 text-[11.5px] text-ink-muted transition hover:border-accent-line hover:text-ink"
                    onClick={() => onSnooze(o.ms)}
                  >
                    {o.label}
                  </button>
                ))}
              </div>
              <p className="mt-1 text-[10.5px] text-ink-subtle">
                Hides this prompt and brings it back after the time you pick. The
                task keeps waiting.
              </p>
            </div>

            <div className="border-t border-line pt-3">
              <div className="text-[11.5px] font-medium text-ink">
                Tell the agent you need more time
              </div>
              <div className="mt-1.5 flex gap-1.5">
                <input
                  className="min-w-0 flex-1 rounded-lg border border-line bg-panel px-2 py-1 text-[12px] text-ink outline-none focus:border-accent-line"
                  placeholder="e.g. 30 minutes"
                  value={estimate}
                  onChange={(e) => setEstimate(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void tellAgentMoreTime();
                    }
                  }}
                />
                <button
                  className="btn btn-primary !py-1"
                  disabled={sending || !estimate.trim()}
                  onClick={() => void tellAgentMoreTime()}
                >
                  <Icon name="play" className="h-3.5 w-3.5" />
                  Send
                </button>
              </div>
              <p className="mt-1 text-[10.5px] text-ink-subtle">
                Posts a note to the running agent with your estimate, then snoozes
                this prompt for about that long.
              </p>
            </div>
          </div>
        )}

        {err && <p className="text-[11px] text-danger">{err}</p>}
      </div>
    </Modal>
  );
}
