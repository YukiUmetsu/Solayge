/**
 * An in-memory `RemoteClient` for local development and tests.
 *
 * It mirrors the seed and behavior of `solayge_protocol::client::MockHost`:
 * idempotent commands keyed by `command_id`, one definitive approval decision,
 * and transport errors when the desktop is offline. It never touches the
 * network or the OS, which is exactly what the UI is allowed to depend on.
 */
import type {
  ApprovalDecision,
  ApprovalSummary,
  ChangedFileSummary,
  CommandReceipt,
  CommandStatusName,
  CommandStatusPayload,
  ConnectionHealth,
  EventsBatch,
  FilesChangedSummary,
  HostMetadata,
  JsonValue,
  ProjectSummary,
  PushPreferences,
  SessionDetail,
  SessionEvent,
  SessionSummary,
} from "@solayge/shared-types";
import { PROTOCOL_VERSION } from "@solayge/shared-types";

import {
  RemoteError,
  type ClientDiagnostics,
  type RemoteClient,
} from "./remote";

export interface MockClientOptions {
  desktopId?: string;
  hostName?: string;
  platform?: string;
  appVersion?: string;
  /** Start disconnected; every exchange then fails with a transport error. */
  offline?: boolean;
  /** Seed with no projects/sessions/approvals (for empty-state coverage). */
  empty?: boolean;
  /**
   * When true, a sent prompt advances on a timer for a lively local demo.
   * Tests keep this off so statuses stay deterministic.
   */
  autoAdvance?: boolean;
  /** The clock used to seed relative timestamps (seconds since epoch). */
  now?: () => number;
  /** Extra draft sessions to seed (drafts are otherwise not in the fixture). */
  drafts?: { title: string; project_id?: string }[];
}

const AGENT_FIXTURE = "Claude Code";

function secondsNow(): number {
  return Math.floor(Date.now() / 1000);
}

/**
 * A stable id for a logical command so that retrying reuses the same
 * idempotency key, exactly as the real client must.
 */
export function commandIdFor(desktopId: string, sessionId: string, text: string): string {
  const input = `${desktopId}\u0000${sessionId}\u0000${text}`;
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i += 1) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `cmd_${hash.toString(16).padStart(8, "0")}`;
}

export class MockRemoteClient implements RemoteClient {
  readonly desktopId: string;

  private readonly hst: HostMetadata;
  private readonly now: () => number;
  private readonly autoAdvance: boolean;

  private projects: ProjectSummary[];
  private sessions: SessionDetail[];
  private approvals: ApprovalSummary[];
  private readonly events = new Map<string, SessionEvent[]>();
  private readonly changed = new Map<string, ChangedFileSummary[]>();
  private readonly commands = new Map<string, CommandStatusPayload>();
  private readonly decisions = new Map<string, CommandReceipt>();
  private readonly resolved = new Set<string>();

  private offline: boolean;

  constructor(options: MockClientOptions = {}) {
    this.now = options.now ?? secondsNow;
    this.offline = options.offline ?? false;
    this.autoAdvance = options.autoAdvance ?? false;
    this.desktopId = options.desktopId ?? "desk_mock";
    this.hst = {
      desktop_id: this.desktopId,
      name: options.hostName ?? "Mock Desktop",
      platform: options.platform ?? "macos",
      app_version: options.appVersion ?? "0.1.0",
      protocol_version: PROTOCOL_VERSION,
    };
    this.projects = [];
    this.sessions = [];
    this.approvals = [];
    if (!options.empty) this.seed();
    (options.drafts ?? []).forEach((draft, index) => {
      this.sessions.push(
        detail({
          id: `draft_${index + 1}`,
          project_id: draft.project_id ?? "/repo/solayge",
          title: draft.title,
          agent: AGENT_FIXTURE,
          status: "draft",
          pending_decisions: 0,
          started_at: this.now(),
          updated_at: this.now(),
          summary: null,
          files_changed: 0,
        }),
      );
    });
  }

  // ---- connection ----

  isConnected(): boolean {
    return !this.offline;
  }

  setOffline(offline: boolean): void {
    this.offline = offline;
  }

  private requireOnline(): void {
    if (this.offline) {
      throw new RemoteError("transport", "The desktop is unreachable. Showing last known data.");
    }
  }

  async health(): Promise<ConnectionHealth> {
    this.requireOnline();
    return {
      desktop_id: this.hst.desktop_id,
      protocol_version: this.hst.protocol_version,
      app_version: this.hst.app_version,
      uptime_secs: 42,
    };
  }

  async hostMetadata(): Promise<HostMetadata> {
    this.requireOnline();
    return { ...this.hst };
  }

  async diagnostics(): Promise<ClientDiagnostics> {
    return {
      transport: "loopback (local development)",
      endpoint: `mock://${this.desktopId}`,
      path: "direct",
      rttMs: 0,
      lastError: this.offline ? "transport: desktop unreachable" : null,
    };
  }

  // ---- reads ----

  async listProjects(): Promise<ProjectSummary[]> {
    this.requireOnline();
    return this.projects.map((p) => ({ ...p }));
  }

  async listSessions(): Promise<SessionSummary[]> {
    this.requireOnline();
    return this.sessions.map(summaryOf);
  }

  async getSession(sessionId: string): Promise<SessionDetail> {
    this.requireOnline();
    const session = this.sessions.find((s) => s.id === sessionId);
    if (!session) throw new RemoteError("not_found", `Unknown session ${sessionId}`);
    return { ...session };
  }

  async subscribeEvents(sessionId: string | null, fromSeq: number): Promise<EventsBatch> {
    this.requireOnline();
    const list: SessionEvent[] = [];
    if (sessionId) {
      for (const event of this.events.get(sessionId) ?? []) {
        if (event.seq >= fromSeq) list.push({ ...event });
      }
    } else {
      for (const stream of this.events.values()) {
        for (const event of stream) if (event.seq >= fromSeq) list.push({ ...event });
      }
      list.sort((a, b) => a.seq - b.seq);
    }
    const nextSeq = list.length > 0 ? list[list.length - 1].seq + 1 : fromSeq;
    return { session_id: sessionId, events: list, next_seq: nextSeq, resync: false };
  }

  async filesChangedSummary(sessionId: string): Promise<FilesChangedSummary> {
    this.requireOnline();
    const session = this.sessions.find((s) => s.id === sessionId);
    if (!session) throw new RemoteError("not_found", `Unknown session ${sessionId}`);
    const files = (this.changed.get(sessionId) ?? []).map((f) => ({ ...f }));
    return {
      project_id: session.project_id,
      session_id: sessionId,
      total: files.length,
      files,
    };
  }

  // ---- commands ----

  async sendPrompt(
    sessionId: string,
    text: string,
    commandId: string = commandIdFor(this.desktopId, sessionId, text),
  ): Promise<CommandReceipt> {
    this.requireOnline();
    const session = this.sessions.find((s) => s.id === sessionId);
    if (!session) throw new RemoteError("not_found", `Unknown session ${sessionId}`);
    if (!text.trim()) throw new RemoteError("invalid_payload", "Message text is empty");

    const existing = this.commands.get(commandId);
    if (existing) {
      // A duplicate never runs the agent a second time.
      return { command_id: commandId, status: existing.status, session_id: sessionId };
    }

    this.pushEvent(sessionId, {
      kind: "message",
      payload: { role: "user", text },
    });
    const status: CommandStatusName = "delivered";
    this.commands.set(commandId, {
      command_id: commandId,
      status,
      error: null,
      updated_at: this.now(),
    });
    if (this.autoAdvance) this.scheduleAdvance(commandId);
    return { command_id: commandId, status, session_id: sessionId };
  }

  private scheduleAdvance(commandId: string): void {
    const steps: CommandStatusName[] = ["accepted", "running", "succeeded"];
    steps.forEach((status, index) => {
      setTimeout(() => {
        const current = this.commands.get(commandId);
        if (current && current.status !== "canceled") {
          this.commands.set(commandId, { ...current, status, updated_at: this.now() });
        }
      }, 400 * (index + 1));
    });
  }

  async getCommandStatus(commandId: string): Promise<CommandStatusPayload> {
    this.requireOnline();
    const status = this.commands.get(commandId);
    if (!status) throw new RemoteError("not_found", `Unknown command ${commandId}`);
    return { ...status };
  }

  async cancelSession(sessionId: string): Promise<CommandReceipt> {
    this.requireOnline();
    const session = this.sessions.find((s) => s.id === sessionId);
    if (!session) throw new RemoteError("not_found", `Unknown session ${sessionId}`);
    const commandId = commandIdFor(this.desktopId, sessionId, "cancel");
    const status: CommandStatusName = "canceled";
    this.commands.set(commandId, {
      command_id: commandId,
      status,
      error: null,
      updated_at: this.now(),
    });
    const index = this.sessions.findIndex((s) => s.id === sessionId);
    if (index >= 0) {
      this.sessions[index] = { ...this.sessions[index], status: "canceled", updated_at: this.now() };
    }
    return { command_id: commandId, status, session_id: sessionId };
  }

  // ---- approvals ----

  async listApprovals(): Promise<ApprovalSummary[]> {
    this.requireOnline();
    return this.approvals.map((a) => ({ ...a }));
  }

  async getApproval(approvalId: string): Promise<ApprovalSummary> {
    this.requireOnline();
    const approval = this.approvals.find((a) => a.id === approvalId);
    if (!approval) throw new RemoteError("not_found", `Unknown approval ${approvalId}`);
    return { ...approval };
  }

  async respondApproval(
    approvalId: string,
    decision: ApprovalDecision,
  ): Promise<CommandReceipt> {
    this.requireOnline();
    const commandId = `apr_cmd_${approvalId}_${decision}`;
    const existing = this.decisions.get(commandId);
    if (existing) return { ...existing };

    const index = this.approvals.findIndex((a) => a.id === approvalId);
    if (index < 0) {
      // A request that was already decided (or never existed) is not found;
      // the caller must not treat a lost decision as a successful replay.
      this.decisions.delete(commandId);
      throw new RemoteError("not_found", `Unknown approval ${approvalId}`);
    }
    const [approval] = this.approvals.splice(index, 1);
    const receipt: CommandReceipt = {
      command_id: commandId,
      status: "succeeded",
      session_id: approval.session_id,
    };
    this.decisions.set(commandId, receipt);
    this.resolved.add(approvalId);

    // The pending count on the session reflects the resolved decision.
    const sessionIndex = this.sessions.findIndex((s) => s.id === approval.session_id);
    if (sessionIndex >= 0) {
      this.sessions[sessionIndex] = {
        ...this.sessions[sessionIndex],
        pending_decisions: Math.max(0, this.sessions[sessionIndex].pending_decisions - 1),
        updated_at: this.now(),
      };
    }
    return { ...receipt };
  }

  async answerApproval(
    sessionId: string,
    approvalId: string,
    answers: Record<string, JsonValue>,
  ): Promise<CommandReceipt> {
    this.requireOnline();
    const commandId = `ans_cmd_${approvalId}`;
    const existing = this.decisions.get(commandId);
    if (existing) return { ...existing };

    const index = this.approvals.findIndex((a) => a.id === approvalId);
    if (index < 0) {
      this.decisions.delete(commandId);
      throw new RemoteError("not_found", `Unknown approval ${approvalId}`);
    }
    const [approval] = this.approvals.splice(index, 1);
    if (approval.session_id !== sessionId) {
      // Put it back: answering another session's question is refused.
      this.approvals.splice(index, 0, approval);
      this.decisions.delete(commandId);
      throw new RemoteError("not_authorized", "The question belongs to another session");
    }
    const receipt: CommandReceipt = {
      command_id: commandId,
      status: "succeeded",
      session_id: approval.session_id,
    };
    this.decisions.set(commandId, receipt);
    this.resolved.add(approvalId);

    // Answering is a message, so it appears in the transcript like any other.
    this.pushEvent(approval.session_id, {
      kind: "message",
      payload: { role: "user", text: Object.values(answers).join(", ") },
    });

    const sessionIndex = this.sessions.findIndex((s) => s.id === approval.session_id);
    if (sessionIndex >= 0) {
      this.sessions[sessionIndex] = {
        ...this.sessions[sessionIndex],
        pending_decisions: Math.max(0, this.sessions[sessionIndex].pending_decisions - 1),
        updated_at: this.now(),
      };
    }
    return { ...receipt };
  }

  async registerPush(
    _token: string,
    _platform: string,
    _preferences: PushPreferences,
  ): Promise<void> {
    // The in-memory mock has no push gateway; registration is a no-op.
  }

  async unregisterPush(_token: string): Promise<void> {
    // The in-memory mock has no push gateway; unregistration is a no-op.
  }

  /** Whether an approval has already received its one definitive decision. */
  isResolved(approvalId: string): boolean {
    return this.resolved.has(approvalId);
  }

  // ---- test / dev helpers ----

  setCommandStatus(commandId: string, status: CommandStatusName): void {
    const current = this.commands.get(commandId);
    this.commands.set(commandId, {
      command_id: commandId,
      status,
      error: current?.error ?? null,
      updated_at: this.now(),
    });
  }

  private pushEvent(
    sessionId: string,
    event: { kind: string; payload: SessionEvent["payload"] },
  ): void {
    const stream = this.events.get(sessionId) ?? [];
    const seq = stream.length > 0 ? stream[stream.length - 1].seq + 1 : 1;
    stream.push({
      seq,
      event_id: `${sessionId}_ev_${seq}`,
      kind: event.kind,
      timestamp: this.now(),
      payload: event.payload,
    });
    this.events.set(sessionId, stream);
  }

  private seed(): void {
    const now = this.now();
    this.projects = [
      { id: "/repo/solayge", name: "Solayge", active_sessions: 1, pending_approvals: 2, updated_at: now - 45 },
      { id: "/repo/filequell", name: "FileQuell", active_sessions: 1, pending_approvals: 1, updated_at: now - 120 },
    ];
    this.sessions = [
      detail({
        id: "ses_1",
        project_id: "/repo/solayge",
        title: "Refactor UI",
        agent: AGENT_FIXTURE,
        status: "running",
        pending_decisions: 1,
        started_at: now - 720,
        updated_at: now - 45,
        summary: "Updated 3 components; now running frontend tests.",
        prompt: "Refactor the header and network panels, then run the frontend tests.",
        log_tail:
          "[12:01] Editing src/SessionView.tsx\n[12:02] Editing src/NetworkPanel.tsx\n[12:04] $ pnpm test\nRunning frontend tests…",
        files_changed: 2,
      }),
      detail({
        id: "ses_3",
        project_id: "/repo/solayge",
        title: "Fix navigation",
        agent: "Codex",
        status: "waiting",
        pending_decisions: 1,
        started_at: now - 900,
        updated_at: now - 180,
        summary: "Navigation stack fixed; one design question left.",
        files_changed: 1,
      }),
      detail({
        id: "ses_4",
        project_id: "/repo/filequell",
        title: "Update dependencies",
        agent: "Codex",
        status: "running",
        pending_decisions: 1,
        started_at: now - 1800,
        updated_at: now - 300,
        summary: "Resolving the dependency tree; needs permission to clean build output.",
        files_changed: 0,
      }),
      detail({
        id: "ses_2",
        project_id: "/repo/filequell",
        title: "PDF import",
        agent: "OpenCode",
        status: "running",
        pending_decisions: 0,
        started_at: now - 3600,
        updated_at: now - 120,
        summary: "Editing 3 files for the PDF import path.",
        files_changed: 3,
      }),
      detail({
        id: "ses_5",
        project_id: "/repo/filequell",
        title: "Add dark mode",
        agent: "OpenCode",
        status: "completed",
        pending_decisions: 0,
        started_at: now - 7200,
        updated_at: now - 3600,
        summary: "Dark mode implemented; all tests passed.",
        files_changed: 1,
      }),
    ];

    this.approvals = [
      {
        id: "apr_1",
        session_id: "ses_1",
        project_id: "/repo/solayge",
        kind: "permission",
        title: "Run command?",
        message: "The agent wants to run the frontend test suite.",
        command: "cargo test --workspace",
        risk: "medium",
        read_only: false,
        requested_at: now - 120,
        expires_at: now + 3600,
        fields: [],
      },
      {
        id: "apr_2",
        session_id: "ses_3",
        project_id: "/repo/solayge",
        kind: "question",
        title: "Which navigation library should I use?",
        message:
          "I can either keep the existing stack or move to a standards-compliant one. Which do you prefer?",
        command: null,
        risk: "low",
        read_only: true,
        requested_at: now - 180,
        expires_at: null,
        fields: [
          {
            key: "approach",
            label: "Navigation approach",
            description: "Pick one or type your own preference.",
            kind: "string",
            required: true,
            custom: true,
            options: [
              { value: "existing", label: "Keep the existing stack", description: null },
              {
                value: "standards",
                label: "Move to a standards-compliant one",
                description: null,
              },
            ],
            placeholder: "Type your preference…",
            url: null,
          },
        ],
      },
      {
        id: "apr_3",
        session_id: "ses_4",
        project_id: "/repo/filequell",
        kind: "permission",
        title: "Delete generated files?",
        message: "The agent wants to remove build output before reinstalling dependencies.",
        command: "rm -rf dist/ && rm -rf target/",
        risk: "high",
        read_only: false,
        requested_at: now - 60,
        expires_at: now + 1800,
        fields: [],
      },
    ];

    this.events.set("ses_1", [
      ev("ses_1", 1, "message", { role: "agent", text: "I've updated the header component." }),
      ev("ses_1", 2, "tool", { tool: "edit", path: "src/SessionView.tsx" }),
      ev("ses_1", 3, "tool", { tool: "edit", path: "src/NetworkPanel.tsx" }),
      ev("ses_1", 4, "status", { status: "running", detail: "Running frontend tests…" }),
    ]);
    this.events.set("ses_2", [
      ev("ses_2", 1, "message", { role: "agent", text: "Starting the PDF import implementation." }),
      ev("ses_2", 2, "tool", { tool: "edit", path: "src/pdf/import.ts" }),
      ev("ses_2", 3, "status", { status: "running", detail: "Editing 3 files…" }),
    ]);
    this.events.set("ses_3", [
      ev("ses_3", 1, "message", { role: "agent", text: "I fixed the navigation stack." }),
      ev("ses_3", 2, "tool", { tool: "edit", path: "src/nav/Stack.tsx" }),
      ev("ses_3", 3, "status", { status: "waiting", detail: "Waiting for your answer" }),
    ]);
    this.events.set("ses_4", [
      ev("ses_4", 1, "message", { role: "agent", text: "Updating dependencies now." }),
      ev("ses_4", 2, "status", { status: "running", detail: "Resolving dependency tree…" }),
    ]);
    this.events.set("ses_5", [
      ev("ses_5", 1, "message", { role: "agent", text: "Dark mode is implemented." }),
      ev("ses_5", 2, "tool", { tool: "test", path: "src/theme.test.ts" }),
      ev("ses_5", 3, "status", { status: "completed", detail: "All tests passed" }),
    ]);

    this.changed.set("ses_1", [
      { path: "src/SessionView.tsx", status: "modified", additions: 42, deletions: 9 },
      { path: "src/NetworkPanel.tsx", status: "modified", additions: 12, deletions: 3 },
    ]);
    this.changed.set("ses_2", [
      { path: "src/pdf/import.ts", status: "added", additions: 120, deletions: 0 },
      { path: "src/pdf/parse.ts", status: "added", additions: 64, deletions: 0 },
      { path: "src/index.ts", status: "modified", additions: 3, deletions: 1 },
    ]);
    this.changed.set("ses_5", [
      { path: "src/theme.ts", status: "added", additions: 58, deletions: 0 },
    ]);
  }
}

function detail(session: SessionDetail): SessionDetail {
  return session;
}

function summaryOf(session: SessionDetail): SessionSummary {
  return {
    id: session.id,
    project_id: session.project_id,
    title: session.title,
    agent: session.agent,
    status: session.status,
    pending_decisions: session.pending_decisions,
    started_at: session.started_at,
    updated_at: session.updated_at,
  };
}

function ev(
  sessionId: string,
  seq: number,
  kind: string,
  payload: SessionEvent["payload"],
): SessionEvent {
  return { seq, event_id: `${sessionId}_ev_${seq}`, kind, timestamp: seq * 10, payload };
}
