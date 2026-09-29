import { useCallback, useEffect, useState } from "react";
import type { DiffResult, Project } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { DiffBody } from "./DiffView";

/** Diff of the current branch against the repository's default branch. */
export function BranchDiffModal({
  project,
  onClose,
}: {
  project: Project;
  onClose: () => void;
}) {
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [base, setBase] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [result, defaultBranch] = await Promise.all([
        api.branchDiff(project.path),
        api.projectDefaultBranch(project.path),
      ]);
      setDiff(result);
      setBase(defaultBranch);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [project.path]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Modal
      title="Local changes"
      subtitle={`Working tree vs ${base ?? "default branch"} — ${project.name}`}
      onClose={onClose}
      width="max-w-3xl"
      footer={
        <button className="btn btn-ghost" onClick={onClose}>
          Close
        </button>
      }
    >
      <div className="mb-3 flex items-center justify-between gap-3">
        <span className="text-[11px] text-ink-subtle">
          Committed and uncommitted changes in the working tree, relative to{" "}
          <span className="mono">{base ?? "the default branch"}</span>.
        </span>
        <button
          className="btn btn-ghost !px-2 !py-1"
          onClick={() => void load()}
          title="Refresh"
        >
          <Icon name="refresh" className="h-3 w-3" />
        </button>
      </div>
      {loading && <p className="text-xs text-ink-subtle">Loading…</p>}
      {error && <p className="text-xs text-danger">{error}</p>}
      {diff && <DiffBody result={diff} empty="No local changes vs the default branch." />}
    </Modal>
  );
}
