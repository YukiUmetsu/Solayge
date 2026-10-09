import { useCallback, useEffect, useState } from "react";
import type { BranchInfo, CommitFile, Project, Worktree, WorktreeDetail } from "../types";
import { api } from "../api";
import { relTime } from "../lib/format";
import { Icon } from "./Icons";

/**
 * Git management panels: every branch with its landing state against the
 * default branch, and every worktree. Both surface "not merged" clearly and
 * offer to land (local merge) or delete, so work that only lives on a
 * `devtools/*` branch cannot quietly succeed and be forgotten. The default
 * branch is never deletable and its delete controls are not even rendered.
 */

/** A shared "run one git action, report, refresh" helper. */
function useGitAction(onChanged: () => void) {
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [fail, setFail] = useState<string | null>(null);

  const run = useCallback(
    async (
      key: string,
      fn: () => Promise<string>,
      confirmMessage?: string,
    ) => {
      if (confirmMessage && !window.confirm(confirmMessage)) return;
      setBusy(key);
      setNote(null);
      setFail(null);
      try {
        setNote((await fn()) || "Done.");
        onChanged();
      } catch (e) {
        setFail(String(e));
      } finally {
        setBusy(null);
      }
    },
    [onChanged],
  );

  return { busy, note, fail, run };
}

/** The "merged into <default>" indicator: icon + word, never colour alone. */
function LandedBadge({ merged, target }: { merged: boolean; target: string }) {
  return merged ? (
    <span
      className="flex shrink-0 items-center gap-1 rounded border border-line bg-well px-1.5 py-0.5 text-[10px] text-success"
      title={`Merged into ${target}`}
    >
      <Icon name="check" className="h-3 w-3" />
      Merged
    </span>
  ) : (
    <span
      className="flex shrink-0 items-center gap-1 rounded border border-warning-line bg-warning-soft px-1.5 py-0.5 text-[10px] text-warning"
      title={`Not merged into ${target}`}
    >
      <Icon name="branch" className="h-3 w-3" />
      Not merged
    </span>
  );
}

/**
 * Warns that a branch's worktree holds uncommitted changes. They are not on the
 * branch, so a "Merged" badge alone would be misleading: the edits would be
 * left behind by a combine or lost when the worktree is removed.
 */
function DirtyBadge() {
  return (
    <span
      className="flex shrink-0 items-center gap-1 rounded border border-warning-line bg-warning-soft px-1.5 py-0.5 text-[10px] text-warning"
      title="Uncommitted changes in the worktree — not on the branch, and not included in a merge"
    >
      <Icon name="alert" className="h-3 w-3" />
      Uncommitted
    </span>
  );
}

function ResultNotes({
  note,
  fail,
}: {
  note: string | null;
  fail: string | null;
}) {
  return (
    <>
      {note && <p className="text-[11px] text-success">{note}</p>}
      {fail && <p className="text-[11px] text-danger">{fail}</p>}
    </>
  );
}

function plural(n: number, one: string, many = `${one}s`): string {
  return n === 1 ? one : many;
}

function sortBranches(a: BranchInfo, b: BranchInfo): number {
  // Unmerged first (they need attention), then the default branch, then names.
  if (a.merged !== b.merged) return a.merged ? 1 : -1;
  if (a.is_default !== b.is_default) return a.is_default ? -1 : 1;
  return a.name.localeCompare(b.name);
}

export function GitBranches({
  project,
  branches,
  defaultBranch,
  loading,
  error,
  onChanged,
}: {
  project: Project;
  branches: BranchInfo[];
  defaultBranch: string | null;
  loading: boolean;
  error: string | null;
  onChanged: () => void;
}) {
  const { busy, note, fail, run } = useGitAction(onChanged);
  const target = defaultBranch ?? "the default branch";
  const unmerged = branches.filter((b) => !b.merged && !b.is_default).length;
  const ordered = [...branches].sort(sortBranches);

  // Everything safe to bulk-delete: merged, local, not the default, not the
  // checked-out branch, and not held by a worktree.
  const deletableMerged = ordered.filter(
    (b) =>
      !b.is_remote && b.merged && !b.is_default && !b.is_current && !b.worktree,
  );

  const deleteMerged = () => {
    if (deletableMerged.length === 0) return;
    void run(
      "bulk-branches",
      async () => {
        const failed: string[] = [];
        for (const b of deletableMerged) {
          try {
            await api.gitDeleteBranch(project.path, b.name);
          } catch (e) {
            failed.push(`${b.name}: ${e}`);
          }
        }
        const done = deletableMerged.length - failed.length;
        if (failed.length > 0) {
          throw new Error(
            `Deleted ${done}/${deletableMerged.length}. Failed: ${failed.join("; ")}`,
          );
        }
        return `Deleted ${done} merged ${plural(done, "branch", "branches")}.`;
      },
      `Delete ${deletableMerged.length} merged local ${plural(
        deletableMerged.length,
        "branch",
        "branches",
      )}? The default branch is never deleted.`,
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
        <span className="text-ink-muted">
          {branches.length} {plural(branches.length, "branch", "branches")}
        </span>
        {unmerged > 0 ? (
          <span className="text-warning">
            {unmerged} not merged into <span className="mono">{target}</span>
          </span>
        ) : (
          <span className="text-success">All merged into {target}</span>
        )}
        {deletableMerged.length > 0 && (
          <button
            className="btn btn-ghost !px-2 !py-1 !text-[11px]"
            disabled={busy !== null}
            onClick={deleteMerged}
            title={`Delete all ${deletableMerged.length} merged local branches (the default branch is excluded)`}
          >
            <Icon name="trash" className="h-3 w-3" />
            Delete merged ({deletableMerged.length})
          </button>
        )}
      </div>

      {loading && <p className="text-xs text-ink-subtle">Loading…</p>}
      {error && <p className="text-xs text-danger">{error}</p>}
      {!loading && !error && branches.length === 0 && (
        <p className="text-xs text-ink-subtle">No branches found.</p>
      )}

      <ul className="space-y-1.5">
        {ordered.map((b) => {
          // Remote-only branches are merged by their tracking ref.
          const mergeRef = b.is_remote ? `origin/${b.name}` : b.name;
          const inUse = Boolean(b.worktree);
          const canDeleteLocal = !b.is_remote && !b.is_default && !b.is_current && !inUse;
          return (
            <li
              key={`${b.is_remote ? "r:" : "l:"}${b.name}`}
              className="rounded-lg border border-line px-3 py-2"
            >
              <div className="flex items-center gap-2">
                <span className="mono truncate text-[12px] text-ink">
                  {b.is_remote ? `origin/${b.name}` : b.name}
                </span>
                {b.is_default && (
                  <span className="shrink-0 rounded bg-well-strong px-1.5 py-0.5 text-[10px] text-ink-muted">
                    default
                  </span>
                )}
                {b.is_current && (
                  <span className="shrink-0 rounded bg-accent-soft px-1.5 py-0.5 text-[10px] text-accent-text">
                    current
                  </span>
                )}
                {b.is_remote && (
                  <span className="shrink-0 rounded bg-well-strong px-1.5 py-0.5 text-[10px] text-ink-muted">
                    remote
                  </span>
                )}
                <span className="ml-auto flex items-center gap-1.5">
                  {b.dirty && <DirtyBadge />}
                  <LandedBadge merged={b.merged} target={target} />
                </span>
              </div>

              <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10.5px] text-ink-subtle">
                {!b.merged && b.ahead > 0 && (
                  <span className="text-warning">{b.ahead} ahead</span>
                )}
                {b.behind > 0 && <span>{b.behind} behind</span>}
                {b.worktree && (
                  <span className="mono truncate" title={b.worktree}>
                    worktree: {b.worktree}
                  </span>
                )}
                {b.has_remote && <span>on origin</span>}
              </div>

              <div className="mt-2 flex flex-wrap gap-1.5">
                {!b.merged && !b.is_default && (
                  <button
                    className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                    disabled={busy !== null}
                    onClick={() =>
                      void run(`merge:${b.name}`, () =>
                        api.gitMergeBranch(project.path, mergeRef, defaultBranch),
                      )
                    }
                    title={`Merge ${mergeRef} into ${target}`}
                  >
                    <Icon name="diff" className="h-3 w-3" />
                    Merge into {target}
                  </button>
                )}
                {!b.is_default && !b.is_remote && (
                  <button
                    className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                    disabled={busy !== null || !canDeleteLocal}
                    onClick={() =>
                      void run(
                        `del:${b.name}`,
                        () => api.gitDeleteBranch(project.path, b.name),
                        `Delete local branch "${b.name}"? This can discard unmerged commits.`,
                      )
                    }
                    title={
                      b.is_current
                        ? "Checked out in the project folder"
                        : inUse
                          ? "Checked out in a worktree"
                          : "Delete this local branch"
                    }
                  >
                    <Icon name="trash" className="h-3 w-3" />
                    Delete
                  </button>
                )}
                {!b.is_default && b.has_remote && (
                  <button
                    className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                    disabled={busy !== null}
                    onClick={() =>
                      void run(
                        `delr:${b.name}`,
                        () => api.gitDeleteRemoteBranch(project.path, b.name),
                        `Delete remote branch "origin/${b.name}"?`,
                      )
                    }
                    title={`Delete origin/${b.name}`}
                  >
                    <Icon name="trash" className="h-3 w-3" />
                    Delete remote
                  </button>
                )}
                {!b.is_default && !b.is_remote && b.has_remote && (
                  <button
                    className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                    disabled={busy !== null || !canDeleteLocal}
                    onClick={() =>
                      void run(
                        `delboth:${b.name}`,
                        async () => {
                          await api.gitDeleteBranch(project.path, b.name);
                          await api.gitDeleteRemoteBranch(project.path, b.name);
                          return `Deleted ${b.name} locally and on origin.`;
                        },
                        `Delete branch "${b.name}" locally AND on origin? This can discard unmerged commits.`,
                      )
                    }
                    title={`Delete ${b.name} locally and origin/${b.name}`}
                  >
                    <Icon name="trash" className="h-3 w-3" />
                    Delete local + remote
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>

      <ResultNotes note={note} fail={fail} />
    </div>
  );
}

/** Small inline add/delete change counts, e.g. `+12 −3`. */
function ChangeCounts({ additions, deletions }: { additions: number; deletions: number }) {
  if (!additions && !deletions) return null;
  return (
    <span className="shrink-0 font-mono text-[10px]">
      {additions > 0 && <span className="text-success">+{additions}</span>}
      {additions > 0 && deletions > 0 && " "}
      {deletions > 0 && <span className="text-danger">−{deletions}</span>}
    </span>
  );
}

/**
 * The progressive-disclosure detail for one worktree: commit/file counts and
 * last commit at a glance, then its commits (each expandable to the files it
 * touched) and a safe "undo" that reverts a commit. Loaded only when expanded,
 * so the default Worktrees list stays cheap.
 */
function WorktreeDetails({
  project,
  worktree,
  onChanged,
}: {
  project: Project;
  worktree: Worktree;
  onChanged: () => void;
}) {
  const [detail, setDetail] = useState<WorktreeDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Kept separate from `error` (which replaces the whole panel): a failed
  // revert must not hide the commit list the user is working from.
  const [actionError, setActionError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [openCommit, setOpenCommit] = useState<string | null>(null);
  const [files, setFiles] = useState<Record<string, CommitFile[]>>({});
  const [filesError, setFilesError] = useState<Record<string, string>>({});
  const [reverting, setReverting] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setDetail(await api.worktreeDetail(project.path, worktree.path, 50));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [project.path, worktree.path]);

  useEffect(() => {
    void load();
  }, [load]);

  const toggleCommit = async (commitId: string) => {
    if (openCommit === commitId) {
      setOpenCommit(null);
      return;
    }
    setOpenCommit(commitId);
    if (files[commitId]) return;
    try {
      const list = await api.worktreeCommitFiles(worktree.path, commitId);
      setFiles((prev) => ({ ...prev, [commitId]: list }));
    } catch (e) {
      setFilesError((prev) => ({ ...prev, [commitId]: String(e) }));
    }
  };

  const revert = async (commitId: string, subject: string) => {
    if (
      !window.confirm(
        `Undo commit "${subject}"?\n\nThis reverts it with a new commit that inverts its changes — history and other commits are preserved.`,
      )
    ) {
      return;
    }
    setReverting(commitId);
    setNote(null);
    setActionError(null);
    try {
      setNote(await api.gitRevertCommit(worktree.path, commitId));
      setOpenCommit(null);
      await load();
      onChanged();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setReverting(null);
    }
  };

  if (loading) return <p className="px-3 py-2 text-[11px] text-ink-subtle">Loading details…</p>;
  if (error) return <p className="px-3 py-2 text-[11px] text-danger">{error}</p>;
  if (!detail) return null;

  return (
    <div className="mt-2 space-y-2 rounded-lg bg-well px-3 py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-ink-muted">
        <span title={`Commits on ${detail.base}..HEAD that this worktree introduces`}>
          {detail.commit_count} {plural(detail.commit_count, "commit")}
        </span>
        <span>
          {detail.files_changed} {plural(detail.files_changed, "file")} changed
        </span>
        <ChangeCounts additions={detail.additions} deletions={detail.deletions} />
        {detail.last_commit_at != null && (
          <span title={detail.last_commit_subject ?? undefined}>
            last commit {relTime(detail.last_commit_at)}
          </span>
        )}
        {detail.dirty && <span className="text-warning">uncommitted changes</span>}
      </div>

      {detail.commits.length === 0 ? (
        <p className="text-[11px] text-ink-subtle">
          No commits ahead of <span className="mono">{detail.base}</span>.
        </p>
      ) : (
        <ul className="space-y-1">
          {detail.commits.map((commit) => {
            const open = openCommit === commit.id;
            return (
              <li key={commit.id} className="rounded border border-line bg-panel">
                <div className="flex items-center gap-2 px-2 py-1">
                  <button
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    onClick={() => void toggleCommit(commit.id)}
                    aria-expanded={open}
                    title="Show files changed by this commit"
                  >
                    <Icon
                      name={open ? "collapse" : "expand"}
                      className="h-3 w-3 shrink-0 text-ink-subtle"
                    />
                    <span className="mono shrink-0 text-[10px] text-ink-subtle">
                      {commit.short_id}
                    </span>
                    <span className="truncate text-[11.5px] text-ink">{commit.subject}</span>
                  </button>
                  <span className="shrink-0 text-[10px] text-ink-subtle">
                    {commit.files_changed} {plural(commit.files_changed, "file")}
                  </span>
                  <ChangeCounts additions={commit.additions} deletions={commit.deletions} />
                  <span className="shrink-0 text-[10px] text-ink-subtle">
                    {relTime(commit.committed_at)}
                  </span>
                  <button
                    className="btn btn-ghost !px-1.5 !py-0.5 !text-[10px]"
                    disabled={reverting !== null}
                    onClick={() => void revert(commit.id, commit.subject)}
                    title={`Undo (revert) commit ${commit.short_id}`}
                  >
                    <Icon name="retry" className="h-3 w-3" />
                    Undo
                  </button>
                </div>

                {open && (
                  <div className="border-t border-line px-2 py-1.5">
                    {filesError[commit.id] ? (
                      <p className="text-[10.5px] text-danger">{filesError[commit.id]}</p>
                    ) : !files[commit.id] ? (
                      <p className="text-[10.5px] text-ink-subtle">Loading files…</p>
                    ) : files[commit.id].length === 0 ? (
                      <p className="text-[10.5px] text-ink-subtle">No file changes.</p>
                    ) : (
                      <ul className="space-y-0.5">
                        {files[commit.id].map((file) => (
                          <li key={file.path} className="flex items-center gap-2 text-[10.5px]">
                            <span className="mono truncate text-ink">{file.path}</span>
                            <span className="shrink-0 text-ink-subtle">{file.status}</span>
                            <span className="ml-auto shrink-0">
                              <ChangeCounts
                                additions={file.additions}
                                deletions={file.deletions}
                              />
                            </span>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {actionError && <p className="text-[10.5px] text-danger">{actionError}</p>}
      {note && <p className="text-[10.5px] text-success">{note}</p>}
    </div>
  );
}

export function GitWorktrees({
  project,
  branches,
  defaultBranch,
  worktrees,
  loading,
  error,
  onReload,
  onChanged,
}: {
  project: Project;
  branches: BranchInfo[];
  defaultBranch: string | null;
  worktrees: Worktree[];
  loading: boolean;
  error: string | null;
  onReload: () => void;
  onChanged: () => void;
}) {
  const { busy, note, fail, run } = useGitAction(onChanged);
  const target = defaultBranch ?? "the default branch";

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggleExpanded = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const refresh = useCallback(() => {
    onReload();
    onChanged();
  }, [onReload, onChanged]);

  const byBranch = new Map(branches.map((b) => [b.name, b]));
  const mergedWorktrees = worktrees.filter((w) => {
    if (w.is_main || !w.branch) return false;
    const info = byBranch.get(w.branch);
    // Never bulk-remove a worktree with uncommitted changes: the removal would
    // be refused anyway, and the leftover work needs the user's eyes first.
    return Boolean(info?.merged && !info.is_default && !info.dirty);
  });

  const removeMerged = () => {
    if (mergedWorktrees.length === 0) return;
    void run(
      "bulk-worktrees",
      async () => {
        const failed: string[] = [];
        for (const w of mergedWorktrees) {
          try {
            await api.gitRemoveWorktree(project.path, w.path);
            if (w.branch) await api.gitDeleteBranch(project.path, w.branch);
          } catch (e) {
            failed.push(`${w.branch ?? w.path}: ${e}`);
          }
        }
        const done = mergedWorktrees.length - failed.length;
        if (failed.length > 0) {
          throw new Error(
            `Removed ${done}/${mergedWorktrees.length}. Failed: ${failed.join("; ")}`,
          );
        }
        return `Removed ${done} merged ${plural(done, "worktree")} and their branches.`;
      },
      `Remove ${mergedWorktrees.length} merged ${plural(
        mergedWorktrees.length,
        "worktree",
      )} and delete their merged branches?`,
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className="text-[11px] text-ink-muted">
          {worktrees.length} {plural(worktrees.length, "worktree")}
        </span>
        {mergedWorktrees.length > 0 && (
          <button
            className="btn btn-ghost !px-2 !py-1 !text-[11px]"
            disabled={busy !== null}
            onClick={removeMerged}
            title={`Remove all ${mergedWorktrees.length} merged worktrees and delete their branches`}
          >
            <Icon name="trash" className="h-3 w-3" />
            Remove merged ({mergedWorktrees.length})
          </button>
        )}
        <button
          className="btn btn-ghost !ml-auto !px-2 !py-1 text-[11px]"
          onClick={refresh}
          title="Refresh worktrees"
        >
          <Icon name="refresh" className="h-3 w-3" />
          Refresh
        </button>
      </div>

      {loading && <p className="text-xs text-ink-subtle">Loading…</p>}
      {error && <p className="text-xs text-danger">{error}</p>}
      {!loading && !error && worktrees.length === 0 && (
        <p className="text-xs text-ink-subtle">No worktrees found.</p>
      )}

      <ul className="space-y-1.5">
        {worktrees.map((w) => {
          const info = w.branch ? byBranch.get(w.branch) : undefined;
          const merged = info ? info.merged : false;
          // Never offer to delete the default branch, even if a worktree holds it.
          const canDeleteBranch = Boolean(w.branch) && merged && !info?.is_default;
          return (
            <li key={w.path} className="rounded-lg border border-line px-3 py-2">
              <div className="flex items-center gap-2">
                <span className="mono truncate text-[12px] text-ink" title={w.path}>
                  {w.branch ?? "(detached)"}
                </span>
                <button
                  className="btn btn-ghost !shrink-0 !px-1.5 !py-0.5 !text-[10px]"
                  onClick={() => toggleExpanded(w.path)}
                  aria-expanded={expanded.has(w.path)}
                  title="Show commits and files for this worktree"
                >
                  <Icon
                    name={expanded.has(w.path) ? "collapse" : "expand"}
                    className="h-3 w-3"
                  />
                  Details
                </button>
                {w.is_main && (
                  <span className="shrink-0 rounded bg-well-strong px-1.5 py-0.5 text-[10px] text-ink-muted">
                    main worktree
                  </span>
                )}
                <span className="ml-auto flex items-center gap-1.5">
                  {info?.dirty && <DirtyBadge />}
                  {!w.is_main && <LandedBadge merged={merged} target={target} />}
                </span>
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10.5px] text-ink-subtle">
                <span className="mono truncate" title={w.path}>
                  {w.path}
                </span>
                {w.head && <span className="mono">{w.head.slice(0, 8)}</span>}
                {!w.is_main && info && !info.merged && info.ahead > 0 && (
                  <span className="text-warning">{info.ahead} ahead</span>
                )}
              </div>

              {!w.is_main && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {w.branch && info && !info.merged && (
                    <button
                      className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                      disabled={busy !== null}
                      onClick={() =>
                        void run(`merge:${w.branch}`, () =>
                          api.gitMergeBranch(project.path, w.branch!, defaultBranch),
                        )
                      }
                      title={`Merge ${w.branch} into ${target}`}
                    >
                      <Icon name="diff" className="h-3 w-3" />
                      Merge into {target}
                    </button>
                  )}
                  <button
                    className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                    disabled={busy !== null}
                    onClick={() =>
                      void run(
                        `rmwt:${w.path}`,
                        () => api.gitRemoveWorktree(project.path, w.path),
                        `Remove worktree "${w.path}"? Its branch is kept.`,
                      )
                    }
                    title="Remove this worktree (keeps the branch)"
                  >
                    <Icon name="trash" className="h-3 w-3" />
                    Remove worktree
                  </button>
                  {w.branch && (
                    <button
                      className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                      disabled={busy !== null || !canDeleteBranch}
                      onClick={() =>
                        void run(
                          `del:${w.branch}`,
                          () => api.gitDeleteBranch(project.path, w.branch!),
                          `Delete local branch "${w.branch}"? This can discard unmerged commits.`,
                        )
                      }
                      title={
                        canDeleteBranch
                          ? `Delete local branch ${w.branch}`
                          : "Remove the worktree before deleting its branch"
                      }
                    >
                      <Icon name="trash" className="h-3 w-3" />
                      Delete branch
                    </button>
                  )}
                  {w.branch && info?.has_remote && (
                    <button
                      className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                      disabled={busy !== null}
                      onClick={() =>
                        void run(
                          `delr:${w.branch}`,
                          () => api.gitDeleteRemoteBranch(project.path, w.branch!),
                          `Delete remote branch "origin/${w.branch}"?`,
                        )
                      }
                      title={`Delete origin/${w.branch}`}
                    >
                      <Icon name="trash" className="h-3 w-3" />
                      Delete remote
                    </button>
                  )}
                  {w.branch && info?.has_remote && canDeleteBranch && (
                    <button
                      className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                      disabled={busy !== null}
                      onClick={() =>
                        void run(
                          `delboth:${w.branch}`,
                          async () => {
                            await api.gitRemoveWorktree(project.path, w.path);
                            await api.gitDeleteBranch(project.path, w.branch!);
                            await api.gitDeleteRemoteBranch(project.path, w.branch!);
                            return `Removed worktree and deleted ${w.branch} locally and on origin.`;
                          },
                          `Remove the worktree and delete "${w.branch}" locally AND on origin?`,
                        )
                      }
                      title={`Remove worktree and delete ${w.branch} locally and on origin`}
                    >
                      <Icon name="trash" className="h-3 w-3" />
                      Delete local + remote
                    </button>
                  )}
                </div>
              )}

              {expanded.has(w.path) && (
                <WorktreeDetails project={project} worktree={w} onChanged={refresh} />
              )}
            </li>
          );
        })}
      </ul>

      <ResultNotes note={note} fail={fail} />
    </div>
  );
}
