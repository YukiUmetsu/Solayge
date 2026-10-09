import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { ClientProvider, useClientContext } from "./ClientContext";
import { NotificationProvider } from "./NotificationCenter";
import { MockRemoteClient } from "./mock";
import { saveNotificationState } from "./notifications";

const NOW = 1_700_000_000;

/** Renders the number of loaded sessions so tests know the data arrived. */
function Probe() {
  const { data, status } = useClientContext();
  return (
    <span data-testid="probe">
      {status}:{data.sessions.length}
    </span>
  );
}

function renderCenter(client: MockRemoteClient) {
  return render(
    <ClientProvider
      client={client}
      desktopId={client.desktopId}
      desktops={[{ id: client.desktopId, name: "Mock Desktop", platform: "android" }]}
      now={() => NOW}
      refreshIntervalMs={0}
    >
      <NotificationProvider>
        <Probe />
      </NotificationProvider>
    </ClientProvider>,
  );
}

function enableNotifications() {
  localStorage.setItem(
    "solayge.notifications.settings.v1",
    JSON.stringify({
      enabled: true,
      onQuestions: true,
      onCompleted: true,
      onFailed: true,
      sound: false,
      volume: 0,
    }),
  );
}

describe("NotificationProvider", () => {
  it("shows a toast for events that arrived since the last known state", async () => {
    enableNotifications();
    // Pretend we already saw the current sessions but no approvals: the seeded
    // approvals are therefore new and should be announced.
    saveNotificationState("desk_mock", { approvals: [], sessions: {} });

    renderCenter(new MockRemoteClient({ now: () => NOW }));

    expect((await screen.findAllByText("Permission needed")).length).toBeGreaterThan(0);
    expect(screen.getByText("Agent has a question")).toBeInTheDocument();
  });

  it("stays quiet when notifications are disabled", async () => {
    saveNotificationState("desk_mock", { approvals: [], sessions: {} });
    renderCenter(new MockRemoteClient({ now: () => NOW }));

    expect(await screen.findByTestId("probe")).toHaveTextContent("online:5");
    await waitFor(() => expect(screen.queryByText("Permission needed")).toBeNull());
  });

  it("dismisses a toast on demand", async () => {
    enableNotifications();
    saveNotificationState("desk_mock", { approvals: [], sessions: {} });
    const user = userEvent.setup();

    renderCenter(new MockRemoteClient({ now: () => NOW }));

    const toast = (await screen.findByText("Agent has a question")).closest(".toast") as HTMLElement;
    await user.click(within(toast).getByRole("button", { name: "Dismiss notification" }));
    expect(screen.queryByText("Agent has a question")).toBeNull();
    // The other alerts are untouched.
    expect(screen.getAllByText("Permission needed").length).toBeGreaterThan(0);
  });
});
