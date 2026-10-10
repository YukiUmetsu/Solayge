import { describe, expect, it, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MockRemoteClient } from "../client/mock";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { SessionsHome } from "./SessionsHome";

describe("Sessions home", () => {
  it("groups needs-input, running, and recent, with full row facts", async () => {
    renderWithClient(
      <SessionsHome onOpenSession={() => {}} />,
      new MockRemoteClient({ now: () => FIXED_NOW }),
    );

    expect(await screen.findByText("Refactor UI")).toBeInTheDocument();

    const needsInput = screen.getByRole("region", { name: "Needs your input" });
    const running = screen.getByRole("region", { name: "Running" });
    const recent = screen.getByRole("region", { name: "Recent" });

    // Refactor UI is running with a pending decision, so it leads.
    expect(within(needsInput).getByText("Refactor UI")).toBeInTheDocument();
    expect(within(needsInput).getByText("Claude Code")).toBeInTheDocument();
    expect(within(needsInput).getAllByText("Waiting for your answer").length).toBeGreaterThan(0);

    // Active work without a pending decision.
    expect(within(running).getByText("PDF import")).toBeInTheDocument();
    expect(within(recent).getByText("Add dark mode")).toBeInTheDocument();
    expect(within(recent).getByText("Completed")).toBeInTheDocument();

    // Pending count is exposed numerically, not only as color.
    expect(screen.getAllByLabelText("1 pending").length).toBeGreaterThan(0);
  });

  it("shows an empty state with no sessions", async () => {
    renderWithClient(
      <SessionsHome onOpenSession={() => {}} />,
      new MockRemoteClient({ empty: true, now: () => FIXED_NOW }),
    );
    expect(await screen.findByText("No sessions yet")).toBeInTheDocument();
  });

  it("shows draft tasks under their own group", async () => {
    renderWithClient(
      <SessionsHome onOpenSession={() => {}} />,
      new MockRemoteClient({
        now: () => FIXED_NOW,
        drafts: [{ title: "Plan the migration" }],
      }),
    );
    expect(await screen.findByText("Plan the migration")).toBeInTheDocument();
    const drafts = screen.getByRole("region", { name: "Drafts" });
    expect(within(drafts).getByText("Plan the migration")).toBeInTheDocument();
    expect(within(drafts).getByText("Draft")).toBeInTheDocument();
  });

  it("reloads all task data from the desktop", async () => {
    const user = userEvent.setup();
    const c = new MockRemoteClient({ now: () => FIXED_NOW });
    const spy = vi.spyOn(c, "listSessions");
    renderWithClient(<SessionsHome onOpenSession={() => {}} />, c);
    await screen.findByText("Refactor UI");

    const before = spy.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Reload all tasks from the desktop" }));
    expect(spy.mock.calls.length).toBeGreaterThan(before);
  });

  it("shows an error state when the desktop is unreachable and nothing is cached", async () => {
    renderWithClient(
      <SessionsHome onOpenSession={() => {}} />,
      new MockRemoteClient({ offline: true, now: () => FIXED_NOW }),
    );
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });
});
