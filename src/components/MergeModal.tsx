import { useEffect, useState } from "react";
import type {
  BranchInfo,
  MergeSourceStatus,
  MergeSpec,
  MergeStrategy,
  NewTask,
  Project,
  Snapshot,
  Task,
} from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Field, SectionLabel } from "./AgentConfigForm";
import { ErrorNote } from "./Field";
import { Icon } from "./Icons";
import { shortId } from "../lib/format";

const STRATEGIES: { id: MergeStrategy; label: string; hint: string }[] = [
  { id: "merge", label: "Merge", hint: "one merge commit per source branch" },
  { id: "octopus", label: "Octopus", hint: "combine every source in a single merge" },
  { id: "rebase", label: "Rebase", hint: "replay the current branch onto each source" },
];

/** Creates one `merge` task that joins the selected branches in the tree. */
export function MergeModal({
  project,
  tasks,
  onClose,
  onCreated,
}: {
  project: Project;
  tasks: Task[];
  onClose: () => void;
  onCreated: (s: Snapshot) => void;
}) {
  const branchy = tasks.filter((t) => t.branch);
  const [selected, setSelected] = useState<string[]>([]);
  const [extra, setExtra] = useState("");
  const [target, setTarget] = useState("");
  const [strategy, setStrategy] = useState<MergeStrategy>("merge");
  const [testCommand, setTestCommand] = useState("");
  const [fixOnFailure, setFixOnFailure] = useState(true);
  const [pushTarget, setPushTarget] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [commitSources, setCommitSources] = useState(true);
  const [sourcesStatus, setSourcesStatus] = useState<MergeSourceStatus[]>([]);
  const [preflightError, setPreflightError] = useState<string | null>(null);

  // The landing branch is explicit: the resolved default branch is preselected,
  // but the user always sees and confirms exactly where the combine lands.
  const [defaultBranch, setDefaultBranch] = useState<string | null>(null);
  const [branches, setBranches] = useState<BranchInfo[]>([]);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      api.projectDefaultBranch(project.path),
      api.projectBranches(project.path).catch(() => [] as BranchInfo[]),
    ])
      .then(([def, list]) => {
        if (cancelled) return;
        setDefaultBranch(def);
        setBranches(list);
        setTarget((cur) => cur || def);
      })
      .catch(() => {
        /* target falls back to the default branch at run time */
      });
    return () => {
      cancelled = true;
    };
  }, [project.path]);

  const extraBranches = extra
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  const sources = [...selected, ...extraBranches];
  const skills = project.skills ?? [];
  const dirty = sourcesStatus.filter((s) => s.dirty);

  // Ask the backend which chosen source worktrees have uncommitted work. Those
  // changes are not on their branches, so a combine would leave them out unless
  // we commit first. Debounced because the "other branches" field changes per
  // keystroke.
  const sourceKey = sources.join("\u0000");
  useEffect(() => {
    if (sources.length === 0) {
      setSourcesStatus([]);
      setPreflightError(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      api
        .mergePreflight(project.path, sources)
        .then((next) => {
          if (cancelled) return;
          setSourcesStatus(next);
          setPreflightError(null);
        })
        .catch((e) => {
          if (!cancelled) setPreflightError(String(e));
        });
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
    // `sources` is derived from `sourceKey`; re-running on the joined key avoids
    // a fetch on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceKey, project.path]);

  async function submit() {
    setError(null);
    if (sources.length === 0) {
      setError("Pick at least one branch or finished task to combine.");
      return;
    }
    setBusy(true);
    try {
      if (commitSources) {
        // Re-check at submit time: the selection may have changed since the
        // debounced preflight, and a stale set could commit the wrong worktrees.
        const fresh = await api.mergePreflight(project.path, sources);
        const dirtyNow = fresh.filter((s) => s.dirty);
        if (dirtyNow.length > 0) {
          await api.commitWorktrees(
            dirtyNow
              .map((s) => s.worktree)
              .filter((w): w is string => typeof w === "string"),
          );
        }
      }
      const spec: MergeSpec = {
        sources,
        target: target.trim() || null,
        strategy,
        test_command: testCommand.trim() || null,
        fix_on_failure: fixOnFailure,
        push_target: pushTarget,
        commit_sources: commitSources,
      };
      const task: NewTask = {
        title: `Combine ${sources.length} branch${sources.length === 1 ? "" : "es"}`,
        prompt: `Combine ${sources.join(", ")}`,
        kind: "merge",
        isolation: "shared",
        merge: spec,
        after: selected,
      };
      onCreated(await api.createTasks(project.path, [task], "shared"));
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Combine branches"
      subtitle="Merge worktrees/branches, resolve conflicts, test, and land the result."
      onClose={onClose}
      width="max-w-xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={submit} disabled={busy}>
            <Icon name="diff" className="h-3.5 w-3.5" />
            {busy
              ? "Creating…"
              : commitSources && dirty.length > 0
                ? "Commit & combine"
                : "Create combine task"}
          </button>
        </>
      }
    >
      <div className="space-y-5">
        <section className="space-y-2">
          <SectionLabel icon="branch">Source branches</SectionLabel>
          {branchy.length === 0 ? (
            <p className="text-[11px] text-ink-subtle">
              No tasks have a branch yet. Run some isolated tasks first, or add a
              branch name below.
            </p>
          ) : (
            <div className="space-y-1.5">
              {branchy.map((t) => (
                <label
                  key={t.id}
                  className="flex items-center gap-2 rounded-lg border border-line bg-well px-3 py-2 text-[12px] text-ink-muted"
                >
                  <input
                    type="checkbox"
                    checked={selected.includes(t.id)}
                    onChange={(e) =>
                      setSelected((cur) =>
                        e.target.checked
                          ? [...cur, t.id]
                          : cur.filter((x) => x !== t.id),
                      )
                    }
                  />
                  <span className="truncate font-medium text-ink">{t.title}</span>
                  <span className="mono ml-auto shrink-0 text-[10.5px] text-ink-subtle">
                    {t.branch} · {shortId(t.id)}
                  </span>
                </label>
              ))}
            </div>
          )}
          <Field label="Other branches" hint="Comma-separated branch names.">
            <input
              className="input mono !text-[11.5px]"
              placeholder="feature/a, feature/b"
              value={extra}
              onChange={(e) => setExtra(e.target.value)}
            />
          </Field>
        </section>

        {dirty.length > 0 && (
          <section className="space-y-2 rounded-lg border border-warning-line bg-warning-soft p-3">
            <div className="flex items-start gap-2">
              <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
              <div className="min-w-0 space-y-1 text-[11.5px] text-ink-muted">
                <div className="font-medium text-warning">
                  {dirty.length} source worktree
                  {dirty.length === 1 ? "" : "s"} have uncommitted work
                </div>
                <ul className="space-y-0.5">
                  {dirty.map((s) => (
                    <li key={s.source} className="mono truncate text-[10.5px]">
                      {s.worktree ?? s.source} · {s.changed} file
                      {s.changed === 1 ? "" : "s"}
                    </li>
                  ))}
                </ul>
                <p>
                  Only committed work on a source branch is combined, so these
                  changes would be left out. Commit them first, or they stay
                  uncommitted.
                </p>
              </div>
            </div>
            <label className="flex items-center gap-2 text-[11.5px] text-ink-muted">
              <input
                type="checkbox"
                checked={commitSources}
                onChange={(e) => setCommitSources(e.target.checked)}
              />
              Stage and commit this work before combining
            </label>
          </section>
        )}
        {preflightError && (
          <p className="text-[11px] text-danger">{preflightError}</p>
        )}

        <div className="grid grid-cols-2 gap-3">
          <Field
            label="Land on"
            hint={
              defaultBranch
                ? `Default branch: ${defaultBranch}.`
                : "The repository's default branch."
            }
          >
            <select
              className="select mono !text-[11.5px]"
              value={target}
              onChange={(e) => setTarget(e.target.value)}
            >
              {target &&
                !branches.some((b) => !b.is_remote && b.name === target) && (
                  <option value={target}>{target}</option>
                )}
              {branches
                .filter((b) => !b.is_remote)
                .map((b) => (
                  <option key={b.name} value={b.name}>
                    {b.name}
                    {b.is_default ? " (default)" : ""}
                    {b.is_current ? " (current)" : ""}
                  </option>
                ))}
            </select>
          </Field>
          <Field label="Strategy">
            <select
              className="select"
              value={strategy}
              onChange={(e) => setStrategy(e.target.value as MergeStrategy)}
            >
              {STRATEGIES.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <p className="-mt-2 text-[11px] text-ink-subtle">
          {STRATEGIES.find((s) => s.id === strategy)?.hint}
        </p>

        <section className="space-y-2">
          <SectionLabel icon="check">After a clean merge</SectionLabel>
          {skills.length > 0 && (
            <Field label="Use a project skill as the test command">
              <select
                className="select"
                value=""
                onChange={(e) => e.target.value && setTestCommand(e.target.value)}
              >
                <option value="">Choose a skill…</option>
                {skills.map((s) => (
                  <option key={s.id} value={s.command}>
                    {s.name} — {s.command}
                  </option>
                ))}
              </select>
            </Field>
          )}
          <Field label="Test command" hint="Run after the branches are combined.">
            <input
              className="input mono !text-[11.5px]"
              placeholder="e.g. pnpm test"
              value={testCommand}
              onChange={(e) => setTestCommand(e.target.value)}
            />
          </Field>
          <label className="flex items-center gap-2 text-[12px] text-ink-muted">
            <input
              type="checkbox"
              checked={fixOnFailure}
              onChange={(e) => setFixOnFailure(e.target.checked)}
            />
            If tests fail, let an agent fix them and re-run
          </label>
        </section>

        <label className="flex items-center gap-2 text-[12px] text-ink-muted">
          <input
            type="checkbox"
            checked={pushTarget}
            onChange={(e) => setPushTarget(e.target.checked)}
          />
          Push the target branch when it lands
        </label>

        <p className="rounded-lg border border-line bg-well p-3 text-[11.5px] leading-relaxed text-ink-muted">
          Conflicts follow this project's <span className="text-ink">Git</span>{" "}
          setting. The combine task waits for every selected task to succeed, then
          runs on its own.
        </p>

        <p className="rounded-lg border border-accent-line bg-accent-soft p-3 text-[11.5px] leading-relaxed text-ink-muted">
          This will merge{" "}
          <span className="text-ink">
            {sources.length > 0 ? sources.join(", ") : "the selected sources"}
          </span>{" "}
          into{" "}
          <span className="mono text-ink">
            {target || defaultBranch || "the default branch"}
          </span>
          {pushTarget ? " and push it." : "."}
        </p>

        <ErrorNote error={error} />
      </div>
    </Modal>
  );
}
