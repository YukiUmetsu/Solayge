/**
 * Pure projections from protocol values to user-facing status. Kept free of
 * React and transport so both screens and tests share one source of truth.
 *
 * A core rule from the mobile spec: never communicate state with color alone,
 * and never mark a message "accepted" merely because a packet was sent.
 */
import type { CommandStatusName, SessionEvent } from "@solayge/shared-types";

/** The transport/application connection state (mobile spec section 5.2). */
export type ConnectionStatus = "connecting" | "online" | "offline" | "revoked" | "error";

export interface StatusView {
  label: string;
  /** The icon carries as much meaning as the label. */
  icon: string;
  tone: "neutral" | "progress" | "success" | "warning" | "danger";
}

/**
 * Message-send status. `created` is local-only, `delivered` means the desktop
 * received the frame, `accepted` means the host handed it to the agent.
 * `unknown` is deliberately *not* `failed`: the outcome is unresolved, so the
 * UI must query rather than blind-retry.
 */
export function sendStatusView(status: CommandStatusName): StatusView {
  switch (status) {
    case "created":
      return { label: "Sending", icon: "◌", tone: "progress" };
    case "delivered":
      return { label: "Received by desktop", icon: "→", tone: "progress" };
    case "accepted":
      return { label: "Accepted by agent", icon: "✓", tone: "progress" };
    case "running":
      return { label: "Running", icon: "◌", tone: "progress" };
    case "succeeded":
      return { label: "Completed", icon: "✓", tone: "success" };
    case "failed":
      return { label: "Failed", icon: "!", tone: "danger" };
    case "canceled":
      return { label: "Canceled", icon: "⊘", tone: "neutral" };
    case "unknown":
      return { label: "Delivery unknown", icon: "?", tone: "warning" };
    default:
      return { label: "Unknown", icon: "?", tone: "warning" };
  }
}

/**
 * Whether a command may be safely retried.
 *
 * Only a command that is still local (`created`, never handed to the
 * transport) or was rejected *before dispatch* with a definite protocol error
 * is known not to have executed. A transport/`unknown` outcome must never be
 * retried blindly (mobile spec sections 3.2 and 6.1).
 */
export function canRetryCommand(status: CommandStatusName, errorCode?: string | null): boolean {
  if (status === "created") return true;
  if (errorCode && isDefiniteRejection(errorCode)) return true;
  return false;
}

/** Protocol error codes that prove the command was rejected before dispatch. */
export function isDefiniteRejection(code: string): boolean {
  switch (code) {
    case "invalid_frame":
    case "unsupported_version":
    case "unknown_action":
    case "missing_field":
    case "invalid_payload":
    case "frame_too_large":
    case "payload_too_large":
    case "idempotency_conflict":
    case "invalid_transition":
    case "unsupported_action":
    case "not_found":
    case "not_authorized":
    case "resource_exhausted":
      return true;
    default:
      return false;
  }
}

/** Connection state as a labeled, icon-bearing status. */
export function connectionStatusView(status: ConnectionStatus): StatusView {
  switch (status) {
    case "connecting":
      return { label: "Connecting", icon: "◌", tone: "progress" };
    case "online":
      return { label: "Connected", icon: "●", tone: "success" };
    case "offline":
      return { label: "Offline", icon: "⚠", tone: "warning" };
    case "revoked":
      return { label: "Access revoked", icon: "⊘", tone: "danger" };
    case "error":
      return { label: "Connection error", icon: "!", tone: "danger" };
  }
}

/** Human agent-provider names for the compact session rows. */
export function agentLabel(agent: string): string {
  const names: Record<string, string> = {
    opencode: "OpenCode",
    claude: "Claude Code",
    "claude-code": "Claude Code",
    codex: "Codex",
  };
  return names[agent.toLowerCase()] ?? agent;
}

/**
 * Whether a session is waiting on the user. The host is authoritative and
 * encodes this as a positive pending decision count.
 */
export function needsInput(status: string, pendingDecisions: number): boolean {
  if (pendingDecisions > 0) return true;
  return status === "waiting" || status === "blocked" || status === "needs_input";
}

/** A labeled status for a session, preferring the explicit waiting state. */
export function sessionStatusView(
  status: string,
  pendingDecisions: number,
): StatusView {
  if (pendingDecisions > 0) {
    return { label: "Waiting for your answer", icon: "◔", tone: "warning" };
  }
  switch (status) {
    case "running":
      return { label: "Running", icon: "◌", tone: "progress" };
    case "completed":
    case "succeeded":
      return { label: "Completed", icon: "✓", tone: "success" };
    case "failed":
      return { label: "Failed", icon: "!", tone: "danger" };
    case "canceled":
      return { label: "Canceled", icon: "⊘", tone: "neutral" };
    case "waiting":
      return { label: "Waiting for your answer", icon: "◔", tone: "warning" };
    case "blocked":
      return { label: "Blocked", icon: "⚠", tone: "warning" };
    default:
      return { label: status ? status[0].toUpperCase() + status.slice(1) : "Idle", icon: "•", tone: "neutral" };
  }
}

export interface EventSummary {
  icon: string;
  label: string;
  detail: string | null;
  /** Whether this event is a message (transcript) rather than a step. */
  isMessage: boolean;
  role: "agent" | "user" | "system";
}

function str(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/**
 * Project one protocol event into something a person can read. Agent output is
 * treated as untrusted data: it is only ever rendered as text.
 */
export function summarizeEvent(event: SessionEvent): EventSummary {
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  switch (event.kind) {
    case "message": {
      const role = payload.role === "user" ? "user" : payload.role === "system" ? "system" : "agent";
      return {
        icon: role === "user" ? "→" : role === "system" ? "•" : "←",
        label: str(payload.text) ?? "(empty message)",
        detail: null,
        isMessage: true,
        role,
      };
    }
    case "tool": {
      const tool = str(payload.tool) ?? "tool";
      const path = str(payload.path);
      const command = str(payload.command);
      return {
        icon: "✓",
        label: toolVerb(tool),
        detail: command ? `$ ${command}` : path,
        isMessage: false,
        role: "agent",
      };
    }
    case "status":
      return {
        icon: "•",
        label: str(payload.detail) ?? str(payload.status) ?? event.kind,
        detail: null,
        isMessage: false,
        role: "system",
      };
    default:
      return {
        icon: "•",
        label: event.kind,
        detail: str(payload.text),
        isMessage: false,
        role: "system",
      };
  }
}

function toolVerb(tool: string): string {
  const verbs: Record<string, string> = {
    read: "Read",
    edit: "Edited",
    write: "Wrote",
    bash: "Ran",
    shell: "Ran",
    grep: "Searched",
    test: "Running tests",
    fetch: "Fetched",
  };
  return verbs[tool.toLowerCase()] ?? tool;
}

/** The current step: the most recent tool/status event, if any. */
export function currentStep(events: SessionEvent[]): EventSummary | null {
  for (let i = events.length - 1; i >= 0; i -= 1) {
    if (events[i].kind !== "message") return summarizeEvent(events[i]);
  }
  return null;
}
