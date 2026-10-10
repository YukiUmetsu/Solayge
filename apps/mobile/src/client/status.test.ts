import { describe, expect, it } from "vitest";
import type { SessionEvent } from "@solayge/shared-types";

import {
  agentLabel,
  canRetryCommand,
  connectionStatusView,
  currentStep,
  isDefiniteRejection,
  needsInput,
  sendStatusView,
  sessionStatusView,
  summarizeEvent,
} from "./status";
import { formatChange, formatExpiry, relativeTime } from "./format";

describe("message send status", () => {
  it("maps the four required delivery stages and keeps unknown distinct from failed", () => {
    expect(sendStatusView("created").label).toBe("Sending");
    expect(sendStatusView("delivered").label).toBe("Received by desktop");
    expect(sendStatusView("accepted").label).toBe("Accepted by agent");
    expect(sendStatusView("running").label).toBe("Running");
    expect(sendStatusView("succeeded").label).toBe("Completed");
    expect(sendStatusView("failed").label).toBe("Failed");
    expect(sendStatusView("unknown").label).toBe("Delivery unknown");
    expect(sendStatusView("unknown").tone).not.toBe(sendStatusView("failed").tone);
  });

  it("offers retry only when the command is known not to have executed", () => {
    expect(canRetryCommand("created")).toBe(true);
    expect(canRetryCommand("delivered")).toBe(false);
    expect(canRetryCommand("running")).toBe(false);
    expect(canRetryCommand("succeeded")).toBe(false);
    // A transport timeout is ambiguous: never blind-retry.
    expect(canRetryCommand("unknown", "transport")).toBe(false);
    // A definite pre-dispatch rejection is safe to retry.
    expect(canRetryCommand("failed", "invalid_payload")).toBe(true);
    expect(canRetryCommand("failed", "not_found")).toBe(true);
  });

  it("classifies protocol error codes", () => {
    expect(isDefiniteRejection("not_authorized")).toBe(true);
    expect(isDefiniteRejection("transport")).toBe(false);
    expect(isDefiniteRejection("internal")).toBe(false);
  });
});

describe("connection and session status", () => {
  it("labels connection state with an icon and never only a color", () => {
    for (const status of ["connecting", "online", "offline", "revoked", "error"] as const) {
      const view = connectionStatusView(status);
      expect(view.label.length).toBeGreaterThan(0);
      expect(view.icon.length).toBeGreaterThan(0);
    }
    expect(connectionStatusView("online").label).toBe("Connected");
    expect(connectionStatusView("revoked").label).toBe("Access revoked");
  });

  it("treats a pending decision as needing input", () => {
    expect(needsInput("running", 1)).toBe(true);
    expect(needsInput("waiting", 0)).toBe(true);
    expect(needsInput("running", 0)).toBe(false);
    expect(sessionStatusView("running", 1).label).toBe("Waiting for your answer");
  });

  it("uses friendly agent names", () => {
    expect(agentLabel("opencode")).toBe("OpenCode");
    expect(agentLabel("Codex")).toBe("Codex");
  });
});

describe("event projection", () => {
  it("renders messages as transcript entries and tools as steps", () => {
    const message = summarizeEvent({
      seq: 1,
      event_id: "e1",
      kind: "message",
      timestamp: 1,
      payload: { role: "agent", text: "Hello" },
    });
    expect(message.isMessage).toBe(true);
    expect(message.label).toBe("Hello");

    const tool = summarizeEvent({
      seq: 2,
      event_id: "e2",
      kind: "tool",
      timestamp: 2,
      payload: { tool: "edit", path: "src/App.tsx" },
    });
    expect(tool.isMessage).toBe(false);
    expect(tool.label).toBe("Edited");
    expect(tool.detail).toBe("src/App.tsx");
  });

  it("finds the current step from the latest non-message event", () => {
    const events: SessionEvent[] = [
      { seq: 1, event_id: "e1", kind: "message", timestamp: 1, payload: { text: "hi" } },
      { seq: 2, event_id: "e2", kind: "status", timestamp: 2, payload: { detail: "Running tests" } },
    ];
    expect(currentStep(events)?.label).toBe("Running tests");
  });
});

describe("formatting", () => {
  it("labels relative time", () => {
    expect(relativeTime(1000, 1000)).toBe("just now");
    expect(relativeTime(1000, 1120)).toBe("2m ago");
    expect(relativeTime(1000, 1000 + 7200)).toBe("2h ago");
  });

  it("labels expiry and change counts", () => {
    expect(formatExpiry(1600, 1000)).toBe("in 10m");
    expect(formatExpiry(900, 1000)).toBe("expired");
    expect(formatExpiry(null, 1000)).toBeNull();
    expect(formatChange(10, 2)).toBe("+10 / −2");
    expect(formatChange(null, null)).toBe("no line counts");
  });
});
