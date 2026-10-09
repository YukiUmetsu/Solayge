import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import { RemoteError } from "./remote";
import { TauriRemoteClient } from "./tauri";

describe("TauriRemoteClient", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("authenticates through connect_desktop and reports the connection", async () => {
    invokeMock.mockResolvedValue({
      connected: true,
      authenticated: true,
      desktop_id: "did_desk",
    });
    const client = new TauriRemoteClient("did_desk");
    expect(client.isConnected()).toBe(false);

    await client.connect();

    expect(invokeMock).toHaveBeenCalledWith("connect_desktop", { desktopId: "did_desk" });
    expect(client.isConnected()).toBe(true);
  });

  it("maps a read onto its protocol action and unwraps the response", async () => {
    invokeMock.mockResolvedValue({
      projects: [
        { id: "/repo", name: "app", active_sessions: 1, pending_approvals: 0, updated_at: 5 },
      ],
    });
    const client = new TauriRemoteClient("did_desk");

    const projects = await client.listProjects();

    expect(invokeMock).toHaveBeenCalledWith("remote_call", {
      action: "projects.list",
      payload: null,
      sessionId: null,
      commandId: null,
    });
    expect(projects).toHaveLength(1);
    expect(projects[0].name).toBe("app");
  });

  it("forwards the session and idempotency key for a mutating call", async () => {
    invokeMock.mockResolvedValue({ command_id: "cmd_1", status: "delivered" });
    const client = new TauriRemoteClient("did_desk");

    await client.sendPrompt("task_1", "run tests", "cmd_1");

    expect(invokeMock).toHaveBeenCalledWith("remote_call", {
      action: "sessions.send_prompt",
      payload: { text: "run tests" },
      sessionId: "task_1",
      commandId: "cmd_1",
    });
  });

  it("turns a typed wire error into a RemoteError and drops the connection", async () => {
    invokeMock.mockRejectedValue({
      code: "transport",
      message: "desktop unreachable",
    });
    const client = new TauriRemoteClient("did_desk");

    await expect(client.hostMetadata()).rejects.toBeInstanceOf(RemoteError);
    expect(client.isConnected()).toBe(false);
  });
});
