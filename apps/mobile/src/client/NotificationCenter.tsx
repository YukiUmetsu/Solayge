import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { ToastViewport } from "../components/ToastViewport";
import { useClientContext } from "./ClientContext";
import { pushToken, takePushTap } from "./push";
import {
  allows,
  diffNotifications,
  ensureNotificationChannel,
  loadNotificationSettings,
  loadNotificationState,
  notificationPermission,
  playTone,
  requestNotificationPermission,
  saveNotificationSettings,
  saveNotificationState,
  sendTestNotification,
  syncSettingsToRust,
  type NotificationKind,
  type NotificationPermission,
  type NotificationSettings,
} from "./notifications";

export interface Toast {
  id: number;
  kind: NotificationKind;
  title: string;
  body: string;
}

export interface NotificationContextValue {
  settings: NotificationSettings;
  permission: NotificationPermission;
  /** Whether the platform can show OS notifications at all. */
  supported: boolean;
  updateSettings: (patch: Partial<NotificationSettings>) => void;
  requestPermission: () => Promise<NotificationPermission>;
  sendTest: () => Promise<boolean>;
  toasts: Toast[];
  dismissToast: (id: number) => void;
}

const NotificationContext = createContext<NotificationContextValue | null>(null);

const MAX_TOASTS = 3;
const TOAST_TTL_MS = 8000;

/**
 * Watches refreshed data and turns new agent questions, completions, and
 * failures into in-app toasts. The OS notifications for the same events are
 * produced by the Rust background monitor, so a throttled webview never hides
 * an alert.
 */
export function NotificationProvider({ children }: { children: ReactNode }) {
  const { client, data, desktopId, status } = useClientContext();
  const [settings, setSettings] = useState<NotificationSettings>(() =>
    loadNotificationSettings(),
  );
  const [permission, setPermission] = useState<NotificationPermission>("prompt");
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextToastId = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const updateSettings = useCallback(
    (patch: Partial<NotificationSettings>) => {
      setSettings((current) => {
        const next = { ...current, ...patch };
        saveNotificationSettings(next);
        void syncSettingsToRust(next);
        void ensureNotificationChannel(next);
        return next;
      });
    },
    [],
  );

  const requestPermission = useCallback(async () => {
    const result = await requestNotificationPermission();
    setPermission(result);
    return result;
  }, []);

  const sendTest = useCallback(async () => {
    const result = await sendTestNotification(settings);
    if (result) setPermission("granted");
    return result;
  }, [settings]);

  // Push the stored preferences to the background monitor and read the current
  // permission once the provider mounts.
  useEffect(() => {
    void syncSettingsToRust(loadNotificationSettings());
    void ensureNotificationChannel(loadNotificationSettings());
    void notificationPermission().then((result) => {
      if (mounted.current) setPermission(result);
    });
  }, []);

  // Detect new events on each fresh online snapshot. The first snapshot for a
  // desktop is recorded as a baseline, never announced.
  useEffect(() => {
    if (!desktopId || status !== "online" || data.updatedAt == null) return;
    const previous = loadNotificationState(desktopId);
    const { events, state } = diffNotifications(previous, data.sessions, data.approvals);
    saveNotificationState(desktopId, state);
    if (!previous) return;

    const allowed = events.filter((event) => allows(settings, event.kind));
    if (allowed.length === 0) return;
    if (settings.sound) {
      for (const event of allowed) playTone(event.kind, settings.volume);
    }
    setToasts((current) => {
      const added = allowed.map((event) => ({
        id: (nextToastId.current += 1),
        kind: event.kind,
        title: event.title,
        body: event.body,
      }));
      const next = [...current, ...added];
      const trimmed = next.slice(-MAX_TOASTS);
      for (const toast of added) {
        window.setTimeout(() => {
          if (mounted.current) dismissToast(toast.id);
        }, TOAST_TTL_MS);
      }
      return trimmed;
    });
  }, [data, desktopId, status, settings, dismissToast]);

  // Register this device's FCM token with the desktop so background alerts can
  // reach the phone even when the app is not running. Registration is idempotent
  // and is retried whenever the app returns to the foreground, which also picks
  // up a token Firebase rotated while the app was backgrounded.
  //
  // The master switch is authoritative for the push path too: while
  // notifications are disabled the token is explicitly *unregistered*, so the
  // desktop stops sending background pushes to a user who never opted in (or
  // turned them back off). The desktop cannot read the phone's per-kind
  // preferences, so enabling notifications re-registers and lets the shared
  // preference file gate the local path.
  const syncPushRegistration = useCallback(async () => {
    const token = await pushToken();
    if (!token) return;
    try {
      if (settings.enabled) {
        await client.registerPush(token.token, token.platform);
      } else {
        await client.unregisterPush(token.token);
      }
    } catch {
      // The desktop may have dropped; the next refresh or foreground retries.
    }
  }, [client, settings.enabled]);

  useEffect(() => {
    if (status !== "online" || !desktopId) return undefined;
    void syncPushRegistration();
    return undefined;
  }, [desktopId, syncPushRegistration, status]);

  // A notification tap deep-links to the session/approval it names. Checked on
  // mount and whenever the app returns to the foreground.
  useEffect(() => {
    const check = async () => {
      const tap = await takePushTap();
      if (tap) {
        window.dispatchEvent(new CustomEvent("solayge:open", { detail: tap }));
      }
    };
    const refresh = () => {
      void check();
      void syncPushRegistration();
    };
    void check();
    const onVisible = () => {
      if (document.visibilityState === "visible") refresh();
    };
    const onFocus = () => refresh();
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onFocus);
    return () => {
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onFocus);
    };
  }, [syncPushRegistration]);

  const value: NotificationContextValue = {
    settings,
    permission,
    supported: permission !== "unsupported",
    updateSettings,
    requestPermission,
    sendTest,
    toasts,
    dismissToast,
  };

  return (
    <NotificationContext.Provider value={value}>
      {children}
      <ToastViewport toasts={toasts} onDismiss={dismissToast} />
    </NotificationContext.Provider>
  );
}

export function useNotifications(): NotificationContextValue {
  const value = useContext(NotificationContext);
  if (!value) throw new Error("useNotifications must be used within a NotificationProvider");
  return value;
}
