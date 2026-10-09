import type { SessionSummary } from "@solayge/shared-types";

import { useNow } from "../client/ClientContext";
import { relativeTime } from "../client/format";
import { agentLabel, sessionStatusView } from "../client/status";
import { StatusPill } from "./StatusPill";

/**
 * One session row: project, task, agent/provider, current state, last
 * meaningful activity, and pending count (mobile spec section 7.2 B).
 */
export function SessionRow({
  session,
  projectName,
  onOpen,
}: {
  session: SessionSummary;
  projectName: string;
  onOpen: (sessionId: string) => void;
}) {
  const now = useNow();
  const view = sessionStatusView(session.status, session.pending_decisions);
  const pending = session.pending_decisions;

  return (
    <li>
      <button className="session-row" type="button" onClick={() => onOpen(session.id)}>
        <span className="session-row__head">
          <span className="session-row__project">{projectName}</span>
          {pending > 0 ? (
            <span className="badge" aria-label={`${pending} pending`}>
              {pending}
            </span>
          ) : null}
        </span>
        <span className="session-row__title">{session.title}</span>
        <span className="session-row__meta">
          <span className="session-row__agent">{agentLabel(session.agent)}</span>
          <StatusPill view={view} size="sm" />
          <span className="session-row__activity">Last activity {relativeTime(session.updated_at, now())}</span>
        </span>
      </button>
    </li>
  );
}
