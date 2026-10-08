import { describe, expect, it } from "vitest";
import {
  canCancel,
  canEditTask,
  canMessageTask,
  canRun,
  displayStatus,
  IN_REVIEW_META,
  isClearable,
  isRetryable,
  isReviewInProgress,
  isTerminalStatus,
  statusMeta,
} from "./tasks";
import { makeTask } from "../test/fixtures";
import type { Task } from "../types";

describe("status predicates", () => {
  it("marks the finished statuses terminal", () => {
    for (const s of ["succeeded", "failed", "canceled", "blocked", "interrupted"] as const) {
      expect(isTerminalStatus(s)).toBe(true);
    }
    for (const s of ["draft", "waiting", "ready", "running"] as const) {
      expect(isTerminalStatus(s)).toBe(false);
    }
  });

  it("knows what can run, cancel, retry, and edit", () => {
    expect(canRun({ status: "draft" })).toBe(true);
    expect(canRun({ status: "running" })).toBe(false);
    expect(canCancel({ status: "running" })).toBe(true);
    expect(canCancel({ status: "draft" })).toBe(false);
    expect(isRetryable({ status: "failed" })).toBe(true);
    expect(isRetryable({ status: "succeeded" })).toBe(false);
    expect(canEditTask({ status: "draft" })).toBe(true);
    expect(canEditTask({ status: "ready" })).toBe(true);
    expect(canEditTask({ status: "running" })).toBe(false);
  });
});

describe("isClearable", () => {
  it("clears failed and canceled tasks", () => {
    expect(isClearable(makeTask({ status: "failed" }))).toBe(true);
    expect(isClearable(makeTask({ status: "canceled" }))).toBe(true);
  });

  it("keeps a success whose review has not settled", () => {
    const pending = makeTask({
      status: "succeeded",
      review: { mode: "report", status: "pending" },
    });
    expect(isClearable(pending)).toBe(false);

    const settled = makeTask({
      status: "succeeded",
      review: { mode: "report", status: "passed" },
    });
    expect(isClearable(settled)).toBe(true);
  });

  it("keeps paused and retryable tasks", () => {
    expect(isClearable(makeTask({ status: "blocked" }))).toBe(false);
    expect(isClearable(makeTask({ status: "interrupted" }))).toBe(false);
  });
});

describe("review display status", () => {
  it("shows in_review only for a success with a live review", () => {
    const live = makeTask({
      status: "succeeded",
      review: { mode: "report", status: "running" },
    });
    expect(isReviewInProgress(live)).toBe(true);
    expect(displayStatus(live)).toBe("in_review");
    expect(statusMeta(displayStatus(live))).toBe(IN_REVIEW_META);

    const off = makeTask({ status: "succeeded", review: { mode: "off", status: "pending" } });
    expect(displayStatus(off)).toBe("succeeded");
  });
});

describe("canMessageTask", () => {
  const tasks: Task[] = [];

  it("allows messaging a live task with a session", () => {
    const t = makeTask({ status: "running", session_id: "ses_1" });
    expect(canMessageTask(t, tasks)).toBe(true);
  });

  it("refuses without a session", () => {
    expect(canMessageTask(makeTask({ status: "running" }), tasks)).toBe(false);
  });

  it("refuses a finished task that has a dependent", () => {
    const t = makeTask({ id: "a", status: "succeeded", session_id: "ses_1" });
    const dependent = makeTask({ id: "b", depends_on: ["a"] });
    expect(canMessageTask(t, [t, dependent])).toBe(false);
  });

  it("refuses while a review is in progress", () => {
    const t = makeTask({
      status: "succeeded",
      session_id: "ses_1",
      review: { mode: "report", status: "running" },
    });
    expect(canMessageTask(t, [t])).toBe(false);
  });
});
