import { useState } from "react";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { SessionSummary } from "@solayge/shared-types";

import { ClientProvider, useClientContext } from "./ClientContext";
import { MockRemoteClient } from "./mock";
import type { RemoteClient } from "./remote";

const FIXED_NOW = 1_700_000_000;

/** A mock whose reads hang until released, to expose the switch window. */
class GatedMock extends MockRemoteClient {
  private gate: Promise<void> | null = null;
  private releaseGate: (() => void) | null = null;

  hold(): void {
    this.gate = new Promise<void>((resolve) => {
      this.releaseGate = resolve;
    });
  }

  release(): void {
    this.releaseGate?.();
  }

  async listSessions(): Promise<SessionSummary[]> {
    if (this.gate) await this.gate;
    return super.listSessions();
  }
}

function Probe() {
  const { data, status, isReadOnly } = useClientContext();
  return (
    <div>
      <span data-testid="status">{status}</span>
      <span data-testid="readonly">{String(isReadOnly)}</span>
      <span data-testid="sessions">{data.sessions.map((session) => session.id).join(",")}</span>
    </div>
  );
}

function Harness({ clients }: { clients: Record<string, RemoteClient> }) {
  const [desktopId, setDesktopId] = useState("desk_a");
  return (
    <ClientProvider
      client={clients[desktopId]}
      desktopId={desktopId}
      now={() => FIXED_NOW}
      onSelectDesktop={setDesktopId}
    >
      <Probe />
      <button type="button" onClick={() => setDesktopId("desk_b")}>
        switch
      </button>
    </ClientProvider>
  );
}

describe("ClientProvider host isolation", () => {
  it("drops the previous desktop's data the moment the selected host changes", async () => {
    const user = userEvent.setup();
    const gatedB = new GatedMock({ desktopId: "desk_b", empty: true, now: () => FIXED_NOW });
    const clients: Record<string, RemoteClient> = {
      desk_a: new MockRemoteClient({ desktopId: "desk_a", now: () => FIXED_NOW }),
      desk_b: gatedB,
    };

    render(<Harness clients={clients} />);

    // The first desktop loads its sessions.
    expect(await screen.findByTestId("sessions")).toHaveTextContent("ses_1");

    // Switch while the new host is still unreachable.
    gatedB.hold();
    await user.click(screen.getByRole("button", { name: "switch" }));

    // The old host's sessions must not linger under the new desktop, and
    // mutations must be disabled until the new host answers.
    await waitFor(() => expect(screen.getByTestId("sessions")).toHaveTextContent(""));
    expect(screen.getByTestId("status")).toHaveTextContent("connecting");
    expect(screen.getByTestId("readonly")).toHaveTextContent("true");

    // Let the background read finish so no state update lands after the test.
    gatedB.release();
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("online"));
  });
});
