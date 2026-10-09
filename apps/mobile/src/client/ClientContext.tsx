import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type {
  ApprovalSummary,
  HostMetadata,
  ProjectSummary,
  SessionSummary,
} from "@solayge/shared-types";

import {
  RemoteError,
  type PairedDesktop,
  type RemoteClient,
  type TransportMode,
} from "./remote";
import type { TaskCache } from "./cache";
import type { ConnectionStatus } from "./status";

export interface ClientData {
  projects: ProjectSummary[];
  sessions: SessionSummary[];
  approvals: ApprovalSummary[];
  updatedAt: number | null;
}

const EMPTY_DATA: ClientData = {
  projects: [],
  sessions: [],
  approvals: [],
  updatedAt: null,
};

export interface ClientContextValue {
  client: RemoteClient;
  desktopId: string;
  status: ConnectionStatus;
  desktop: HostMetadata | null;
  desktops: PairedDesktop[];
  transportMode: TransportMode;
  data: ClientData;
  loading: boolean;
  error: string | null;
  /** True when actions that mutate the host must be refused. */
  isReadOnly: boolean;
  now: () => number;
  refresh: () => Promise<void>;
  selectDesktop: (id: string) => void;
  setTransportMode: (mode: TransportMode) => void;
  addDesktop: () => void;
}

const ClientContext = createContext<ClientContextValue | null>(null);

/** Stable default clock; a fresh function each render would loop the refresh effect. */
const systemNow = () => Math.floor(Date.now() / 1000);

export interface ClientProviderProps {
  client: RemoteClient;
  desktopId: string;
  desktops?: PairedDesktop[];
  transportMode?: TransportMode;
  now?: () => number;
  onSelectDesktop?: (id: string) => void;
  onTransportModeChange?: (mode: TransportMode) => void;
  onAddDesktop?: () => void;
  /** Periodic refresh interval; 0 disables it (tests do this). */
  refreshIntervalMs?: number;
  /** On-device cache for the offline view. Omitted in tests (no caching). */
  cache?: TaskCache;
  children: ReactNode;
}

export function ClientProvider({
  client,
  desktopId,
  desktops,
  transportMode: transportModeProp,
  now = systemNow,
  onSelectDesktop,
  onTransportModeChange,
  onAddDesktop,
  refreshIntervalMs = 0,
  cache,
  children,
}: ClientProviderProps) {
  const [status, setStatus] = useState<ConnectionStatus>("connecting");
  const [desktop, setDesktop] = useState<HostMetadata | null>(null);
  const [data, setData] = useState<ClientData>(EMPTY_DATA);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [transportMode, setTransportModeState] = useState<TransportMode>(
    transportModeProp ?? "automatic",
  );

  // Guards against a slow refresh from a previous desktop overwriting newer
  // state after the user switches hosts.
  const refreshToken = useRef(0);

  const refresh = useCallback(async () => {
    const token = (refreshToken.current += 1);
    setLoading(true);
    setError(null);

    if (!client.isConnected() && client.connect) {
      // A transport may have dropped since the last refresh; try to re-establish
      // it before reporting offline so a transient loss recovers on its own.
      try {
        await client.connect();
      } catch {
        /* fall through to the offline state below */
      }
      if (token !== refreshToken.current) return;
    }

    if (!client.isConnected()) {
      if (token === refreshToken.current) {
        setStatus("offline");
        setError("The desktop is unreachable. Showing last known data.");
        setLoading(false);
      }
      return;
    }

    setStatus((current) => (current === "online" ? current : "connecting"));
    try {
      const [metadata, projects, sessions, approvals] = await Promise.all([
        client.hostMetadata(),
        client.listProjects(),
        client.listSessions(),
        client.listApprovals(),
      ]);
      if (token !== refreshToken.current) return;
      setDesktop(metadata);
      const next: ClientData = {
        projects,
        sessions,
        approvals,
        updatedAt: now(),
      };
      setData(next);
      cache?.write(desktopId, next, next.updatedAt ?? now());
      setStatus("online");
    } catch (caught) {
      if (token !== refreshToken.current) return;
      const code = caught instanceof RemoteError ? caught.code : "internal";
      const message = caught instanceof Error ? caught.message : String(caught);
      if (code === "not_authorized") {
        // A revoked device and a device missing a scope share the same wire
        // code; only the first is terminal, so tell them apart by message.
        if (/revo|no longer authorized/i.test(message)) {
          setStatus("revoked");
          setError("This phone's access was revoked on the desktop.");
        } else {
          setStatus("error");
          setError(message);
        }
      } else if (code === "transport") {
        setStatus("offline");
        setError(message);
      } else {
        setStatus("error");
        setError(message);
      }
    } finally {
      if (token === refreshToken.current) setLoading(false);
    }
  }, [client, now, cache, desktopId]);

  // Host-scoped data must never survive a desktop switch. Otherwise the
  // previous desktop's sessions and approvals would remain visible — and
  // actionable — under the newly selected desktop until its reads complete,
  // which could send a decision to the wrong host. Reset to the new host's
  // cached (read-only) view, or to empty when there is no cache.
  useEffect(() => {
    const cached = cache?.read(desktopId, now());
    setData(
      cached
        ? {
            projects: cached.projects,
            sessions: cached.sessions,
            approvals: cached.approvals,
            updatedAt: cached.savedAt,
          }
        : EMPTY_DATA,
    );
    setDesktop(null);
    setStatus("connecting");
    setError(null);
  }, [client, desktopId, now, cache]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (refreshIntervalMs <= 0) return undefined;
    const id = setInterval(() => void refresh(), refreshIntervalMs);
    return () => clearInterval(id);
  }, [refresh, refreshIntervalMs]);

  useEffect(() => {
    if (transportModeProp !== undefined) setTransportModeState(transportModeProp);
  }, [transportModeProp]);

  const setTransportMode = useCallback(
    (mode: TransportMode) => {
      setTransportModeState(mode);
      onTransportModeChange?.(mode);
    },
    [onTransportModeChange],
  );

  const selectDesktop = useCallback(
    (id: string) => {
      onSelectDesktop?.(id);
    },
    [onSelectDesktop],
  );

  const addDesktop = useCallback(() => {
    onAddDesktop?.();
  }, [onAddDesktop]);

  const value = useMemo<ClientContextValue>(() => {
    const resolvedDesktops =
      desktops !== undefined
        ? desktops
        : [{ id: desktopId, name: desktop?.name ?? "Desktop", platform: desktop?.platform ?? "" }];
    return {
      client,
      desktopId,
      status,
      desktop,
      desktops: resolvedDesktops,
      transportMode,
      data,
      loading,
      error,
      isReadOnly: status !== "online",
      now,
      refresh,
      selectDesktop,
      setTransportMode,
      addDesktop,
    };
  }, [
    client,
    desktopId,
    status,
    desktop,
    desktops,
    transportMode,
    data,
    loading,
    error,
    now,
    refresh,
    selectDesktop,
    setTransportMode,
    addDesktop,
  ]);

  return <ClientContext.Provider value={value}>{children}</ClientContext.Provider>;
}

export function useClientContext(): ClientContextValue {
  const value = useContext(ClientContext);
  if (!value) throw new Error("useClientContext must be used within a ClientProvider");
  return value;
}

export function useClient(): RemoteClient {
  return useClientContext().client;
}

export function useNow(): () => number {
  return useClientContext().now;
}
