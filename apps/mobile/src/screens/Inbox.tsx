import { useMemo } from "react";
import type { ApprovalSummary } from "@solayge/shared-types";

import { useClientContext, useNow } from "../client/ClientContext";
import { relativeTime } from "../client/format";
import { filterApprovals, isExpired, type InboxFilter } from "../client/selectors";
import { agentLabel } from "../client/status";
import { EmptyState } from "../components/EmptyState";

function count(approvals: ApprovalSummary[], filter: InboxFilter): number {
  return filterApprovals(approvals, filter).length;
}

function ApprovalRow({
  approval,
  agent,
  onOpen,
}: {
  approval: ApprovalSummary;
  agent: string;
  onOpen: (id: string) => void;
}) {
  const now = useNow();
  const expired = isExpired(approval, now());
  const isQuestion = approval.kind === "question";
  return (
    <li>
      <button className="inbox-row" type="button" onClick={() => onOpen(approval.id)}>
        <span className="inbox-row__kind">
          <span aria-hidden="true">{isQuestion ? "?" : "⚠"} </span>
          {isQuestion ? "Question" : "Permission request"}
        </span>
        <span className="inbox-row__title">{approval.title}</span>
        <span className="inbox-row__meta">
          <span>{agentLabel(agent)}</span>
          <span>
            {isQuestion
              ? `Asked ${relativeTime(approval.requested_at, now())}`
              : `Requested ${relativeTime(approval.requested_at, now())}`}
          </span>
          {expired ? <span className="inbox-row__expired">Expired</span> : null}
        </span>
        {approval.command ? (
          <code className="inbox-row__command">{approval.command}</code>
        ) : null}
        <span className="inbox-row__action">
          {isQuestion ? "Answer question" : "Review request"}
        </span>
      </button>
    </li>
  );
}

/**
 * Inbox: pending decisions and agent questions, one tap from anywhere
 * (mobile spec section 7.2 D).
 */
export function Inbox({
  filter,
  onFilter,
  onOpenApproval,
}: {
  filter: InboxFilter;
  onFilter: (filter: InboxFilter) => void;
  onOpenApproval: (id: string) => void;
}) {
  const { data, status } = useClientContext();
  const offline = status === "offline" || status === "revoked" || status === "error";
  const agents = useMemo(() => {
    const bySession = new Map(data.sessions.map((session) => [session.id, session.agent]));
    return (sessionId: string) => bySession.get(sessionId) ?? "agent";
  }, [data.sessions]);

  const approvals = useMemo(
    () => filterApprovals(data.approvals, filter),
    [data.approvals, filter],
  );

  return (
    <div className="inbox">
      <h1 className="sr-only">Inbox</h1>
      <div className="segmented" role="group" aria-label="Inbox filters">
        <button
          type="button"
          className={`segmented__item${filter === "decision" ? " segmented__item--active" : ""}`}
          aria-pressed={filter === "decision"}
          onClick={() => onFilter("decision")}
        >
          Needs decision
          <span className="segmented__count" aria-label={`${count(data.approvals, "decision")} items`}>
            {count(data.approvals, "decision")}
          </span>
        </button>
        <button
          type="button"
          className={`segmented__item${filter === "questions" ? " segmented__item--active" : ""}`}
          aria-pressed={filter === "questions"}
          onClick={() => onFilter("questions")}
        >
          Questions
          <span className="segmented__count" aria-label={`${count(data.approvals, "questions")} items`}>
            {count(data.approvals, "questions")}
          </span>
        </button>
      </div>

      {offline && data.approvals.length === 0 ? (
        <EmptyState
          icon="⚠"
          title="Inbox unavailable offline"
          body="Reconnect to the desktop to see pending decisions and questions."
        />
      ) : approvals.length === 0 ? (
        <EmptyState
          icon={filter === "decision" ? "✓" : "?"}
          title={filter === "decision" ? "Nothing needs a decision" : "No open questions"}
          body={
            filter === "decision"
              ? "Permission requests from your agents will appear here for review."
              : "When an agent needs an answer, it will show up here."
          }
        />
      ) : (
        <ul className="inbox-list">
          {approvals.map((approval) => (
            <ApprovalRow
              key={approval.id}
              approval={approval}
              agent={agents(approval.session_id)}
              onOpen={onOpenApproval}
            />
          ))}
        </ul>
      )}
    </div>
  );
}
