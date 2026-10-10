import { useCallback, useEffect, useMemo, useState } from "react";

import { AddDesktopFlow } from "./AddDesktopFlow";
import { ClientProvider, useClientContext } from "./client/ClientContext";
import { NotificationProvider } from "./client/NotificationCenter";
import { createLocalStorageCache } from "./client/cache";
import { MockRemoteClient } from "./client/mock";
import type { PairedDesktop, RemoteClient, TransportMode } from "./client/remote";
import { TauriRemoteClient } from "./client/tauri";
import { AppHeader } from "./components/AppHeader";
import { HostSwitcher } from "./components/HostSwitcher";
import { NotificationPrompt } from "./components/NotificationPrompt";
import { OfflineBanner } from "./components/OfflineBanner";
import { PullToRefresh } from "./components/PullToRefresh";
import { isRootRoute, routeTab, type Route } from "./navigation";
import { pairingApi, type PairedDesktopRecord } from "./pairing";
import { ApprovalDetail } from "./screens/ApprovalDetail";
import { DesktopSelector } from "./screens/DesktopSelector";
import { Inbox } from "./screens/Inbox";
import { ProjectDetail } from "./screens/ProjectDetail";
import { Projects } from "./screens/Projects";
import { SessionDetail } from "./screens/SessionDetail";
import { SessionsHome } from "./screens/SessionsHome";
import { Settings } from "./screens/Settings";
import { TABS, type TabId } from "./tabs";

/** The production client: a real, authenticated Iroh connection per desktop. */
function defaultClientFactory(desktopId: string): RemoteClient {
  return new TauriRemoteClient(desktopId);
}

export interface AppProps {
  clientFactory?: (desktopId: string) => RemoteClient;
  desktops?: PairedDesktop[];
  initialTransportMode?: TransportMode;
}

export default function App({
  clientFactory = defaultClientFactory,
  desktops: desktopsProp,
  initialTransportMode = "automatic",
}: AppProps = {}) {
  // Tests pass `desktops`/`clientFactory` explicitly; the shipped app loads its
  // paired desktops from the backend and never seeds a placeholder host.
  const controlled = desktopsProp !== undefined;
  const [desktops, setDesktops] = useState<PairedDesktop[]>(desktopsProp ?? []);
  const [loadingDesktops, setLoadingDesktops] = useState(!controlled);
  const [desktopId, setDesktopId] = useState(desktopsProp?.[0]?.id ?? "");
  const [route, setRoute] = useState<Route>({ name: "sessions" });
  const [transportMode, setTransportMode] = useState<TransportMode>(initialTransportMode);
  const [showAddDesktop, setShowAddDesktop] = useState(false);

  const reloadDesktops = useCallback(async () => {
    try {
      const records = await pairingApi.pairedDesktops();
      const next = records.map((r) => ({ id: r.id, name: r.name, platform: r.platform }));
      setDesktops(next);
      setDesktopId((prev) => prev || next[0]?.id || "");
    } catch {
      setDesktops([]);
    } finally {
      setLoadingDesktops(false);
    }
  }, []);

  useEffect(() => {
    if (controlled) return;
    void reloadDesktops();
  }, [controlled, reloadDesktops]);

  const client = useMemo<RemoteClient>(
    () => clientFactory(desktopId),
    [clientFactory, desktopId],
  );

  // The shipped app caches task data for offline use; tests (which pass their
  // own desktops) opt out so cache state never leaks between cases.
  const cache = useMemo(
    () => (controlled ? undefined : createLocalStorageCache()),
    [controlled],
  );

  const handlePaired = useCallback(
    (record: PairedDesktopRecord) => {
      setDesktops((prev) => [
        ...prev.filter((d) => d.id !== record.id),
        { id: record.id, name: record.name, platform: record.platform },
      ]);
      setDesktopId(record.id);
      void reloadDesktops();
    },
    [reloadDesktops],
  );

  const addDesktop = useCallback(() => setShowAddDesktop(true), []);

  if (loadingDesktops) {
    return (
      <div className="app">
        <main className="app__content" />
      </div>
    );
  }

  return (
    <ClientProvider
      client={client}
      desktopId={desktopId}
      desktops={desktops}
      transportMode={transportMode}
      onTransportModeChange={setTransportMode}
      onSelectDesktop={setDesktopId}
      onAddDesktop={addDesktop}
      refreshIntervalMs={30_000}
      cache={cache}
    >
      <NotificationProvider>
        {showAddDesktop ? (
          <div className="app">
            <main className="app__content">
              <AddDesktopFlow
                onClose={() => setShowAddDesktop(false)}
                onPaired={handlePaired}
              />
            </main>
          </div>
        ) : (
          <AppShell route={route} setRoute={setRoute} onAddDesktop={addDesktop} onShowAddDesktop={addDesktop} />
        )}
      </NotificationProvider>
    </ClientProvider>
  );
}

function AppShell({
  route,
  setRoute,
  onAddDesktop,
  onShowAddDesktop,
}: {
  route: Route;
  setRoute: (route: Route) => void;
  onAddDesktop: () => void;
  onShowAddDesktop: () => void;
}) {
  const { client, data, desktops, desktopId, selectDesktop, refresh, status } =
    useClientContext();
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [simulateOffline, setSimulateOffline] = useState(false);
  const [, setHistory] = useState<Route[]>([]);

  const navigate = useCallback(
    (next: Route) => {
      setHistory((stack) => [...stack, route]);
      setRoute(next);
    },
    [route, setRoute],
  );

  // A push notification tap opens the session or approval it names, switching
  // desktop first when the alert came from a different one.
  useEffect(() => {
    const handler = (event: Event) => {
      const tap = (event as CustomEvent).detail as
        | { desktop_id?: string; session_id?: string; approval_id?: string }
        | undefined;
      if (!tap) return;
      if (tap.desktop_id && tap.desktop_id !== desktopId) selectDesktop(tap.desktop_id);
      if (tap.approval_id) navigate({ name: "approval", approvalId: tap.approval_id });
      else if (tap.session_id) navigate({ name: "session", sessionId: tap.session_id });
    };
    window.addEventListener("solayge:open", handler);
    return () => window.removeEventListener("solayge:open", handler);
  }, [navigate, desktopId, selectDesktop]);

  const goToTab = useCallback(
    (tab: TabId) => {
      setHistory([]);
      if (tab === "inbox") setRoute({ name: "inbox", filter: "decision" });
      else if (tab === "projects") setRoute({ name: "projects" });
      else setRoute({ name: "sessions" });
    },
    [setRoute],
  );

  const back = useCallback(() => {
    setHistory((stack) => {
      if (stack.length === 0) {
        setRoute({ name: "sessions" });
        return stack;
      }
      setRoute(stack[stack.length - 1]);
      return stack.slice(0, -1);
    });
  }, [setRoute]);

  const simulate = useCallback(
    (offline: boolean) => {
      if (client instanceof MockRemoteClient) client.setOffline(offline);
      setSimulateOffline(offline);
      void refresh();
    },
    [client, refresh],
  );

  // A pull-to-refresh reloads the global snapshot and tells the active screen
  // to reload its own detail (a session's events, a project's changed files),
  // which the global refresh does not cover.
  const refreshAll = useCallback(async () => {
    await refresh();
    window.dispatchEvent(new CustomEvent("solayge:refresh"));
  }, [refresh]);

  const title = useMemo(() => {
    switch (route.name) {
      case "session":
        return data.sessions.find((session) => session.id === route.sessionId)?.title ?? "Session";
      case "approval":
        return "Request";
      case "project":
        return data.projects.find((project) => project.id === route.projectId)?.name ?? "Project";
      case "settings":
        return "Settings";
      default:
        return undefined;
    }
  }, [route, data.sessions, data.projects]);

  const activeTab = routeTab(route);

  function renderRoute() {
    switch (route.name) {
      case "sessions":
        return <SessionsHome onOpenSession={(id) => navigate({ name: "session", sessionId: id })} />;
      case "session":
        return (
          <SessionDetail
            sessionId={route.sessionId}
            onOpenApproval={(id) => navigate({ name: "approval", approvalId: id })}
          />
        );
      case "inbox":
        return (
          <Inbox
            filter={route.filter}
            onFilter={(filter) => setRoute({ name: "inbox", filter })}
            onOpenApproval={(id) => navigate({ name: "approval", approvalId: id })}
          />
        );
      case "approval":
        return <ApprovalDetail approvalId={route.approvalId} />;
      case "projects":
        return <Projects onOpenProject={(id) => navigate({ name: "project", projectId: id })} />;
      case "project":
        return (
          <ProjectDetail
            projectId={route.projectId}
            onOpenSession={(id) => navigate({ name: "session", sessionId: id })}
          />
        );
      case "settings":
        return (
          <Settings
            onAddDesktop={onShowAddDesktop}
            simulateOffline={simulateOffline}
            onSimulateOffline={client instanceof MockRemoteClient ? simulate : undefined}
          />
        );
    }
  }

  if (desktops.length === 0) {
    return (
      <div className="app">
        <main className="app__content">
          <DesktopSelector onAddDesktop={onAddDesktop} />
        </main>
      </div>
    );
  }

  return (
    <>
      <div className="app">
        <AppHeader
          title={title}
          onBack={isRootRoute(route) ? undefined : back}
          onOpenSettings={() => navigate({ name: "settings" })}
          onOpenSwitcher={() => setSwitcherOpen(true)}
        />
        <main className="app__content">
          <PullToRefresh onRefresh={refreshAll} disabled={status === "revoked"}>
            <NotificationPrompt />
            <OfflineBanner />
            <div className="screen__body app__screen">{renderRoute()}</div>
          </PullToRefresh>
        </main>
        <nav className="tabbar" role="tablist" aria-label="Primary">
          {TABS.map((tab) => {
            const selected = tab.id === activeTab;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={selected}
                className={selected ? "tabbar__item tabbar__item--active" : "tabbar__item"}
                onClick={() => goToTab(tab.id)}
              >
                <span className="tabbar__icon" aria-hidden="true">
                  {tab.icon}
                </span>
                <span className="tabbar__label">{tab.label}</span>
              </button>
            );
          })}
        </nav>
      </div>
      <HostSwitcher
        open={switcherOpen}
        onClose={() => setSwitcherOpen(false)}
        onAddDesktop={() => {
          setSwitcherOpen(false);
          onShowAddDesktop();
        }}
      />
    </>
  );
}
