import { render } from "@testing-library/react";
import type { ReactElement } from "react";

import { ClientProvider } from "../client/ClientContext";
import { NotificationProvider } from "../client/NotificationCenter";
import type { PairedDesktop, RemoteClient, TransportMode } from "../client/remote";

/** A fixed clock so relative-time text is deterministic across tests. */
export const FIXED_NOW = 1_700_000_000;

export interface RenderOptions {
  desktopId?: string;
  desktops?: PairedDesktop[];
  transportMode?: TransportMode;
  onSelectDesktop?: (id: string) => void;
  onTransportModeChange?: (mode: TransportMode) => void;
  onAddDesktop?: () => void;
  now?: () => number;
  refreshIntervalMs?: number;
}

/**
 * Render a screen with a client injected, so no test ever touches the network
 * or a native capability.
 */
export function renderWithClient(ui: ReactElement, client: RemoteClient, options: RenderOptions = {}) {
  const desktopId = options.desktopId ?? client.desktopId;
  return render(
    <ClientProvider
      client={client}
      desktopId={desktopId}
      desktops={options.desktops ?? [{ id: desktopId, name: "Mock Desktop", platform: "macos" }]}
      transportMode={options.transportMode ?? "automatic"}
      now={options.now ?? (() => FIXED_NOW)}
      onSelectDesktop={options.onSelectDesktop}
      onTransportModeChange={options.onTransportModeChange}
      onAddDesktop={options.onAddDesktop}
      refreshIntervalMs={options.refreshIntervalMs ?? 0}
    >
      <NotificationProvider>{ui}</NotificationProvider>
    </ClientProvider>,
  );
}
