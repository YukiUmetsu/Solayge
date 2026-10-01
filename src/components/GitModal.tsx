import { useCallback, useEffect, useState } from "react";
import type { BranchInfo, DiffResult, GitStatus, Project, Worktree } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { DiffBody, DiffStats } from "./DiffView";
import { GitBranches, GitWorktrees } from "./GitPanels";
import { fileStatusLabel } from "../lib/format";

type Tab = "local" | "branch" | "branches" | "worktrees" | "git";
type MergeMethod = "squash" | "merge" | "rebase";

const MERGE_METHODS: { value: MergeMethod; label: string }[] = [
  { value: "squash", label: "Squash" },
  { value: "merge", label: "Merge" },
  { value: "rebase", label: "Rebase" },
];

/** Class for one entry in the left tab rail. */
function tabClass(active: boolean): string {
  return `mb-1 flex w-full items-center rounded-lg px-3 py-2 text-left text-xs transition ${
    active ? "bg-well-strong text-ink" : "text-ink-muted hover:bg-well"
  }`;
}

/**
 * A wide, tabbed Git management view for a project: local working-tree changes,
 * committed work on the current branch, every branch and worktree with its
 * landing state (merged into the default branch or not), and a Git panel to
 * stage, commit, push, ship, and refresh.
 */
export function GitModal({
  project,
  onClose,
}: {
  project: Project;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<Tab>("local");
  const [base, setBase] = useState<string | null>(null);
  const [current, setCurrent] = useState<string | null>(null);

  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [branchesLoading, setBranchesLoading] = useState(true);
  const [branchesError, setBranchesError] = useState<string | null>(null);

  const [worktrees, setWorktrees] = useState<Worktree[]>([]);
  const [worktreesLoading, setWorktreesLoading] = useState(true);
  const [worktreesError, setWorktreesError] = useState<string | null>(null);

  const [local, setLocal] = useState<DiffResult | null>(null);
  const [localLoading, setLocalLoading] = useState(true);
  const [localError, setLocalError] = useState<string | null>(null);

  const [includeLocal, setIncludeLocal] = useState(false);
  const [branch, setBranch] = useState<DiffResult | null>(null);
  const [branchLoading, setBranchLoading] = useState(true);
  const [branchError, setBranchError] = useState<string | null>(null);

  // ---- Git tab ----
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(true);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [commitMessage, setCommitMessage] = useState("");
  const [prTitle, setPrTitle] = useState(`Ship: ${project.name}`);
  const [mergeMethod, setMergeMethod] = useState<MergeMethod>("squash");
  const [gitBusy, setGitBusy] = useState(false);
  const [gitMessage, setGitMessage] = useState<string | null>(null);
  const [gitError, setGitError] = useState<string | null>(null);

  const loadLabels = useCallback(async () => {
    try {
      const [defaultBranch, repoStatus] = await Promise.all([
        api.projectDefaultBranch(project.path),
        api.projectStatus(project.path),
      ]);
      setBase(defaultBranch);
      setCurrent(repoStatus.branch ?? null);
    } catch {
      // Labels are cosmetic; each view surfaces its own error.
    }
  }, [project.path]);

  const loadLocal = useCallback(async () => {
    setLocalLoading(true);
    setLocalError(null);
    try {
      setLocal(await api.gitDiff(project.path));
    } catch (e) {
      setLocalError(String(e));
    } finally {
      setLocalLoading(false);
    }
  }, [project.path]);

  const loadBranch = useCallback(
    async (withLocal: boolean) => {
      setBranchLoading(true);
      setBranchError(null);
      try {
        setBranch(await api.branchDiff(project.path, withLocal));
      } catch (e) {
        setBranchError(String(e));
      } finally {
        setBranchLoading(false);
      }
    },
    [project.path],
  );

  const loadStatus = useCallback(async () => {
    setStatusLoading(true);
    setStatusError(null);
    try {
      const next = await api.projectStatus(project.path);
      setStatus(next);
      // Default every changed file to selected.
      setSelected(new Set(next.changed_files.map((f) => f.path)));
    } catch (e) {
      setStatusError(String(e));
    } finally {
      setStatusLoading(false);
    }
  }, [project.path]);

  const loadBranches = useCallback(async () => {
    setBranchesLoading(true);
    setBranchesError(null);
    try {
      setBranches(await api.projectBranches(project.path));
    } catch (e) {
      setBranchesError(String(e));
    } finally {
      setBranchesLoading(false);
    }
  }, [project.path]);

  const loadWorktrees = useCallback(async () => {
    setWorktreesLoading(true);
    setWorktreesError(null);
    try {
      setWorktrees(await api.projectWorktrees(project.path));
    } catch (e) {
      setWorktreesError(String(e));
    } finally {
      setWorktreesLoading(false);
    }
  }, [project.path]);

  useEffect(() => {
    void loadLabels();
    void loadLocal();
    void loadStatus();
    void loadBranches();
    void loadWorktrees();
  }, [loadLabels, loadLocal, loadStatus, loadBranches, loadWorktrees]);

  useEffect(() => {
    void loadBranch(includeLocal);
  }, [loadBranch, includeLocal]);

  const refreshAll = useCallback(() => {
    void loadLabels();
    void loadStatus();
    void loadLocal();
    void loadBranch(includeLocal);
    void loadBranches();
    void loadWorktrees();
  }, [
    loadLabels,
    loadStatus,
    loadLocal,
    loadBranch,
    includeLocal,
    loadBranches,
    loadWorktrees,
  ]);

  const refresh = useCallback(() => {
    if (tab === "git") {
      refreshAll();
      return;
    }
    void loadLabels();
    if (tab === "local") {
      void loadLocal();
    } else if (tab === "branch") {
      void loadBranch(includeLocal);
    } else if (tab === "worktrees") {
      void loadWorktrees();
    } else {
      void loadBranches();
    }
  }, [
    tab,
    refreshAll,
    loadLabels,
    loadLocal,
    loadBranch,
    includeLocal,
    loadBranches,
    loadWorktrees,
  ]);

  /** Run a Git action, reporting its message or error and refreshing on success. */
  const runGitAction = async (label: string, action: () => Promise<string>) => {
    setGitBusy(true);
    setGitMessage(null);
    setGitError(null);
    try {
      const result = await action();
      setGitMessage(result || `${label} succeeded.`);
      refreshAll();
    } catch (e) {
      setGitError(String(e));
    } finally {
      setGitBusy(false);
    }
  };

  const toggleSelected = (path: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const stageSelected = () => {
    const files = Array.from(selected);
    if (files.length === 0) {
      setGitMessage(null);
      setGitError("Select at least one file to stage.");
      return;
    }
    void runGitAction("Stage selected", () => api.gitStage(project.path, files));
  };

  const stageAll = () =>
    void runGitAction("Stage all", () => api.gitStage(project.path));

  const commit = () =>
    void runGitAction("Commit", async () => {
      const result = await api.gitCommit(project.path, commitMessage);
      setCommitMessage("");
      return result;
    });

  const branchLabel =
    base && current ? `${base} vs ${current}` : "main vs current branch";
  const loading = tab === "local" ? localLoading : branchLoading;
  const error = tab === "local" ? localError : branchError;
  const result = tab === "local" ? local : branch;
  const changed = status?.changed_files ?? [];

  return (
    <Modal
      title="Git"
      subtitle={project.name}
      onClose={onClose}
      width="max-w-6xl"
      footer={
        <button className="btn btn-ghost" onClick={onClose}>
          Close
        </button>
      }
    >
      <div className="flex h-[70vh]">
        <nav className="w-52 shrink-0 border-r border-line pr-3">
          <button
            className={tabClass(tab === "local")}
            onClick={() => setTab("local")}
          >
            Local changes
          </button>
          <button
            className={tabClass(tab === "branch")}
            onClick={() => setTab("branch")}
          >
            {branchLabel}
          </button>
          <button
            className={tabClass(tab === "branches")}
            onClick={() => setTab("branches")}
          >
            Branches
            {branches.some((b) => !b.merged && !b.is_default) && (
              <span
                className="ml-auto h-1.5 w-1.5 rounded-full bg-warning"
                title="Some branches are not merged into the default branch"
              />
            )}
          </button>
          <button
            className={tabClass(tab === "worktrees")}
            onClick={() => setTab("worktrees")}
          >
            Worktrees
          </button>
          <button
            className={tabClass(tab === "git")}
            onClick={() => setTab("git")}
          >
            Changes
          </button>
        </nav>

        <div className="min-w-0 flex-1 overflow-y-auto pl-4">
          <div className="mb-3 flex items-start justify-between gap-3">
            <span className="text-[11px] text-ink-subtle">
              {tab === "local" && (
                "Committed and uncommitted changes in the working tree."
              )}
              {tab === "branch" && (
                <>
                  Committed work on{" "}
                  <span className="mono">{current ?? "the current branch"}</span>{" "}
                  since it diverged from{" "}
                  <span className="mono">{base ?? "the base branch"}</span>.
                </>
              )}
              {tab === "branches" && (
                <>
                  Every branch with its landing state against{" "}
                  <span className="mono">{base ?? "the default branch"}</span>.
                  Land unmerged work or remove stale branches.
                </>
              )}
              {tab === "worktrees" && (
                <>
                  Linked worktrees and the branch each holds. Remove one when its
                  work is landed.
                </>
              )}
              {tab === "git" && (
                <>
                  Stage, commit, push, and ship changes against{" "}
                  <span className="mono">{base ?? "the default branch"}</span>.
                </>
              )}
            </span>
            <button
              className="btn btn-ghost !px-2 !py-1"
              onClick={refresh}
              title="Refresh"
            >
              <Icon name="refresh" className="h-3 w-3" />
            </button>
          </div>

          {tab === "branch" && (
            <label className="mb-3 flex items-center gap-2 text-[11px] text-ink-muted">
              <input
                type="checkbox"
                checked={includeLocal}
                onChange={(e) => setIncludeLocal(e.target.checked)}
              />
              Include local changes
            </label>
          )}

          {(tab === "local" || tab === "branch") && (
            <>
              {loading && <p className="text-xs text-ink-subtle">Loading…</p>}
              {error && <p className="text-xs text-danger">{error}</p>}
              {!loading && !error && result && (
                <>
                  <DiffStats files={result.files} />
                  <DiffBody
                    result={result}
                    empty={
                      tab === "local"
                        ? "No local changes."
                        : "No committed changes vs the base branch."
                    }
                  />
                </>
              )}
            </>
          )}

          {tab === "branches" && (
            <GitBranches
              project={project}
              branches={branches}
              defaultBranch={base}
              loading={branchesLoading}
              error={branchesError}
              onChanged={refreshAll}
            />
          )}

          {tab === "worktrees" && (
            <GitWorktrees
              project={project}
              branches={branches}
              defaultBranch={base}
              worktrees={worktrees}
              loading={worktreesLoading}
              error={worktreesError}
              onReload={loadWorktrees}
              onChanged={refreshAll}
            />
          )}

          {tab === "git" && (
            <div className="space-y-5">
              <section>
                <div className="mb-2 flex items-center justify-between gap-2">
                  <h3 className="text-xs font-medium text-ink">
                    Changed files
                    {changed.length > 0 && (
                      <span className="ml-2 text-[11px] font-normal text-ink-subtle">
                        {changed.length}
                      </span>
                    )}
                  </h3>
                  <div className="flex gap-2">
                    <button
                      className="btn btn-ghost !px-2 !py-1 text-[11px]"
                      disabled={gitBusy || changed.length === 0}
                      onClick={stageAll}
                    >
                      Stage all
                    </button>
                    <button
                      className="btn btn-ghost !px-2 !py-1 text-[11px]"
                      disabled={gitBusy || selected.size === 0}
                      onClick={stageSelected}
                    >
                      Stage selected
                    </button>
                  </div>
                </div>
                {statusLoading && (
                  <p className="text-xs text-ink-subtle">Loading…</p>
                )}
                {statusError && (
                  <p className="text-xs text-danger">{statusError}</p>
                )}
                {!statusLoading && !statusError && changed.length === 0 && (
                  <p className="text-xs text-ink-subtle">Working tree clean.</p>
                )}
                <ul className="space-y-1">
                  {changed.map((f) => {
                    // Porcelain's first column is the index (staged) state;
                    // `??` means untracked, which is not staged.
                    const staged = f.status[0] !== " " && f.status[0] !== "?";
                    return (
                      <li
                        key={f.path}
                        className="flex items-center gap-2 rounded-lg border border-line px-2 py-1 text-[11.5px]"
                      >
                        <input
                          type="checkbox"
                          checked={selected.has(f.path)}
                          onChange={() => toggleSelected(f.path)}
                          disabled={gitBusy}
                        />
                        <span className="mono truncate text-ink">{f.path}</span>
                        {staged && (
                          <span className="shrink-0 rounded bg-well-strong px-1.5 py-0.5 text-[10px] text-success">
                            staged
                          </span>
                        )}
                        <span className="mono ml-auto shrink-0 text-[10px] text-ink-subtle">
                          {fileStatusLabel(f.status)}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              </section>

              <section className="space-y-2">
                <h3 className="text-xs font-medium text-ink">Commit</h3>
                <div className="flex gap-2">
                  <input
                    className="input flex-1"
                    placeholder="Commit message (default: chore: update)"
                    value={commitMessage}
                    onChange={(e) => setCommitMessage(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commit();
                    }}
                    disabled={gitBusy}
                  />
                  <button className="btn btn-primary" disabled={gitBusy} onClick={commit}>
                    Commit
                  </button>
                </div>
                <button
                  className="btn btn-ghost"
                  disabled={gitBusy}
                  onClick={() => void runGitAction("Push", () => api.gitPush(project.path))}
                >
                  Push
                </button>
              </section>

              <section className="space-y-2">
                <h3 className="text-xs font-medium text-ink">
                  Pull request →{" "}
                  <span className="mono">{base ?? "default branch"}</span>
                </h3>
                <div className="flex gap-2">
                  <input
                    className="input flex-1"
                    placeholder="PR title"
                    value={prTitle}
                    onChange={(e) => setPrTitle(e.target.value)}
                    disabled={gitBusy}
                  />
                  <button
                    className="btn btn-primary"
                    disabled={gitBusy}
                    onClick={() =>
                      void runGitAction("Create PR", () =>
                        api.gitCreatePr(project.path, prTitle),
                      )
                    }
                  >
                    Create PR
                  </button>
                </div>
                <div className="flex gap-2">
                  <select
                    className="select flex-1"
                    value={mergeMethod}
                    onChange={(e) => setMergeMethod(e.target.value as MergeMethod)}
                    disabled={gitBusy}
                  >
                    {MERGE_METHODS.map((m) => (
                      <option key={m.value} value={m.value}>
                        {m.label}
                      </option>
                    ))}
                  </select>
                  <button
                    className="btn btn-ghost"
                    disabled={gitBusy}
                    onClick={() =>
                      void runGitAction("Merge PR", () =>
                        api.gitMergePr(project.path, mergeMethod),
                      )
                    }
                  >
                    Merge PR
                  </button>
                </div>
              </section>

              <section>
                <button
                  className="btn btn-ghost"
                  disabled={gitBusy}
                  onClick={() =>
                    void runGitAction("Checkout & pull", () =>
                      api.gitCheckoutPull(project.path),
                    )
                  }
                >
                  Checkout &amp; pull default branch
                </button>
              </section>

              {gitMessage && (
                <p className="text-xs text-success">{gitMessage}</p>
              )}
              {gitError && <p className="text-xs text-danger">{gitError}</p>}
            </div>
          )}
        </div>
      </div>
    </Modal>
  );
}
