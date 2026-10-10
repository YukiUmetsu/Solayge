import { beforeEach, describe, expect, it } from "vitest";
import type { ProjectSummary, SessionSummary } from "@solayge/shared-types";

import {
  applyRetention,
  clampRetention,
  clearCachedData,
  createLocalStorageCache,
  DEFAULT_RETENTION_DAYS,
  loadRetention,
  retentionFor,
  setProjectRetention,
  type CacheEntry,
} from "./cache";

const DAY = 24 * 60 * 60;

function project(id: string): ProjectSummary {
  return { id, name: id, active_sessions: 0, pending_approvals: 0, updated_at: 0 };
}

function session(id: string, projectId: string): SessionSummary {
  return {
    id,
    project_id: projectId,
    title: id,
    agent: "opencode",
    status: "running",
    pending_decisions: 0,
    started_at: 0,
    updated_at: 0,
  };
}

beforeEach(() => {
  clearCachedData();
  localStorage.removeItem("solayge.task-cache.retention.v1");
});

describe("task data cache", () => {
  it("round-trips a snapshot for a desktop", () => {
    const cache = createLocalStorageCache();
    cache.write("desk_1", { projects: [project("/p")], sessions: [session("s1", "/p")], approvals: [] }, 100);

    const entry = cache.read("desk_1", 100);
    expect(entry?.savedAt).toBe(100);
    expect(entry?.sessions.map((s) => s.id)).toEqual(["s1"]);
    expect(cache.read("desk_other", 100)).toBeNull();
  });

  it("drops data past the default one-week window", () => {
    const cache = createLocalStorageCache();
    cache.write("desk_1", { projects: [], sessions: [session("s1", "/p")], approvals: [] }, 0);

    expect(cache.read("desk_1", 6 * DAY)?.sessions).toHaveLength(1);
    expect(cache.read("desk_1", 8 * DAY)?.sessions).toHaveLength(0);
  });

  it("honors a per-project retention window", () => {
    const cache = createLocalStorageCache();
    setProjectRetention("/keep", 30);
    setProjectRetention("/drop", 1);
    cache.write(
      "desk_1",
      {
        projects: [project("/keep"), project("/drop")],
        sessions: [session("keep", "/keep"), session("drop", "/drop")],
        approvals: [],
      },
      0,
    );

    const entry = cache.read("desk_1", 5 * DAY);
    expect(entry?.sessions.map((s) => s.id)).toEqual(["keep"]);
  });

  it("clamps and defaults retention values", () => {
    expect(clampRetention(-5)).toBe(0);
    expect(clampRetention(99999)).toBe(3650);
    expect(clampRetention(Number.NaN)).toBe(DEFAULT_RETENTION_DAYS);
    expect(retentionFor("/unknown", {})).toBe(DEFAULT_RETENTION_DAYS);
    setProjectRetention("/keep", 30);
    expect(loadRetention()["/keep"]).toBe(30);
  });

  it("never returns a snapshot with a bad shape", () => {
    localStorage.setItem("solayge.task-cache.v1.desk_1", JSON.stringify({ nope: true }));
    expect(createLocalStorageCache().read("desk_1", 0)).toBeNull();
  });

  it("clears cached data", () => {
    const cache = createLocalStorageCache();
    cache.write("desk_1", { projects: [], sessions: [session("s1", "/p")], approvals: [] }, 1);
    clearCachedData("desk_1");
    expect(cache.read("desk_1", 1)).toBeNull();
  });
});

describe("applyRetention", () => {
  it("keeps projects but filters sessions and their approvals", () => {
    const entry: CacheEntry = {
      savedAt: 0,
      projects: [project("/p")],
      sessions: [session("s1", "/p")],
      approvals: [
        {
          id: "a1",
          session_id: "s1",
          project_id: "/p",
          kind: "permission",
          title: "t",
          message: null,
          command: null,
          risk: "low",
          read_only: false,
          requested_at: 0,
          expires_at: null,
          fields: [],
        },
      ],
    };
    const kept = applyRetention(entry, DAY, { "/p": 7 });
    expect(kept.approvals).toHaveLength(1);
    const expired = applyRetention(entry, 8 * DAY, { "/p": 7 });
    expect(expired.sessions).toHaveLength(0);
    expect(expired.approvals).toHaveLength(0);
    expect(expired.projects).toHaveLength(1);
  });
});
