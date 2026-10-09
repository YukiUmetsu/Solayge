import { describe, expect, it } from "vitest";
import type { ApprovalSummary, SessionSummary } from "@solayge/shared-types";

import { filterApprovals, groupSessions, isExpired } from "./selectors";

function session(partial: Partial<SessionSummary> & { id: string }): SessionSummary {
  return {
    id: partial.id,
    project_id: partial.project_id ?? "/repo/app",
    title: partial.title ?? partial.id,
    agent: partial.agent ?? "opencode",
    status: partial.status ?? "running",
    pending_decisions: partial.pending_decisions ?? 0,
    started_at: partial.started_at ?? 0,
    updated_at: partial.updated_at ?? 0,
  };
}

describe("session grouping", () => {
  it("orders needs-input, then active, then recent, newest first within a group", () => {
    const sessions = [
      session({ id: "recent", status: "completed", updated_at: 50 }),
      session({ id: "active", status: "running", updated_at: 40 }),
      session({ id: "needOld", status: "waiting", pending_decisions: 1, updated_at: 10 }),
      session({ id: "needNew", status: "running", pending_decisions: 2, updated_at: 30 }),
    ];
    const grouped = groupSessions(sessions);
    expect(grouped.needsInput.map((s) => s.id)).toEqual(["needNew", "needOld"]);
    expect(grouped.active.map((s) => s.id)).toEqual(["active"]);
    expect(grouped.recent.map((s) => s.id)).toEqual(["recent"]);
  });

  it("groups drafts separately and first", () => {
    const sessions = [
      session({ id: "draft", status: "draft", updated_at: 5 }),
      session({ id: "run", status: "running", updated_at: 10 }),
    ];
    const grouped = groupSessions(sessions);
    expect(grouped.drafts.map((s) => s.id)).toEqual(["draft"]);
    expect(grouped.active.map((s) => s.id)).toEqual(["run"]);
  });
});

describe("approval filtering", () => {
  const approval = (partial: Partial<ApprovalSummary> & { id: string }): ApprovalSummary => ({
    id: partial.id,
    session_id: partial.session_id ?? "ses_1",
    project_id: partial.project_id ?? "/repo/app",
    kind: partial.kind ?? "permission",
    title: partial.title ?? "title",
    message: partial.message ?? null,
    command: partial.command ?? null,
    risk: partial.risk ?? "low",
    read_only: partial.read_only ?? true,
    requested_at: partial.requested_at ?? 0,
    expires_at: partial.expires_at ?? null,
  });

  it("separates permission decisions from questions", () => {
    const approvals = [
      approval({ id: "p1", kind: "permission", requested_at: 1 }),
      approval({ id: "q1", kind: "question", requested_at: 2 }),
      approval({ id: "p2", kind: "permission", requested_at: 3 }),
    ];
    expect(filterApprovals(approvals, "decision").map((a) => a.id)).toEqual(["p2", "p1"]);
    expect(filterApprovals(approvals, "questions").map((a) => a.id)).toEqual(["q1"]);
  });

  it("detects expiry", () => {
    expect(isExpired(approval({ id: "a", expires_at: 100 }), 100)).toBe(true);
    expect(isExpired(approval({ id: "b", expires_at: 200 }), 100)).toBe(false);
    expect(isExpired(approval({ id: "c", expires_at: null }), 100)).toBe(false);
  });
});
