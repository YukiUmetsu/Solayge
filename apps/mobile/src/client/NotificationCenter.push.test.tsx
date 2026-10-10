import { describe, expect, it, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";

import { ClientProvider, useClientContext } from "./ClientContext";
import { NotificationProvider } from "./NotificationCenter";
import { MockRemoteClient } from "./mock";

// Pretend the native shell produced an FCM token, so the registration effect
// actually runs (in jsdom `isTauri()` is false and the real bridge is a no-op).
vi.mock("./push", () => ({
  pushToken: vi.fn(async () => ({ token: "fcm-token", platform: "android" })),
  takePushTap: vi.fn(async () => null),
}));

const NOW = 1_700_000_000;

function Probe() {
  const { status } = useClientContext();
  return <span data-testid="status">{status}</span>;
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

describe("FCM token registration", () => {
  it("unregisters (never registers) while notifications are disabled", async () => {
    const client = new MockRemoteClient({ now: () => NOW });
    const register = vi.spyOn(client, "registerPush");
    const unregister = vi.spyOn(client, "unregisterPush");

    renderCenter(client);

    await waitFor(() => expect(unregister).toHaveBeenCalledWith("fcm-token"));
    expect(register).not.toHaveBeenCalled();
  });

  it("registers the token once notifications are enabled", async () => {
    enableNotifications();
    const client = new MockRemoteClient({ now: () => NOW });
    const register = vi.spyOn(client, "registerPush");
    const unregister = vi.spyOn(client, "unregisterPush");

    renderCenter(client);

    await waitFor(() =>
      expect(register).toHaveBeenCalledWith("fcm-token", "android", {
        on_questions: true,
        on_completed: true,
        on_failed: true,
      }),
    );
    expect(unregister).not.toHaveBeenCalled();
  });
});
