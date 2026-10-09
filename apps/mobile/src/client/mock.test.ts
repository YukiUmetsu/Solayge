import { describe, expect, it } from "vitest";

import { MockRemoteClient } from "./mock";
import { RemoteError } from "./remote";

const NOW = 1_700_000_000;
const make = (options = {}) => new MockRemoteClient({ now: () => NOW, ...options });

describe("MockRemoteClient reads", () => {
  it("returns the seeded projects and sessions in insertion order", async () => {
    const client = make();
    expect((await client.listProjects()).map((p) => p.name)).toEqual(["Solayge", "FileQuell"]);
    expect((await client.listSessions()).length).toBe(5);
    expect((await client.getSession("ses_1")).title).toBe("Refactor UI");
  });

  it("throws a typed transport error when offline", async () => {
    const client = make({ offline: true });
    expect(client.isConnected()).toBe(false);
    await expect(client.listSessions()).rejects.toMatchObject({ code: "transport" });
  });

  it("throws a typed not-found for unknown sessions", async () => {
    const client = make();
    await expect(client.getSession("missing")).rejects.toBeInstanceOf(RemoteError);
    await expect(client.getSession("missing")).rejects.toMatchObject({ code: "not_found" });
  });

  it("seeds an empty client for empty-state coverage", async () => {
    const client = make({ empty: true });
    expect(await client.listSessions()).toEqual([]);
    expect(await client.listProjects()).toEqual([]);
    expect(await client.listApprovals()).toEqual([]);
  });
});

describe("MockRemoteClient commands", () => {
  it("is idempotent: the same command id executes once", async () => {
    const client = make();
    const first = await client.sendPrompt("ses_1", "run tests", "cmd_1");
    expect(first.status).toBe("delivered");
    await client.sendPrompt("ses_1", "run tests", "cmd_1");
    const events = await client.subscribeEvents("ses_1", 0);
    const userMessages = events.events.filter((e) => e.kind === "message");
    // Four seeded events plus exactly one new user message.
    expect(userMessages.length).toBe(2);
  });

  it("never fabricates terminal success in getCommandStatus", async () => {
    const client = make();
    await client.sendPrompt("ses_1", "hello", "cmd_1");
    expect((await client.getCommandStatus("cmd_1")).status).toBe("delivered");
    client.setCommandStatus("cmd_1", "succeeded");
    expect((await client.getCommandStatus("cmd_1")).status).toBe("succeeded");
  });

  it("rejects an empty message as an invalid payload", async () => {
    const client = make();
    await expect(client.sendPrompt("ses_1", "   ", "cmd_1")).rejects.toMatchObject({
      code: "invalid_payload",
    });
  });
});

describe("MockRemoteClient approvals", () => {
  it("records exactly one definitive decision and is idempotent", async () => {
    const client = make();
    expect(await client.listApprovals()).toHaveLength(3);
    const receipt = await client.respondApproval("apr_1", "approve_once");
    expect(receipt.status).toBe("succeeded");
    expect(client.isResolved("apr_1")).toBe(true);
    expect((await client.listApprovals()).map((a) => a.id)).not.toContain("apr_1");

    const again = await client.respondApproval("apr_1", "approve_once");
    expect(again.status).toBe("succeeded");
    expect(await client.listApprovals()).toHaveLength(2);
  });

  it("reports a lost request as not found", async () => {
    const client = make();
    await expect(client.respondApproval("missing", "deny")).rejects.toMatchObject({
      code: "not_found",
    });
  });
});

describe("MockRemoteClient diagnostics", () => {
  it("describes the loopback transport without secrets", async () => {
    const diagnostics = await make().diagnostics();
    expect(diagnostics?.transport).toContain("loopback");
    expect(diagnostics?.endpoint).toContain("mock://");
  });
});
