import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  askDeadline,
  clock,
  compactDuration,
  duration,
  fileStatusLabel,
  isLive,
  relTime,
  runDuration,
  runMinutes,
  shortId,
} from "./format";
import type { TaskAsk } from "../types";

const NOW = 1_700_000_000;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW * 1000);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("relTime", () => {
  it("returns a dash without a timestamp", () => {
    expect(relTime(null)).toBe("—");
    expect(relTime(0)).toBe("—");
  });

  it("formats past and future times", () => {
    expect(relTime(NOW - 3)).toBe("just now");
    expect(relTime(NOW - 30)).toBe("30s ago");
    expect(relTime(NOW - 120)).toBe("2m ago");
    expect(relTime(NOW - 7200)).toBe("2h ago");
    expect(relTime(NOW + 120)).toBe("2m from now");
  });
});

describe("duration helpers", () => {
  it("formats durations at each scale", () => {
    expect(duration(null)).toBe("—");
    expect(duration(NOW - 30, NOW)).toBe("30s");
    expect(duration(NOW - 90, NOW)).toBe("1m 30s");
    expect(duration(NOW - 3720, NOW)).toBe("1h 2m");
  });

  it("reports compact durations", () => {
    expect(compactDuration(30)).toBe("30s");
    expect(compactDuration(90)).toBe("1m");
    expect(compactDuration(3720)).toBe("1h 2m");
  });

  it("computes a live run duration", () => {
    expect(runDuration({ started_at: NOW - 65, finished_at: NOW }, NOW)).toBe("1m 5s");
    expect(runDuration({ started_at: NOW - 30 }, NOW)).toBe("30s");
    expect(runDuration({}, NOW)).toBe("—");
  });

  it("reports run minutes for task cards", () => {
    expect(runMinutes({ started_at: NOW - 30, finished_at: NOW }, NOW)).toBe("<1m");
    expect(runMinutes({ started_at: NOW - 120, finished_at: NOW }, NOW)).toBe("2m");
    expect(runMinutes({ started_at: NOW - 3600 * 2 }, NOW)).toBe("2h 0m");
  });

  it("detects a live task", () => {
    expect(isLive({ started_at: NOW - 5 })).toBe(true);
    expect(isLive({ started_at: NOW - 5, finished_at: NOW })).toBe(false);
    expect(isLive({})).toBe(false);
  });
});

describe("small helpers", () => {
  it("shortens ids", () => {
    expect(shortId("1234567890abcdef")).toBe("12345678");
  });

  it("reads an ask deadline, preferring an absolute one", () => {
    const base: TaskAsk = {
      id: "a",
      kind: "question",
      title: "t",
      resources: [],
      save: [],
      fields: [],
      options: [],
      deadline: NOW + 60,
      created_at: NOW,
      timeout_secs: 30,
    };
    expect(askDeadline(base)).toBe(NOW + 60);
    expect(askDeadline({ ...base, deadline: null })).toBe(NOW + 30);
    expect(askDeadline({ ...base, deadline: null, timeout_secs: null })).toBeNull();
  });

  it("renders git status letters", () => {
    expect(fileStatusLabel("??")).toBe("U");
    expect(fileStatusLabel(" M")).toBe("M");
    expect(fileStatusLabel("A")).toBe("A");
  });

  it("renders a clock stamp", () => {
    expect(clock(null)).toBe("—");
    expect(clock(NOW)).not.toBe("—");
  });
});
