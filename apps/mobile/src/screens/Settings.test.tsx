import { describe, expect, it, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { MockRemoteClient } from "../client/mock";
import { FIXED_NOW, renderWithClient } from "../test/render";
import { Settings } from "./Settings";

function renderSettings(overrides: { onSimulateOffline?: (offline: boolean) => void } = {}) {
  return renderWithClient(
    <Settings onAddDesktop={() => {}} {...overrides} />,
    new MockRemoteClient({ now: () => FIXED_NOW }),
  );
}

describe("Settings", () => {
  it("offers the three transport modes and switches between them", async () => {
    const user = userEvent.setup();
    renderSettings();

    const automatic = screen.getByRole("radio", { name: /Automatic/ });
    expect(automatic).toBeChecked();
    expect(screen.getByText(/Uses Iroh first/i)).toBeInTheDocument();

    await user.click(screen.getByRole("radio", { name: /Iroh only/ }));
    expect(screen.getByRole("radio", { name: /Iroh only/ })).toBeChecked();
    expect(screen.getByRole("radio", { name: /Automatic/ })).not.toBeChecked();
    expect(screen.getByRole("radio", { name: /Tailscale only/ })).toBeInTheDocument();
  });

  it("exposes sanitized connection details and diagnostics", async () => {
    renderSettings();
    expect(await screen.findByText("Connection details")).toBeInTheDocument();
    expect(screen.getByText("desk_mock")).toBeInTheDocument();

    // Diagnostics load once the connection reports online.
    expect(await screen.findByText(/transport: loopback/)).toBeInTheDocument();
    const diagnostics = screen.getByLabelText("Diagnostic details");
    expect(diagnostics.textContent).not.toMatch(/private|secret|key/i);
  });

  it("starts the add-desktop flow from the desktops section", async () => {
    const user = userEvent.setup();
    const onAddDesktop = vi.fn();
    renderWithClient(
      <Settings onAddDesktop={onAddDesktop} />,
      new MockRemoteClient({ now: () => FIXED_NOW }),
    );
    await screen.findByText("Connection details");

    await user.click(screen.getByRole("button", { name: "+ Add a desktop" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a desktop" });
    await user.click(within(dialog).getByRole("button", { name: "Continue to pairing" }));
    expect(onAddDesktop).toHaveBeenCalled();
  });

  it("can preview the offline read-only state", async () => {
    const user = userEvent.setup();
    const onSimulateOffline = vi.fn();
    renderSettings({ onSimulateOffline });

    await user.click(screen.getByRole("checkbox", { name: /Simulate offline/ }));
    expect(onSimulateOffline).toHaveBeenCalledWith(true);
  });
});
