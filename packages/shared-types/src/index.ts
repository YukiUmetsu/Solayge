/**
 * `@solayge/shared-types` — TypeScript mirror of the `solayge-protocol` Rust
 * DTOs.
 *
 * These types are hand-maintained to match the Rust definitions. The shared
 * fixture in `fixtures/protocol-sample.json` is verified by both languages: the
 * Rust crate asserts it deserializes and re-serializes exactly, and the test in
 * this package asserts the same field names. Change the Rust DTOs and this file
 * together.
 */

// ---- version and limits (mirror solayge-protocol::envelope) ----

export const PROTOCOL_VERSION = 1 as const;
export const MAX_FRAME_BYTES = 256 * 1024;
export const MAX_PAYLOAD_BYTES = 192 * 1024;
export const MAX_ID_LEN = 128;

/** Any JSON value, mirroring `serde_json::Value`. */
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

// ---- enums, mirrored as string unions ----

export type MessageKind = "command" | "response" | "event" | "error";
export const MESSAGE_KINDS = ["command", "response", "event", "error"] as const;

export type CommandStatusName =
  | "created"
  | "delivered"
  | "accepted"
  | "running"
  | "succeeded"
  | "failed"
  | "canceled"
  | "unknown";

/** Lifecycle order; `unknown` is distinct from `failed`. */
export const COMMAND_STATUSES = [
  "created",
  "delivered",
  "accepted",
  "running",
  "succeeded",
  "failed",
  "canceled",
  "unknown",
] as const;

export type ErrorCodeName =
  | "invalid_frame"
  | "unsupported_version"
  | "unknown_action"
  | "missing_field"
  | "invalid_payload"
  | "frame_too_large"
  | "payload_too_large"
  | "idempotency_conflict"
  | "invalid_transition"
  | "unsupported_action"
  | "not_found"
  | "not_authorized"
  | "resource_exhausted"
  | "transport"
  | "internal";

export const ERROR_CODES = [
  "invalid_frame",
  "unsupported_version",
  "unknown_action",
  "missing_field",
  "invalid_payload",
  "frame_too_large",
  "payload_too_large",
  "idempotency_conflict",
  "invalid_transition",
  "unsupported_action",
  "not_found",
  "not_authorized",
  "resource_exhausted",
  "transport",
  "internal",
] as const;

/** The versioned action namespace (mobile spec section 6.1). */
export type ActionName =
  | "pair.begin"
  | "pair.prove_identity"
  | "pair.approve"
  | "pair.reject"
  | "connection.authenticate"
  | "connection.health"
  | "hosts.get_metadata"
  | "projects.list"
  | "sessions.list"
  | "sessions.get"
  | "sessions.events.subscribe"
  | "sessions.send_prompt"
  | "sessions.get_command_status"
  | "sessions.cancel"
  | "sessions.start"
  | "sessions.answer"
  | "approvals.list"
  | "approvals.get"
  | "approvals.respond"
  | "files.changed_summary"
  | "insights.summary"
  | "insights.errors"
  | "insights.agent_questions"
  | "devices.list"
  | "devices.revoke"
  | "push.register"
  | "push.unregister";

export const ACTION_NAMES = [
  "pair.begin",
  "pair.prove_identity",
  "pair.approve",
  "pair.reject",
  "connection.authenticate",
  "connection.health",
  "hosts.get_metadata",
  "projects.list",
  "sessions.list",
  "sessions.get",
  "sessions.events.subscribe",
  "sessions.send_prompt",
  "sessions.get_command_status",
  "sessions.cancel",
  "sessions.start",
  "sessions.answer",
  "approvals.list",
  "approvals.get",
  "approvals.respond",
  "files.changed_summary",
  "insights.summary",
  "insights.errors",
  "insights.agent_questions",
  "devices.list",
  "devices.revoke",
  "push.register",
  "push.unregister",
] as const satisfies readonly ActionName[];

export type ApprovalKind = "question" | "permission";
export type ApprovalRisk = "low" | "medium" | "high";
export type ApprovalDecision = "approve_once" | "deny";

/** The widget type for one agent-question field. */
export type ApprovalFieldKind =
  | "string"
  | "number"
  | "integer"
  | "boolean"
  | "multiselect"
  | "external";

/** One selectable answer for a question field. */
export interface ApprovalFieldOption {
  value: string;
  label: string;
  description?: string | null;
}

/** One field of an agent question. */
export interface ApprovalField {
  key: string;
  label: string;
  description?: string | null;
  kind: ApprovalFieldKind;
  required: boolean;
  /** Whether the user may answer with something other than `options`. */
  custom: boolean;
  options: ApprovalFieldOption[];
  placeholder?: string | null;
  /** For an `external` field: where the user completes it. */
  url?: string | null;
}

// ---- envelope ----

export interface Envelope {
  protocol_version: number;
  message_id: string;
  kind: MessageKind;
  action: ActionName;
  desktop_id?: string;
  session_id?: string;
  command_id?: string;
  payload: JsonValue;
}

// ---- connection / hosts ----

export interface WireError {
  code: ErrorCodeName | string;
  message: string;
}

export interface ConnectionHealth {
  desktop_id: string;
  protocol_version: number;
  app_version: string;
  uptime_secs: number;
}

export interface ConnectionAuthenticatePayload {
  device_id: string;
  desktop_id: string;
  protocol_version: number;
}

export interface ConnectionAuthenticateResult {
  authenticated: boolean;
  desktop_id: string;
}

export interface HostMetadata {
  desktop_id: string;
  name: string;
  platform: string;
  app_version: string;
  protocol_version: number;
}

// ---- projects ----

export interface ProjectSummary {
  id: string;
  name: string;
  active_sessions: number;
  pending_approvals: number;
  updated_at: number;
}

export interface ProjectsResponse {
  projects: ProjectSummary[];
}

// ---- sessions ----

export interface SessionSummary {
  id: string;
  project_id: string;
  title: string;
  agent: string;
  status: string;
  pending_decisions: number;
  started_at: number;
  updated_at: number;
}

export interface SessionDetail {
  id: string;
  project_id: string;
  title: string;
  agent: string;
  status: string;
  pending_decisions: number;
  started_at: number;
  updated_at: number;
  summary?: string | null;
  /** The instruction the agent was given (capped). */
  prompt?: string | null;
  /** A capped tail of the task log, for on-device detail. */
  log_tail?: string | null;
  files_changed: number;
}

export interface SessionsResponse {
  sessions: SessionSummary[];
}

export interface SessionEvent {
  seq: number;
  event_id: string;
  kind: string;
  timestamp: number;
  payload: JsonValue;
}

export interface EventsSubscribePayload {
  from_seq: number;
  session_id?: string | null;
}

export interface EventsBatch {
  session_id?: string | null;
  events: SessionEvent[];
  next_seq: number;
  resync: boolean;
}

export interface SendPromptPayload {
  text: string;
}

export interface CommandStatusQuery {
  command_id: string;
}

export interface CommandReceipt {
  command_id: string;
  status: CommandStatusName;
  session_id?: string | null;
}

export interface CommandStatusPayload {
  command_id: string;
  status: CommandStatusName;
  error?: WireError | null;
  updated_at: number;
}

// ---- approvals ----

export interface ApprovalSummary {
  id: string;
  session_id: string;
  project_id: string;
  kind: ApprovalKind;
  title: string;
  message?: string | null;
  command?: string | null;
  risk: ApprovalRisk;
  read_only: boolean;
  requested_at: number;
  expires_at?: number | null;
  /** The structured content of an agent question. Empty for permissions. */
  fields: ApprovalField[];
}

export interface ApprovalsResponse {
  approvals: ApprovalSummary[];
}

export interface ApprovalQuery {
  approval_id: string;
}

export interface ApprovalResponsePayload {
  approval_id: string;
  decision: ApprovalDecision;
}

/** Answers to an agent question, keyed by field. */
export interface AnswerPayload {
  approval_id: string;
  answers: Record<string, JsonValue>;
}

/** Selects a project for a read-only insight query. */
export interface ProjectQuery {
  project_id: string;
}

// ---- files ----

export interface ChangedFileSummary {
  path: string;
  status: string;
  additions?: number | null;
  deletions?: number | null;
}

export interface FilesChangedSummary {
  project_id: string;
  session_id: string;
  total: number;
  files: ChangedFileSummary[];
}

// ---- insights ----

export interface InsightsSummary {
  project_id: string;
  completed_tasks: number;
  failed_tasks: number;
  running_tasks: number;
  avg_duration_secs?: number | null;
}

export interface ErrorInsight {
  kind: string;
  count: number;
  last_seen: number;
}

export interface InsightsErrors {
  project_id: string;
  errors: ErrorInsight[];
}

export interface QuestionInsight {
  normalized_intent: string;
  count: number;
  last_seen: number;
}

export interface InsightsAgentQuestions {
  project_id: string;
  questions: QuestionInsight[];
}

// ---- devices ----

export interface DeviceSummary {
  device_id: string;
  name: string;
  platform: string;
  scopes: string[];
  paired_at: number;
  last_seen: number;
}

export interface DevicesResponse {
  devices: DeviceSummary[];
}

export interface DeviceRevokePayload {
  device_id: string;
}

export interface DeviceRevokeResult {
  revoked: boolean;
}

// ---- push ----

/** Register this device's push token with the desktop host. */
export interface PushRegisterPayload {
  token: string;
  /** `android` | `ios`. */
  platform: string;
}

export interface PushRegisterResult {
  registered: boolean;
}

export interface PushUnregisterPayload {
  token: string;
}

// ---- pairing ----

export interface PairBeginPayload {
  pairing_id: string;
  desktop_id: string;
  verification_code: string;
  expires_at: number;
}

export interface PairIdentityProof {
  pairing_id: string;
  device_id: string;
  proof: string;
}

export interface PairDecisionPayload {
  pairing_id: string;
  device_id: string;
}

export interface PairAck {
  pairing_id: string;
  state: string;
}
