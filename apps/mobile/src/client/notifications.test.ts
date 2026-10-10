import { beforeEach, describe, expect, it } from "vitest";
import type { ApprovalSummary, SessionSummary } from "@solayge/shared-types";

import {
  DEFAULT_NOTIFICATION_SETTINGS,
  allows,
  diffNotifications,
  loadNotificationSettings,
  loadNotificationState,
  saveNotificationSettings,
  saveNotificationState,
} from "./notifications";

function session(id: string, status: string): SessionSummary {
  return {
    id,
    project_id: "/repo/app",
    title: id,
    agent: "opencode",
    status,
    pending_decisions: 0,
    started_at: 0,
    updated_at: 0,
  };
}

function approval(id: string, kind: "question" | "permission" = "question"): ApprovalSummary {
  return {
    id,
    session_id: "s1",
    project_id: "/repo/app",
    kind,
    title: `title ${id}`,
    message: null,
    command: null,
    risk: "low",
    read_only: kind === "question",
    requested_at: 0,
    expires_at: null,
    fields: [],
  };
}

beforeEach(() => {
  localStorage.clear();
});

describe("notification settings", () => {
  it("defaults to off and round-trips through storage", () => {
    expect(loadNotificationSettings()).toEqual(DEFAULT_NOTIFICATION_SETTINGS);
    saveNotificationSettings({ ...DEFAULT_NOTIFICATION_SETTINGS, enabled: true, volume: 0.3 });
    const loaded = loadNotificationSettings();
    expect(loaded.enabled).toBe(true);
    expect(loaded.volume).toBe(0.3);
  });

  it("clamps a bad stored volume back into range", () => {
    localStorage.setItem(
      "solayge.notifications.settings.v1",
      JSON.stringify({ enabled: true, volume: 9 }),
    );
    expect(loadNotificationSettings().volume).toBe(1);
  });
});

describe("diffNotifications", () => {
  it("records a first snapshot as a baseline with no events", () => {
    const { events, state } = diffNotifications(null, [session("s1", "running")], [approval("a1")]);
    expect(events).toEqual([]);
    expect(state.approvals).toEqual(["a1"]);
    expect(state.sessions.s1).toBe("running");
  });

  it("announces new approvals and status transitions", () => {
    const previous = {
      approvals: ["a1"],
      sessions: { s1: "running", s2: "running" },
    };
    const { events } = diffNotifications(
      previous,
      [session("s1", "succeeded"), session("s2", "failed")],
      [approval("a1"), approval("a2", "permission")],
    );
    expect(events).toEqual([
      { kind: "question", title: "Permission needed", body: "title a2" },
      { kind: "completed", title: "Task completed", body: "s1" },
      { kind: "failed", title: "Task failed", body: "s2" },
    ]);
  });

  it("does not re-announce unchanged work", () => {
    const previous = { approvals: ["a1"], sessions: { s1: "running" } };
    const { events } = diffNotifications(
      previous,
      [session("s1", "running")],
      [approval("a1")],
    );
    expect(events).toEqual([]);
  });
});

describe("allows", () => {
  it("respects the master switch and each category", () => {
    expect(allows(DEFAULT_NOTIFICATION_SETTINGS, "question")).toBe(false);
    const on = { ...DEFAULT_NOTIFICATION_SETTINGS, enabled: true };
    expect(allows(on, "question")).toBe(true);
    expect(allows({ ...on, onCompleted: false }, "completed")).toBe(false);
    expect(allows({ ...on, onFailed: false }, "failed")).toBe(false);
  });
});

describe("notification state", () => {
  it("round-trips per desktop and rejects a bad shape", () => {
    saveNotificationState("desk_1", { approvals: ["a"], sessions: { s: "running" } });
    expect(loadNotificationState("desk_1")?.approvals).toEqual(["a"]);
    expect(loadNotificationState("desk_other")).toBeNull();
    localStorage.setItem("solayge.notifications.state.v1.desk_2", JSON.stringify({ nope: true }));
    expect(loadNotificationState("desk_2")).toBeNull();
  });
});
