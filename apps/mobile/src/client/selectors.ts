import type { ApprovalSummary, SessionSummary } from "@solayge/shared-types";

import { needsInput as sessionNeedsInput } from "./status";

export type InboxFilter = "decision" | "questions";

export interface GroupedSessions {
  needsInput: SessionSummary[];
  active: SessionSummary[];
  recent: SessionSummary[];
}

export function isActiveStatus(status: string): boolean {
  return status === "running" || status === "starting" || status === "paused";
}

/**
 * Order the sessions home: needs-input first, then active, then recent; each
 * group sorted by most recent activity (mobile spec section 7.2 B).
 */
export function groupSessions(sessions: SessionSummary[]): GroupedSessions {
  const sorted = [...sessions].sort((a, b) => b.updated_at - a.updated_at);
  const needsInput: SessionSummary[] = [];
  const active: SessionSummary[] = [];
  const recent: SessionSummary[] = [];
  for (const session of sorted) {
    if (sessionNeedsInput(session.status, session.pending_decisions)) {
      needsInput.push(session);
    } else if (isActiveStatus(session.status)) {
      active.push(session);
    } else {
      recent.push(session);
    }
  }
  return { needsInput, active, recent };
}

/** The Inbox filter: permission requests vs agent questions. */
export function filterApprovals(
  approvals: ApprovalSummary[],
  filter: InboxFilter,
): ApprovalSummary[] {
  const kind = filter === "decision" ? "permission" : "question";
  return approvals
    .filter((approval) => approval.kind === kind)
    .sort((a, b) => b.requested_at - a.requested_at);
}

/** Whether an approval has passed its expiry (or is already expired). */
export function isExpired(approval: ApprovalSummary, now: number): boolean {
  return approval.expires_at != null && approval.expires_at <= now;
}
