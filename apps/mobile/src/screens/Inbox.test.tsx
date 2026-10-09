import { useState } from "react";
import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MockRemoteClient } from "../client/mock";
import type { InboxFilter } from "../client/selectors";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { Inbox } from "./Inbox";

function Harness() {
  const [filter, setFilter] = useState<InboxFilter>("decision");
  return <Inbox filter={filter} onFilter={setFilter} onOpenApproval={() => {}} />;
}

describe("Inbox", () => {
  it("filters permission decisions and questions, labeling each kind", async () => {
    const user = userEvent.setup();
    renderWithClient(<Harness />, new MockRemoteClient({ now: () => FIXED_NOW }));

    // Decisions filter is the default.
    expect(await screen.findByText("Run command?")).toBeInTheDocument();
    expect(screen.getByText("Delete generated files?")).toBeInTheDocument();
    expect(screen.getAllByText("Permission request").length).toBe(2);
    expect(screen.queryByText("Which navigation library should I use?")).toBeNull();

    await user.click(screen.getByRole("button", { name: /Questions/ }));
    expect(await screen.findByText("Which navigation library should I use?")).toBeInTheDocument();
    expect(screen.getByText("Question")).toBeInTheDocument();
    expect(screen.queryByText("Run command?")).toBeNull();
  });

  it("shows distinct empty states per filter", async () => {
    const user = userEvent.setup();
    renderWithClient(<Harness />, new MockRemoteClient({ empty: true, now: () => FIXED_NOW }));

    expect(await screen.findByText("Nothing needs a decision")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Questions/ }));
    expect(await screen.findByText("No open questions")).toBeInTheDocument();
  });

  it("does not claim the inbox is empty when the desktop is unreachable", async () => {
    renderWithClient(<Harness />, new MockRemoteClient({ offline: true, now: () => FIXED_NOW }));

    expect(await screen.findByText("Inbox unavailable offline")).toBeInTheDocument();
    expect(screen.queryByText("Nothing needs a decision")).toBeNull();
  });
});
