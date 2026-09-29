import { useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { DiffResult, Project, Task, Worktree } from "../types";
import { api } from "../api";
import {
  STATUS_META,
  PROFILE_META,
  clock,
  isLive,
  runDuration,
  shortId,
} from "../lib/format";
import { providerLabel, reviewModeLabel, reviewStatusMeta, separationMeta, taskSeparation } from "../lib/providers";
import { useNow } from "../lib/useNow";
import { Icon } from "./Icons";
import { DiffBody } from "./DiffView";

type Tab = "logs" | "diff" | "worktrees" | "review" | "details";

export function DetailPanel({
  task,
  project,
  logs,
  width,
  onRemoveWorktree,
  onCollapse,
}: {
  task: Task | null;
  project: Project | null;
  logs: string[];
  width: number;
  onRemoveWorktree: (id: string) => void;
  onCollapse: () => void;
}) {
  const [tab, setTab] = useState<Tab>("logs");
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffErr, setDiffErr] = useState<string | null>(null);
  const [worktrees, setWorktrees] = useState<Worktree[]>([]);
  const logRef = useRef<HTMLDivElement>(null);
  const now = useNow();

  const diffTarget = task?.worktree_path ?? project?.path ?? null;
  const projectPath = project?.path ?? null;

  const loadDiff = useMemo(
    () => async () => {
      if (!diffTarget) return;
      setDiffLoading(true);
      setDiffErr(null);
      try {
        setDiff(await api.gitDiff(diffTarget));
      } catch (e) {
        setDiffErr(String(e));
      } finally {
        setDiffLoading(false);
      }
    },
    [diffTarget],
  );

  const loadWorktrees = useMemo(
    () => async () => {
      if (!projectPath) return;
      try {
        setWorktrees(await api.projectWorktrees(projectPath));
      } catch {
        setWorktrees([]);
      }
    },
    [projectPath],
  );

  // Depend only on stable values: the snapshot object is replaced on every poll,
  // so depending on `project`/`task` objects re-ran these loaders each poll and
  // made the diff flash "Loading…" once a second.
  useEffect(() => {
    if (tab === "diff") void loadDiff();
    if (tab === "worktrees") void loadWorktrees();
  }, [tab, loadDiff, loadWorktrees, task?.id, task?.status]);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs, tab]);

  const tabs: [Tab, string, Parameters<typeof Icon>[0]["name"]][] = [
    ["logs", "Logs", "terminal"],
    ["diff", "Diff", "diff"],
    ["worktrees", "Worktrees", "branch"],
    ...(task?.review && task.review.mode !== "off"
      ? ([["review", "Review", "check"]] as [
          Tab,
          string,
          Parameters<typeof Icon>[0]["name"],
        ][])
      : []),
    ["details", "Details", "layers"],
  ];

  return (
    <div
      style={{ width }}
      className="panel flex h-full shrink-0 flex-col rounded-none border-y-0 border-r-0"
    >
      <div className="flex gap-1 border-b border-line px-3 pt-3">
        {tabs.map(([id, label, icon]) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={`flex items-center gap-1.5 rounded-t-lg px-3 py-2 text-[12px] transition ${
              tab === id
                ? "border-b-2 border-accent text-ink"
                : "text-ink-muted hover:text-ink"
            }`}
          >
            <Icon name={icon} className="h-3.5 w-3.5" />
            {label}
          </button>
        ))}
        <button
          onClick={onCollapse}
          className="mb-1 ml-auto self-center rounded-lg p-1.5 text-ink-subtle transition hover:bg-hover hover:text-ink"
          title="Hide details"
          aria-label="Hide details"
        >
          <Icon name="chevron" className="h-3.5 w-3.5" />
        </button>
      </div>

      {!task && tab !== "worktrees" ? (
        <EmptyDetail />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          {tab === "logs" && (
            <LogsView task={task} logs={logs} logRef={logRef} now={now} />
          )}

          {tab === "diff" && (
            <div className="scroll min-h-0 flex-1 p-3">
              <div className="mb-2 flex items-center justify-between gap-2">
                <span className="mono truncate text-[11px] text-ink-subtle">
                  {diffTarget}
                </span>
                <div className="flex shrink-0 items-center gap-1">
                  {/* Inline in the fixed-height header so it never reflows the
                      diff below while loading. */}
                  <span
                    className={`text-[10.5px] text-ink-subtle transition-opacity ${
                      diffLoading ? "opacity-100" : "opacity-0"
                    }`}
                    aria-hidden={!diffLoading}
                  >
                    Loading…
                  </span>
                  <button className="btn btn-ghost !px-2 !py-1" onClick={() => void loadDiff()}>
                    <Icon name="refresh" className="h-3 w-3" />
                  </button>
                </div>
              </div>
              {diffErr && <p className="text-xs text-danger">{diffErr}</p>}
              {diff && <DiffBody result={diff} />}
            </div>
          )}

          {tab === "worktrees" && (
            <div className="scroll min-h-0 flex-1 p-3">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-[11px] text-ink-muted">
                  {worktrees.length} worktree(s)
                </span>
                <button
                  className="btn btn-ghost !px-2 !py-1"
                  onClick={() => void loadWorktrees()}
                >
                  <Icon name="refresh" className="h-3 w-3" />
                </button>
              </div>
              <div className="space-y-2">
                {worktrees.map((w) => (
                  <div
                    key={w.path}
                    className="card rounded-lg px-3 py-2 text-[11.5px]"
                  >
                    <div className="flex items-center gap-2">
                      <Icon
                        name="branch"
                        className={`h-3.5 w-3.5 ${
                          w.is_main ? "text-success" : "text-accent-text"
                        }`}
                      />
                      <span className="mono truncate text-ink">
                        {w.branch ?? "(detached)"}
                      </span>
                      {w.is_main && (
                        <span className="rounded bg-success-soft px-1.5 text-[10px] text-success">
                          main
                        </span>
                      )}
                      <button
                        className="ml-auto text-ink-subtle hover:text-ink"
                        onClick={() => void revealItemInDir(w.path)}
                        title="Reveal"
                      >
                        <Icon name="external" className="h-3.5 w-3.5" />
                      </button>
                    </div>
                    <div className="mono mt-0.5 truncate text-[10.5px] text-ink-subtle">
                      {w.path}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {tab === "review" && task && <ReviewView task={task} />}

          {tab === "details" && task && (
            <DetailsView
              task={task}
              now={now}
              onRemoveWorktree={onRemoveWorktree}
            />
          )}
        </div>
      )}
    </div>
  );
}

function EmptyDetail() {
  return (
    <div className="flex flex-1 items-center justify-center px-8 text-center">
      <p className="text-xs leading-relaxed text-ink-subtle">
        Select a task to stream its logs, inspect its diff, and see its worktree.
      </p>
    </div>
  );
}

function LogsView({
  task,
  logs,
  logRef,
  now,
}: {
  task: Task | null;
  logs: string[];
  logRef: RefObject<HTMLDivElement | null>;
  now: number;
}) {
  if (!task) return <EmptyDetail />;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-2 px-3 py-2 text-[11px] text-ink-subtle">
        <span className="mono truncate">{shortId(task.id)}</span>
        {task.status === "running" ? (
          <span className="flex items-center gap-1 text-warning">
            <span className="h-1.5 w-1.5 rounded-full bg-warning running-dot" />
            streaming · {runDuration(task, now)}
          </span>
        ) : (
          task.started_at && (
            <span className="flex items-center gap-1">
              <Icon name="clock" className="h-3 w-3" />
              ran for {runDuration(task, now)}
            </span>
          )
        )}
      </div>
      <div
        ref={logRef}
        className="scroll mono min-h-0 flex-1 whitespace-pre-wrap break-words rounded-none border-t border-line bg-well-strong p-3 text-[11.5px] leading-relaxed text-ink-muted"
      >
        {logs.length === 0 ? (
          <span className="text-ink-faint">No output yet.</span>
        ) : (
          logs.join("\n")
        )}
      </div>
    </div>
  );
}

function ReviewView({ task }: { task: Task }) {
  const review = task.review && task.review.mode !== "off" ? task.review : null;
  const [log, setLog] = useState("");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const now = useNow();

  const status = review?.status ?? "none";
  // Reload on task/verdict changes only. Depending on the `review` object made
  // this refetch on every snapshot poll; the rendered body keys off `log`, so
  // this alone avoids the "Loading…" flash.
  useEffect(() => {
    if (!review) return;
    let alive = true;
    setLoading(true);
    api
      .reviewLog(task.id)
      .then((t) => alive && setLog(t))
      .catch((e) => alive && setErr(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task.id, status]);

  if (!review) return <EmptyDetail />;
  const meta = reviewStatusMeta(review.status);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="space-y-2 border-b border-line px-3 py-3">
        <div className={`flex items-center gap-2 text-[12px] ${meta.text}`}>
          <span className={`h-2 w-2 rounded-full ${meta.dot}`} />
          <span className="font-medium">{meta.label}</span>
          <span className="text-ink-subtle">· {reviewModeLabel(review.mode)}</span>
        </div>
        <div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-ink-subtle">
          <span>
            {providerLabel(review.provider ?? null)}
            {review.model ? ` · ${review.model}` : ""}
          </span>
          {review.started_at && <span>started {clock(review.started_at)}</span>}
          {review.started_at && (
            <span>ran {runDuration(review, now)}</span>
          )}
        </div>
        {review.summary && (
          <pre className="mono max-h-40 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink-muted">
            {review.summary}
          </pre>
        )}
        <div className="flex items-center justify-between">
          <span className="text-[11px] text-ink-subtle">Reviewer log</span>
          {review.status === "running" && (
            <span className="flex items-center gap-1 text-[11px] text-warning">
              <span className="h-1.5 w-1.5 rounded-full bg-warning running-dot" />
              reviewing
            </span>
          )}
        </div>
      </div>
      <div className="scroll mono min-h-0 flex-1 whitespace-pre-wrap break-words border-t border-line bg-well-strong p-3 text-[11.5px] leading-relaxed text-ink-muted">
        {loading && !log
          ? "Loading…"
          : err
            ? err
            : log.replace(/\n$/, "") || "No reviewer output yet."}
      </div>
    </div>
  );
}

function DetailsView({
  task,
  now,
  onRemoveWorktree,
}: {
  task: Task;
  now: number;
  onRemoveWorktree: (id: string) => void;
}) {
  const meta = STATUS_META[task.status];
  const pm = PROFILE_META[task.profile] ?? PROFILE_META.autonomous;
  const review = task.review && task.review.mode !== "off" ? task.review : null;
  const reviewMeta = review ? reviewStatusMeta(review.status) : null;
  return (
    <div className="scroll min-h-0 flex-1 space-y-4 p-4 text-[12px]">
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${meta.dot}`} />
        <span className={`font-medium ${meta.text}`}>{meta.label}</span>
        {task.exit_code !== null && task.exit_code !== undefined && (
          <span className="mono text-ink-subtle">exit {task.exit_code}</span>
        )}
        {isLive(task) && (
          <span className="flex items-center gap-1 text-warning">
            <span className="h-1.5 w-1.5 rounded-full bg-warning running-dot" />
            running for {runDuration(task, now)}
          </span>
        )}
      </div>

      <div className="rounded-lg border border-line bg-well p-2.5">
        <div className="flex items-center gap-2 text-[11.5px]">
          <Icon name="layers" className="h-3 w-3 text-ink-subtle" />
          <span className="font-medium text-ink">
            {providerLabel(task.provider ?? null)}
          </span>
          <span className="mono text-ink-subtle">
            {task.model ?? "provider default"}
          </span>
          {task.used_fallback && (
            <span className="rounded bg-warning-soft px-1.5 text-[10px] text-warning">
              used backup
            </span>
          )}
        </div>
        <p className="mt-1 text-[11px] text-ink-subtle">
          Backup:{" "}
          {task.fallback_provider
            ? providerLabel(task.fallback_provider)
            : "none"}
          {task.fallback_model ? ` · ${task.fallback_model}` : ""}
        </p>
      </div>

      <div className="rounded-lg border border-line bg-well p-2.5">
        <div className="flex items-center gap-2 text-[11.5px]">
          <span className={`h-2 w-2 rounded-full ${pm.dot}`} />
          <span className={`font-medium ${pm.text}`}>{pm.label} permissions</span>
        </div>
        <p className="mt-1 text-[11px] leading-relaxed text-ink-muted">
          {pm.desc}
        </p>
      </div>

      {review && reviewMeta && (
        <div className="rounded-lg border border-line bg-well p-2.5">
          <div className={`flex items-center gap-2 text-[11.5px] ${reviewMeta.text}`}>
            <span className={`h-2 w-2 rounded-full ${reviewMeta.dot}`} />
            <span className="font-medium">{reviewMeta.label}</span>
            <span className="text-ink-subtle">· {reviewModeLabel(review.mode)}</span>
          </div>
          {review.summary && (
            <pre className="mono mt-1 whitespace-pre-wrap text-[11px] leading-relaxed text-ink-muted">
              {review.summary}
            </pre>
          )}
          <p className="mt-1 text-[10.5px] text-ink-subtle">
            Full reviewer log on the Review tab.
          </p>
        </div>
      )}

      <Field label="Prompt">
        <pre className="mono whitespace-pre-wrap rounded-lg border border-line bg-well-strong p-2.5 text-[11.5px] leading-relaxed text-ink-muted">
          {task.prompt}
        </pre>
      </Field>

      {task.command && (
        <Field label="Command">
          <pre className="mono whitespace-pre-wrap rounded-lg border border-line bg-well-strong p-2.5 text-[11.5px] leading-relaxed text-ink-muted">
            {task.command}
          </pre>
        </Field>
      )}

      {task.merge && (
        <Field label="Combine">
          <div className="space-y-1 text-[11.5px] text-ink-muted">
            <div>
              {task.merge.sources.join(", ") || "(none)"} →{" "}
              {task.merge.target ?? "default branch"} · {task.merge.strategy}
            </div>
            {task.merge.test_command && (
              <div className="mono text-[11px] text-ink-subtle">
                tests: {task.merge.test_command}
              </div>
            )}
          </div>
        </Field>
      )}

      <div className="grid grid-cols-2 gap-3">
        <Field label="Separation">
          {separationMeta(taskSeparation(task)).short}
        </Field>
        <Field label="Base ref">{task.base_ref ?? "HEAD"}</Field>
        <Field label="Branch">{task.branch ?? "—"}</Field>
        <Field label="Created">{clock(task.created_at)}</Field>
        <Field label="Started">{clock(task.started_at)}</Field>
        <Field label="Finished">{clock(task.finished_at)}</Field>
        <Field label="Ran for">{runDuration(task, now)}</Field>
      </div>

      {task.worktree_path && (
        <Field label="Worktree">
          <div className="flex items-center gap-2">
            <span className="mono truncate text-[11px] text-ink-muted">
              {task.worktree_path}
            </span>
            <button
              className="btn btn-ghost !px-2 !py-1"
              onClick={() => void revealItemInDir(task.worktree_path!)}
            >
              <Icon name="external" className="h-3 w-3" />
            </button>
            <button
              className="btn btn-danger !px-2 !py-1"
              onClick={() => onRemoveWorktree(task.id)}
            >
              Remove
            </button>
          </div>
        </Field>
      )}

      {task.error && (
        <Field label="Error">
          <div className="rounded-lg border border-danger-line bg-danger-soft p-2.5 text-[11.5px] text-danger">
            {task.error}
          </div>
        </Field>
      )}

      {task.last_permission && (
        <Field label="Last permission request">
          <div className="rounded-lg border border-warning-line bg-warning-soft p-2.5 text-[11.5px] text-warning">
            <div className="mb-1 flex items-center gap-1.5 text-[10.5px] uppercase tracking-wide">
              <Icon name="alert" className="h-3 w-3" />
              auto-rejected (no approver yet)
            </div>
            <span className="mono break-all">{task.last_permission}</span>
          </div>
        </Field>
      )}

      <Field label="Task ID">
        <span className="mono text-[11px] text-ink-muted">{task.id}</span>
      </Field>
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
