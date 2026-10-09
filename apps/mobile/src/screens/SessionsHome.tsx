import { useMemo } from "react";
import type { SessionSummary } from "@solayge/shared-types";

import { useClientContext } from "../client/ClientContext";
import { groupSessions } from "../client/selectors";
import { EmptyState } from "../components/EmptyState";
import { ErrorState } from "../components/ErrorState";
import { SessionRow } from "../components/SessionRow";

function SessionGroup({
  label,
  sessions,
  projectName,
  onOpenSession,
}: {
  label: string;
  sessions: SessionSummary[];
  projectName: (id: string) => string;
  onOpenSession: (id: string) => void;
}) {
  if (sessions.length === 0) return null;
  return (
    <section className="group" aria-label={label}>
      <div className="group__head">
        <h2 className="group__title">{label}</h2>
        <span className="group__count" aria-label={`${sessions.length} sessions`}>
          {sessions.length}
        </span>
      </div>
      <ul className="session-list">
        {sessions.map((session) => (
          <SessionRow
            key={session.id}
            session={session}
            projectName={projectName(session.project_id)}
            onOpen={onOpenSession}
          />
        ))}
      </ul>
    </section>
  );
}

/**
 * Sessions home: needs-input, then active, then recent (mobile spec
 * section 7.2 B).
 */
export function SessionsHome({ onOpenSession }: { onOpenSession: (id: string) => void }) {
  const { data, loading, error, refresh } = useClientContext();

  const projectName = useMemo(() => {
    const names = new Map(data.projects.map((project) => [project.id, project.name]));
    return (id: string) => names.get(id) ?? id;
  }, [data.projects]);

  const grouped = useMemo(() => groupSessions(data.sessions), [data.sessions]);

  if (loading && data.sessions.length === 0 && !error) {
    return (
      <p className="loading" role="status">
        Loading sessions…
      </p>
    );
  }

  if (error && data.sessions.length === 0) {
    return <ErrorState message={error} onRetry={() => void refresh()} />;
  }

  if (data.sessions.length === 0) {
    return (
      <EmptyState
        icon="≣"
        title="No sessions yet"
        body="When your desktop runs a task, it appears here with its project, agent, and status."
      />
    );
  }

  return (
    <div className="sessions-home">
      <h1 className="sr-only">Sessions</h1>
      <SessionGroup
        label="Needs your input"
        sessions={grouped.needsInput}
        projectName={projectName}
        onOpenSession={onOpenSession}
      />
      <SessionGroup
        label="Running"
        sessions={grouped.active}
        projectName={projectName}
        onOpenSession={onOpenSession}
      />
      <SessionGroup
        label="Recent"
        sessions={grouped.recent}
        projectName={projectName}
        onOpenSession={onOpenSession}
      />
    </div>
  );
}
