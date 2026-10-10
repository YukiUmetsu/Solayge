import { useMemo } from "react";

import { useClientContext, useNow } from "../client/ClientContext";
import { relativeTime } from "../client/format";
import { EmptyState } from "../components/EmptyState";
import { ErrorState } from "../components/ErrorState";

/**
 * Project list: name, running/blocked/pending counts, and recent activity
 * (mobile spec section 7.2 E).
 */
export function Projects({ onOpenProject }: { onOpenProject: (id: string) => void }) {
  const { data, loading, error, refresh } = useClientContext();
  const now = useNow();

  const runningCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const session of data.sessions) {
      if (session.status === "running") {
        counts.set(session.project_id, (counts.get(session.project_id) ?? 0) + 1);
      }
    }
    return counts;
  }, [data.sessions]);

  if (loading && data.projects.length === 0 && !error) {
    return (
      <p className="loading" role="status">
        Loading projects…
      </p>
    );
  }

  if (error && data.projects.length === 0) {
    return <ErrorState message={error} onRetry={() => void refresh()} />;
  }

  if (data.projects.length === 0) {
    return (
      <EmptyState
        icon="▦"
        title="No projects yet"
        body="Projects managed by your desktop appear here with their tasks and changed files."
      />
    );
  }

  return (
    <div className="projects">
      <h1 className="sr-only">Projects</h1>
      <ul className="project-list">
        {data.projects.map((project) => (
          <li key={project.id}>
            <button className="project-row" type="button" onClick={() => onOpenProject(project.id)}>
              <span className="project-row__name">{project.name}</span>
              <span className="project-row__path mono">{project.id}</span>
              <span className="project-row__meta">
                <span>{runningCounts.get(project.id) ?? 0} running</span>
                {project.pending_approvals > 0 ? (
                  <span className="project-row__pending">{project.pending_approvals} need input</span>
                ) : null}
                <span>{relativeTime(project.updated_at, now())}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
