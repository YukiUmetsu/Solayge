import { describe, expect, it, vi } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MockRemoteClient } from "../client/mock";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { ApprovalDetail } from "./ApprovalDetail";

const client = () => new MockRemoteClient({ now: () => FIXED_NOW });

describe("Approval detail", () => {
  it("shows every required field for a permission request", async () => {
    renderWithClient(<ApprovalDetail approvalId="apr_1" />, client());

    expect(await screen.findByRole("heading", { name: "Run command?" })).toBeInTheDocument();
    expect(screen.getByText("Permission request")).toBeInTheDocument();
    expect(screen.getByText("Can modify files")).toBeInTheDocument();
    expect(screen.getByText("Medium impact")).toBeInTheDocument();
    expect(screen.getByText("Solayge")).toBeInTheDocument();
    expect(screen.getByText("Claude Code")).toBeInTheDocument();
    expect(screen.getByText("cargo test --workspace")).toBeInTheDocument();
    expect(screen.getByText("The agent wants to run the frontend test suite.")).toBeInTheDocument();
    expect(
      screen.getByText("This decision applies to this single request in this session only."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Approving allows this one operation. It does not approve other commands, now or later."),
    ).toBeInTheDocument();
    expect(screen.getByText(/Expiry in 1h/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Deny" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Approve once" })).toBeInTheDocument();
  });

  it("records one definitive decision and disables further action", async () => {
    const user = userEvent.setup();
    renderWithClient(<ApprovalDetail approvalId="apr_1" />, client());
    await screen.findByRole("heading", { name: "Run command?" });

    await user.click(screen.getByRole("button", { name: "Deny" }));

    expect(await screen.findByText("Denied")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve once" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Deny" })).toBeNull();
  });

  it("requires an extra confirmation for a high-impact approval", async () => {
    const user = userEvent.setup();
    const c = client();
    renderWithClient(<ApprovalDetail approvalId="apr_3" />, c);
    await screen.findByRole("heading", { name: "Delete generated files?" });

    await user.click(screen.getByRole("button", { name: "Approve once" }));

    const dialog = await screen.findByRole("dialog", { name: "Confirm approval" });
    expect(within(dialog).getByText("rm -rf dist/ && rm -rf target/")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Confirm approve once" }));

    expect(await screen.findByText("Approved once")).toBeInTheDocument();
  });

  it("treats a question as a message, never as authorization", async () => {
    const user = userEvent.setup();
    const c = client();
    const respond = vi.spyOn(c, "respondApproval");
    const send = vi.spyOn(c, "sendPrompt");
    renderWithClient(<ApprovalDetail approvalId="apr_2" />, c);

    expect(
      await screen.findByRole("heading", { name: "Which navigation library should I use?" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/not a permission request/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve once" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Deny" })).toBeNull();

    await user.type(screen.getByRole("textbox", { name: "Your answer" }), "Keep the existing stack.");
    await user.click(screen.getByRole("button", { name: "Send answer" }));

    expect(await screen.findByText("Received by desktop")).toBeInTheDocument();
    expect(send).toHaveBeenCalledWith("ses_3", "Keep the existing stack.", expect.any(String));
    expect(respond).not.toHaveBeenCalled();
  });

  it("reports a request that is no longer pending", async () => {
    renderWithClient(<ApprovalDetail approvalId="apr_1" />, new MockRemoteClient({ empty: true, now: () => FIXED_NOW }));
    // Re-query inside waitFor: the async "not found" resolution re-renders and
    // replaces the node, so a value captured once can be detached.
    await waitFor(() => expect(screen.getByText("Request resolved")).toBeInTheDocument());
  });

  it("does not claim a request is resolved when the desktop is unreachable", async () => {
    renderWithClient(
      <ApprovalDetail approvalId="apr_1" />,
      new MockRemoteClient({ offline: true, now: () => FIXED_NOW }),
    );
    expect(await screen.findByText("Request unavailable offline")).toBeInTheDocument();
    expect(screen.queryByText("Request resolved")).toBeNull();
  });
});
