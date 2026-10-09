/**
 * The mobile-side `RemoteClient` seam.
 *
 * This mirrors the Rust `solayge_protocol::client::RemoteClient` trait method
 * for method, but is asynchronous because the JS UI never blocks on a frame
 * exchange. Every payload is a DTO from `@solayge/shared-types`, the
 * hand-maintained mirror of the Rust `solayge-protocol` DTOs. No screen imports
 * a transport or a native capability: it only ever talks to this interface.
 */
import type {
  ApprovalDecision,
  ApprovalSummary,
  CommandReceipt,
  CommandStatusPayload,
  ConnectionHealth,
  EventsBatch,
  FilesChangedSummary,
  HostMetadata,
  ProjectSummary,
  SessionDetail,
  SessionSummary,
} from "@solayge/shared-types";

/** A typed protocol failure, mirroring `solayge_protocol::WireError`. */
export class RemoteError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "RemoteError";
    this.code = code;
  }
}

/** The transport preference (mobile spec section 5.2). */
export type TransportMode = "automatic" | "iroh" | "tailscale";

/** A desktop the user has paired, as far as the mobile client knows. */
export interface PairedDesktop {
  id: string;
  name: string;
  platform: string;
}

/**
 * The one primitive the UI depends on. Real transports (Iroh, Tailscale WSS)
 * will implement this; the in-memory mock implements it for local development
 * and tests.
 */
export interface RemoteClient {
  /** The selected desktop's public id. */
  readonly desktopId: string;

  /** Whether a transport is currently established. */
  isConnected(): boolean;

  health(): Promise<ConnectionHealth>;
  hostMetadata(): Promise<HostMetadata>;

  listProjects(): Promise<ProjectSummary[]>;
  listSessions(): Promise<SessionSummary[]>;
  getSession(sessionId: string): Promise<SessionDetail>;

  subscribeEvents(sessionId: string | null, fromSeq: number): Promise<EventsBatch>;

  /** Send a prompt, reusing `commandId` so a retry is idempotent. */
  sendPrompt(sessionId: string, text: string, commandId: string): Promise<CommandReceipt>;
  getCommandStatus(commandId: string): Promise<CommandStatusPayload>;
  cancelSession(sessionId: string): Promise<CommandReceipt>;

  listApprovals(): Promise<ApprovalSummary[]>;
  getApproval(approvalId: string): Promise<ApprovalSummary>;
  respondApproval(approvalId: string, decision: ApprovalDecision): Promise<CommandReceipt>;

  filesChangedSummary(sessionId: string): Promise<FilesChangedSummary>;

  /** Optional, transport-specific diagnostics for the Settings screen. */
  diagnostics?(): Promise<ClientDiagnostics>;
}

/** Sanitized, display-only connection diagnostics (never secrets). */
export interface ClientDiagnostics {
  transport: string;
  endpoint: string;
  /** Whether the current path is direct or relayed, when knowable. */
  path: "direct" | "relayed" | "unknown";
  rttMs: number | null;
  lastError: string | null;
}
