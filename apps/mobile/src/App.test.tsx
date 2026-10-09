import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import App from "./App";
import { MockRemoteClient } from "./client/mock";
import type { PairedDesktop, RemoteClient } from "./client/remote";

const NOW = 1_700_000_000;
const ONE_DESKTOP: PairedDesktop[] = [{ id: "desk_mock", name: "Mock Desktop", platform: "macos" }];

/** A stable, per-desktop mock so re-renders don't recreate the client. */
function factory(): (desktopId: string) => RemoteClient {
  const clients = new Map<string, MockRemoteClient>();
  return (desktopId: string) => {
    let client = clients.get(desktopId);
    if (!client) {
      client = new MockRemoteClient({ desktopId, now: () => NOW });
      clients.set(desktopId, client);
    }
    return client;
  };
}

describe("App shell", () => {
  it("renders the three tabs, defaults to Sessions, and shows the host switcher", async () => {
    const user = userEvent.setup();
    renderApp();

    expect(await screen.findByText("Refactor UI")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Sessions" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Inbox" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Projects" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Current desktop/ }));
    const dialog = screen.getByRole("dialog", { name: "Your desktops" });
    expect(within(dialog).getByText("Mock Desktop")).toBeInTheDocument();
  });

  it("navigates between destinations", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByText("Refactor UI");

    await user.click(screen.getByRole("tab", { name: "Inbox" }));
    expect(await screen.findByText("Run command?")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Projects" }));
    expect(await screen.findByRole("heading", { name: "Projects" })).toBeInTheDocument();
  });

  it("reaches Settings from the header and returns via a tab", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByText("Refactor UI");

    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByText("Connection method")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Sessions" }));
    expect(await screen.findByText("Refactor UI")).toBeInTheDocument();
  });

  it("shows a last-known read-only state and refuses to queue commands when offline", async () => {
    const user = userEvent.setup();
    renderApp();
    await screen.findByText("Refactor UI");

    // Go offline through the developer preview in Settings.
    await user.click(screen.getByRole("button", { name: "Settings" }));
    await screen.findByText("Connection method");
    await user.click(screen.getByRole("checkbox", { name: /Simulate offline/ }));

    // The banner explains the state and states the last-known timestamp.
    expect(await screen.findByText("Offline · read-only")).toBeInTheDocument();
    expect(screen.getByText(/Last updated/)).toBeInTheDocument();

    // Cached sessions remain readable…
    await user.click(screen.getByRole("tab", { name: "Sessions" }));
    expect(await screen.findByText("Refactor UI")).toBeInTheDocument();

    // …but the composer refuses to run commands.
    await user.click(screen.getByRole("button", { name: /Refactor UI/ }));
    expect(await screen.findByLabelText("Message your agent")).toBeDisabled();
    expect(
      screen.getByText(/offline. sending and stopping are disabled/i),
    ).toBeInTheDocument();
  });

  it("shows the first-launch selector when no desktop is paired", async () => {
    renderApp([]);
    expect(await screen.findByText("Connect to your computer")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Scan QR code" })).toBeInTheDocument();
  });
});

function renderApp(desktops: PairedDesktop[] = ONE_DESKTOP) {
  const clientFactory = factory();
  return render(<App clientFactory={clientFactory} desktops={desktops} />);
}
