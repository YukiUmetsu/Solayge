import { useCallback, useState } from "react";
import type { BranchInfo, Project, Worktree } from "../types";
import { api } from "../api";
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
            </li>
          );
        })}
      </ul>

      <ResultNotes note={note} fail={fail} />
    </div>
  );
}
