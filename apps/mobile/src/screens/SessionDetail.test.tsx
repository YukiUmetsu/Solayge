import { describe, expect, it, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MockRemoteClient } from "../client/mock";
import { RemoteError } from "../client/remote";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { SessionDetail } from "./SessionDetail";

const client = () => new MockRemoteClient({ now: () => FIXED_NOW });

describe("Session detail (loopback client)", () => {
  it("shows summary, current step, timeline, and pending decisions", async () => {
    renderWithClient(
      <SessionDetail sessionId="ses_1" onOpenApproval={() => {}} pollMs={0} />,
      client(),
    );

    expect(await screen.findByRole("heading", { name: "Refactor UI" })).toBeInTheDocument();
    expect(await screen.findByText("Updated 3 components; now running frontend tests.")).toBeInTheDocument();
    expect(within(screen.getByRole("region", { name: "Current step" })).getByText(/Running frontend tests/)).toBeInTheDocument();
    expect(screen.getAllByText("Edited").length).toBeGreaterThan(0);
    expect(screen.getAllByText("src/SessionView.tsx").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: /Run command\?/ })).toBeInTheDocument();
  });

  it("shows the prompt and log in an expandable details section, and reloads on demand", async () => {
    const user = userEvent.setup();
    const c = client();
    const spy = vi.spyOn(c, "getSession");
    renderWithClient(
      <SessionDetail sessionId="ses_1" onOpenApproval={() => {}} pollMs={0} />,
      c,
    );
    await screen.findByRole("heading", { name: "Refactor UI" });

    // Collapsed by default.
    expect(screen.queryByText(/Refactor the header and network panels/)).toBeNull();
    await user.click(await screen.findByText("Task details"));
    expect(screen.getByText(/Refactor the header and network panels/)).toBeInTheDocument();
    expect(screen.getByText(/pnpm test/)).toBeInTheDocument();

    const before = spy.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Reload this task from the desktop" }));
    expect(spy.mock.calls.length).toBeGreaterThan(before);
  });

  it("keeps the transcript collapsed until expanded", async () => {
    const user = userEvent.setup();
    renderWithClient(
      <SessionDetail sessionId="ses_1" onOpenApproval={() => {}} pollMs={0} />,
      client(),
    );
    await screen.findByRole("heading", { name: "Refactor UI" });

    const toggle = screen.getByRole("button", { name: "View full conversation" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("I've updated the header component.")).toBeNull();

    await user.click(toggle);
    expect(screen.getByRole("button", { name: "Hide full conversation" })).toBeInTheDocument();
    expect(screen.getByText("I've updated the header component.")).toBeInTheDocument();
  });

  it("surfaces explicit send status after sending a prompt", async () => {
    const user = userEvent.setup();
    renderWithClient(
      <SessionDetail sessionId="ses_1" onOpenApproval={() => {}} pollMs={0} />,
      client(),
    );
    await screen.findByRole("heading", { name: "Refactor UI" });

    await user.type(screen.getByLabelText("Message your agent"), "run the tests");
    await user.click(screen.getByRole("button", { name: "Send" }));

    // Never marked "Accepted by agent" merely because a packet was sent.
    expect(await screen.findByText("Received by desktop")).toBeInTheDocument();
    expect(screen.queryByText("Accepted by agent")).toBeNull();
  });

  it("asks for confirmation before stopping, then reflects the canceled state", async () => {
    const user = userEvent.setup();
    renderWithClient(
      <SessionDetail sessionId="ses_2" onOpenApproval={() => {}} pollMs={0} />,
      client(),
    );
    await screen.findByRole("heading", { name: "PDF import" });

    await user.click(screen.getByRole("button", { name: "Stop task" }));
    const dialog = await screen.findByRole("dialog", { name: "Stop this task?" });
    await user.click(within(dialog).getByRole("button", { name: "Stop task" }));

    expect(await screen.findByText("Canceled")).toBeInTheDocument();
  });

  it("does not offer retry after a transport-ambiguous send", async () => {
    const user = userEvent.setup();
    const c = client();
    vi.spyOn(c, "sendPrompt").mockRejectedValue(new RemoteError("transport", "timeout"));
    renderWithClient(<SessionDetail sessionId="ses_1" onOpenApproval={() => {}} pollMs={0} />, c);
    await screen.findByRole("heading", { name: "Refactor UI" });

    await user.type(screen.getByLabelText("Message your agent"), "hello");
    await user.click(screen.getByRole("button", { name: "Send" }));

    expect(await screen.findByText("Delivery unknown")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Tap to retry" })).toBeNull();
  });

  it("reports a session that is unavailable offline", async () => {
    renderWithClient(
      <SessionDetail sessionId="ses_1" onOpenApproval={() => {}} pollMs={0} />,
      new MockRemoteClient({ offline: true, now: () => FIXED_NOW }),
    );
    expect(await screen.findByText("Session unavailable")).toBeInTheDocument();
  });
});
