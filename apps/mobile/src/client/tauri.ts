/**
 * The real transport-backed `RemoteClient`.
 *
 * Every call is one `remote_call` Tauri command carrying a protocol action and
 * its typed payload. The Rust side owns the envelope, idempotency key, and the
 * live Iroh/Iroh-authenticated connection, so the UI never touches networking or
 * cryptography directly — it only sees the DTOs from `@solayge/shared-types`.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  ApprovalDecision,
  ApprovalSummary,
  CommandReceipt,
  CommandStatusPayload,
  ConnectionHealth,
  EventsBatch,
  FilesChangedSummary,
  HostMetadata,
  JsonValue,
  ProjectSummary,
  PushPreferences,
  SessionDetail,
  SessionSummary,
} from "@solayge/shared-types";

import {
  RemoteError,
  type ClientDiagnostics,
  type RemoteClient,
  type TransportMode,
} from "./remote";

interface ActionError {
  code?: string;
  message?: string;
}

function toError(caught: unknown): RemoteError {
  if (caught instanceof RemoteError) return caught;
  const shaped = caught as ActionError;
  if (shaped && typeof shaped === "object" && typeof shaped.code === "string") {
    return new RemoteError(shaped.code, shaped.message ?? shaped.code);
  }
  return new RemoteError("internal", String(caught));
}

function newCommandId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `cmd_${crypto.randomUUID()}`;
  }
  return `cmd_${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
}

export class TauriRemoteClient implements RemoteClient {
  readonly desktopId: string;
  private connected = false;
  private lastError: string | null = null;
  private mode: TransportMode = "automatic";

  constructor(desktopId: string) {
    this.desktopId = desktopId;
  }

  setTransportMode(mode: TransportMode): void {
    if (this.mode === mode) return;
    this.mode = mode;
    // Changing the transport preference forces the next connect to redial so
    // the choice takes effect immediately.
    this.connected = false;
  }

  async connect(): Promise<void> {
    // No desktop selected (first launch) — nothing to dial.
    if (!this.desktopId) return;
    try {
      await invoke("connect_desktop", { desktopId: this.desktopId, mode: this.mode });
      this.connected = true;
      this.lastError = null;
    } catch (caught) {
      this.connected = false;
      this.lastError = String(caught);
      throw caught;
    }
  }

  isConnected(): boolean {
    return this.connected;
  }

  private async call<T>(
    action: string,
    payload: unknown = null,
    sessionId?: string,
    commandId?: string,
  ): Promise<T> {
    try {
      return await invoke<T>("remote_call", {
        action,
        payload,
        sessionId: sessionId ?? null,
        commandId: commandId ?? null,
      });
    } catch (caught) {
      const error = toError(caught);
      this.lastError = error.message;
      if (error.code === "transport" || error.code === "not_authorized") {
        this.connected = false;
      }
      throw error;
    }
  }

  health(): Promise<ConnectionHealth> {
    return this.call<ConnectionHealth>("connection.health");
  }

  hostMetadata(): Promise<HostMetadata> {
    return this.call<HostMetadata>("hosts.get_metadata");
  }

  async listProjects(): Promise<ProjectSummary[]> {
    const response = await this.call<{ projects: ProjectSummary[] }>("projects.list");
    return response.projects;
  }

  async listSessions(): Promise<SessionSummary[]> {
    const response = await this.call<{ sessions: SessionSummary[] }>("sessions.list");
    return response.sessions;
  }

  getSession(sessionId: string): Promise<SessionDetail> {
    return this.call<SessionDetail>("sessions.get", null, sessionId);
  }

  subscribeEvents(sessionId: string | null, fromSeq: number): Promise<EventsBatch> {
    return this.call<EventsBatch>("sessions.events.subscribe", {
      from_seq: fromSeq,
      session_id: sessionId,
    });
  }

  sendPrompt(
    sessionId: string,
    text: string,
    commandId: string,
  ): Promise<CommandReceipt> {
    return this.call<CommandReceipt>(
      "sessions.send_prompt",
      { text },
      sessionId,
      commandId,
    );
  }

  getCommandStatus(commandId: string): Promise<CommandStatusPayload> {
    return this.call<CommandStatusPayload>("sessions.get_command_status", {
      command_id: commandId,
    });
  }

  cancelSession(sessionId: string): Promise<CommandReceipt> {
    return this.call<CommandReceipt>(
      "sessions.cancel",
      null,
      sessionId,
      newCommandId(),
    );
  }

  async listApprovals(): Promise<ApprovalSummary[]> {
    const response = await this.call<{ approvals: ApprovalSummary[] }>("approvals.list");
    return response.approvals;
  }

  getApproval(approvalId: string): Promise<ApprovalSummary> {
    return this.call<ApprovalSummary>("approvals.get", { approval_id: approvalId });
  }

  respondApproval(
    approvalId: string,
    decision: ApprovalDecision,
  ): Promise<CommandReceipt> {
    return this.call<CommandReceipt>(
      "approvals.respond",
      { approval_id: approvalId, decision },
      undefined,
      newCommandId(),
    );
  }

  answerApproval(
    sessionId: string,
    approvalId: string,
    answers: Record<string, JsonValue>,
  ): Promise<CommandReceipt> {
    return this.call<CommandReceipt>(
      "sessions.answer",
      { approval_id: approvalId, answers },
      sessionId,
      newCommandId(),
    );
  }

  async registerPush(
    token: string,
    platform: string,
    preferences: PushPreferences,
  ): Promise<void> {
    await this.call<{ registered: boolean }>(
      "push.register",
      { token, platform, preferences },
      undefined,
      newCommandId(),
    );
  }

  async unregisterPush(token: string): Promise<void> {
    await this.call<{ registered: boolean }>(
      "push.unregister",
      { token },
      undefined,
      newCommandId(),
    );
  }

  filesChangedSummary(sessionId: string): Promise<FilesChangedSummary> {
    return this.call<FilesChangedSummary>("files.changed_summary", null, sessionId);
  }

  async diagnostics(): Promise<ClientDiagnostics> {
    const status = await invoke<{
      connected: boolean;
      authenticated: boolean;
      desktop_id?: string;
      transport?: string;
    }>("connection_status");
    return {
      transport: status.transport ?? "iroh",
      endpoint: status.desktop_id ?? this.desktopId,
      path: "unknown",
      rttMs: null,
      lastError: this.lastError,
    };
  }
}
