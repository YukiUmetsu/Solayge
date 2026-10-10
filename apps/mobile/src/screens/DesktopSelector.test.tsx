import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MockRemoteClient } from "../client/mock";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { DesktopSelector } from "./DesktopSelector";

describe("Desktop selector", () => {
  it("explains the requirement and offers QR pairing when no desktop is paired", async () => {
    const user = userEvent.setup();
    renderWithClient(
      <DesktopSelector onAddDesktop={() => {}} />,
      new MockRemoteClient({ now: () => FIXED_NOW }),
      { desktops: [] },
    );

    expect(await screen.findByText("Connect to your computer")).toBeInTheDocument();
    expect(screen.getByText(/must be running Solayge/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Scan QR code" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "How it works" }));
    expect(screen.getByRole("dialog", { name: "How pairing works" })).toBeInTheDocument();
    expect(screen.getByText(/approve this phone on the desktop/i)).toBeInTheDocument();
  });

  it("lists paired desktops for selection", async () => {
    renderWithClient(
      <DesktopSelector onAddDesktop={() => {}} />,
      new MockRemoteClient({ now: () => FIXED_NOW }),
      {
        desktops: [
          { id: "desk_a", name: "Studio Mac", platform: "macos" },
          { id: "desk_b", name: "ThinkPad", platform: "linux" },
        ],
      },
    );
    expect(await screen.findByText("Your desktops")).toBeInTheDocument();
    expect(screen.getByText("Studio Mac")).toBeInTheDocument();
    expect(screen.getByText("ThinkPad")).toBeInTheDocument();
  });
});
