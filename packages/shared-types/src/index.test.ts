import { describe, expect, it } from "vitest";

import fixture from "../fixtures/protocol-sample.json";
import {
  ACTION_NAMES,
  COMMAND_STATUSES,
  ERROR_CODES,
  MESSAGE_KINDS,
  type ApprovalSummary,
  type CommandReceipt,
  type Envelope,
  type SessionDetail,
} from "./index";

/** Assert an object has exactly the given keys, catching renames on either side. */
function expectKeys(value: unknown, keys: string[]): void {
  expect(Object.keys(value as Record<string, unknown>).sort()).toEqual([...keys].sort());
}

const EXPECTED_KEYS: Record<string, string[]> = {
  envelope: [
    "protocol_version",
    "message_id",
    "kind",
    "action",
    "desktop_id",
    "session_id",
    "command_id",
    "payload",
  ],
  connection_health: ["desktop_id", "protocol_version", "app_version", "uptime_secs"],
  connection_authenticate_payload: ["device_id", "desktop_id", "protocol_version"],
  connection_authenticate_result: ["authenticated", "desktop_id"],
  host_metadata: ["desktop_id", "name", "platform", "app_version", "protocol_version"],
  project_summary: ["id", "name", "active_sessions", "pending_approvals", "updated_at"],
  session_summary: [
    "id",
    "project_id",
    "title",
    "agent",
    "status",
    "pending_decisions",
    "started_at",
    "updated_at",
  ],
  session_detail: [
    "id",
    "project_id",
    "title",
    "agent",
    "status",
    "pending_decisions",
    "started_at",
    "updated_at",
    "summary",
    "prompt",
    "log_tail",
    "files_changed",
  ],
  session_event: ["seq", "event_id", "kind", "timestamp", "payload"],
  events_subscribe_payload: ["from_seq", "session_id"],
  events_batch: ["session_id", "events", "next_seq", "resync"],
  send_prompt_payload: ["text"],
  command_status_query: ["command_id"],
  command_receipt: ["command_id", "status", "session_id"],
  command_status: ["command_id", "status", "error", "updated_at"],
  wire_error: ["code", "message"],
  approval_summary: [
    "id",
    "session_id",
    "project_id",
    "kind",
    "title",
    "message",
    "command",
    "risk",
    "read_only",
    "requested_at",
    "expires_at",
    "fields",
  ],
  approval_field: [
    "key",
    "label",
    "description",
    "kind",
    "required",
    "custom",
    "options",
    "placeholder",
    "url",
  ],
  approval_query: ["approval_id"],
  approval_response_payload: ["approval_id", "decision"],
  answer_payload: ["approval_id", "answers"],
  project_query: ["project_id"],
  changed_file_summary: ["path", "status", "additions", "deletions"],
  files_changed_summary: ["project_id", "session_id", "total", "files"],
  insights_summary: [
    "project_id",
    "completed_tasks",
    "failed_tasks",
    "running_tasks",
    "avg_duration_secs",
  ],
  error_insight: ["kind", "count", "last_seen"],
  insights_errors: ["project_id", "errors"],
  question_insight: ["normalized_intent", "count", "last_seen"],
  insights_agent_questions: ["project_id", "questions"],
  device_summary: ["device_id", "name", "platform", "scopes", "paired_at", "last_seen"],
  devices_response: ["devices"],
  device_revoke_payload: ["device_id"],
  device_revoke_result: ["revoked"],
  push_register_payload: ["token", "platform"],
  push_register_result: ["registered"],
  push_unregister_payload: ["token"],
  pair_begin_payload: ["pairing_id", "desktop_id", "verification_code", "expires_at"],
  pair_identity_proof: ["pairing_id", "device_id", "proof"],
  pair_decision_payload: ["pairing_id", "device_id"],
  pair_ack: ["pairing_id", "state"],
};

describe("shared protocol fixture", () => {
  it("exposes every DTO with the exact field names", () => {
    for (const [name, keys] of Object.entries(EXPECTED_KEYS)) {
      const value = (fixture as Record<string, unknown>)[name];
      expect(value, `fixture is missing ${name}`).toBeDefined();
      expectKeys(value, keys);
    }
  });

  it("union-valued fields use the shared constants", () => {
    expect(MESSAGE_KINDS as readonly string[]).toContain(fixture.envelope.kind);
    expect(ACTION_NAMES as readonly string[]).toContain(fixture.envelope.action);
    expect(COMMAND_STATUSES as readonly string[]).toContain(fixture.command_receipt.status);
    expect(COMMAND_STATUSES as readonly string[]).toContain(fixture.command_status.status);
    expect(ERROR_CODES as readonly string[]).toContain(fixture.wire_error.code);
    expect(["question", "permission"]).toContain(fixture.approval_summary.kind);
    expect(["low", "medium", "high"]).toContain(fixture.approval_summary.risk);
    expect(["approve_once", "deny"]).toContain(fixture.approval_response_payload.decision);
  });

  it("keeps the full action namespace covered", () => {
    expect(ACTION_NAMES).toHaveLength(27);
    expect(new Set(ACTION_NAMES).size).toBe(ACTION_NAMES.length);
  });

  it("fixture values are assignable to the exported interfaces", () => {
    // Explicit typed bindings; the casts document that the fixture is the
    // contract and that these interfaces describe it.
    const envelope = fixture.envelope as Envelope;
    const receipt = fixture.command_receipt as CommandReceipt;
    const detail = fixture.session_detail as SessionDetail;
    const approval = fixture.approval_summary as ApprovalSummary;

    expect(envelope.protocol_version).toBe(1);
    expect(receipt.status).toBe("running");
    expect(detail.files_changed).toBe(2);
    expect(approval.kind).toBe("permission");
  });
});
