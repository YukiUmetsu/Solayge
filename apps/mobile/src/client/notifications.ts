/**
 * Notification preferences, permission, and event detection.
 *
 * The OS notifications themselves are produced by the Rust background monitor,
 * which keeps polling a connected desktop even while the webview is throttled.
 * This module owns the user's preferences (shared with Rust), asks for the
 * platform permission, and turns a data refresh into the small set of events a
 * person should be told about. The UI layer renders those as in-app toasts and
 * the Settings screen edits the preferences.
 */
import type { ApprovalSummary, SessionSummary } from "@solayge/shared-types";

export interface NotificationSettings {
  /** Master switch. Off until the user opts in. */
  enabled: boolean;
  /** Agent questions and permission requests that need a decision. */
  onQuestions: boolean;
  /** Tasks that finished successfully. */
  onCompleted: boolean;
  /** Tasks that failed or errored. */
  onFailed: boolean;
  /** Whether the OS notification plays a sound. */
  sound: boolean;
  /** Volume (0..1) for the in-app chime played alongside the alert. */
  volume: number;
}

export const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  enabled: false,
  onQuestions: true,
  onCompleted: true,
  onFailed: true,
  sound: true,
  volume: 0.8,
};

export type NotificationPermission = "granted" | "denied" | "prompt" | "unsupported";

export type NotificationKind = "question" | "completed" | "failed";

export interface NotificationEvent {
  kind: NotificationKind;
  title: string;
  body: string;
}

/** What we have already told the user about, per desktop. */
export interface NotificationState {
  approvals: string[];
  sessions: Record<string, string>;
}

const SETTINGS_KEY = "solayge.notifications.settings.v1";
const STATE_PREFIX = "solayge.notifications.state.v1.";

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** Whether the app is running inside the Tauri shell (and can use native APIs). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Load preferences, defensively coercing anything malformed back to defaults. */
export function loadNotificationSettings(): NotificationSettings {
  const store = storage();
  if (!store) return { ...DEFAULT_NOTIFICATION_SETTINGS };
  try {
    const raw = store.getItem(SETTINGS_KEY);
    if (!raw) return { ...DEFAULT_NOTIFICATION_SETTINGS };
    const parsed = JSON.parse(raw) as Partial<NotificationSettings>;
    return {
      enabled: typeof parsed.enabled === "boolean" ? parsed.enabled : false,
      onQuestions:
        typeof parsed.onQuestions === "boolean" ? parsed.onQuestions : true,
      onCompleted:
        typeof parsed.onCompleted === "boolean" ? parsed.onCompleted : true,
      onFailed: typeof parsed.onFailed === "boolean" ? parsed.onFailed : true,
      sound: typeof parsed.sound === "boolean" ? parsed.sound : true,
      volume:
        typeof parsed.volume === "number" && Number.isFinite(parsed.volume)
          ? Math.min(1, Math.max(0, parsed.volume))
          : 0.8,
    };
  } catch {
    return { ...DEFAULT_NOTIFICATION_SETTINGS };
  }
}

export function saveNotificationSettings(settings: NotificationSettings): void {
  const store = storage();
  if (!store) return;
  try {
    store.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    // Best effort; the in-memory value still applies.
  }
}

/** The preferences in the snake_case shape the Rust command expects. */
function toRust(settings: NotificationSettings) {
  return {
    enabled: settings.enabled,
    on_questions: settings.onQuestions,
    on_completed: settings.onCompleted,
    on_failed: settings.onFailed,
    sound: settings.sound,
  };
}

/**
 * Push preferences to the Rust monitor. Best effort: a dev browser or an older
 * build without the command must not break the Settings screen.
 */
export async function syncSettingsToRust(settings: NotificationSettings): Promise<void> {
  if (!isTauri()) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("set_notification_settings", { settings: toRust(settings) });
  } catch {
    // The JS copy is authoritative for the foreground UI.
  }
}

async function plugin() {
  return import("@tauri-apps/plugin-notification");
}

export async function notificationPermission(): Promise<NotificationPermission> {
  if (!isTauri()) return "unsupported";
  try {
    const { isPermissionGranted } = await plugin();
    return (await isPermissionGranted()) ? "granted" : "prompt";
  } catch {
    return "unsupported";
  }
}

/** Ask the OS for permission. Returns the resulting state. */
export async function requestNotificationPermission(): Promise<NotificationPermission> {
  if (!isTauri()) return "unsupported";
  try {
    const { isPermissionGranted, requestPermission } = await plugin();
    if (await isPermissionGranted()) return "granted";
    return (await requestPermission()) === "granted" ? "granted" : "denied";
  } catch {
    return "unsupported";
  }
}

/**
 * Ensure the Android delivery channel exists and matches the sound preference.
 * Channels are Android-only; on other platforms this is a no-op.
 */
export async function ensureNotificationChannel(settings: NotificationSettings): Promise<void> {
  if (!isTauri()) return;
  try {
    const { createChannel, Importance, Visibility } = await plugin();
    await createChannel({
      id: "solayge",
      name: "Solayge alerts",
      description: "Agent questions, task completions, and failures",
      importance: settings.sound ? Importance.High : Importance.Low,
      visibility: Visibility.Private,
      sound: settings.sound ? "default" : undefined,
    });
  } catch {
    // Android-only; ignore elsewhere.
  }
}

/** Send one immediate notification, used by the Settings "Test" button. */
export async function sendTestNotification(
  settings: NotificationSettings,
): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    const { isPermissionGranted, requestPermission, sendNotification } = await plugin();
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (!granted) return false;
    sendNotification({
      title: "Solayge notifications are on",
      body: "You'll be told when an agent needs you or a task finishes.",
      sound: settings.sound ? "default" : undefined,
      channelId: "solayge",
    });
    return true;
  } catch {
    return false;
  }
}

/** Load the already-notified state for a desktop. */
export function loadNotificationState(desktopId: string): NotificationState | null {
  const store = storage();
  if (!store || !desktopId) return null;
  try {
    const raw = store.getItem(`${STATE_PREFIX}${desktopId}`);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as NotificationState;
    if (!parsed || !Array.isArray(parsed.approvals) || typeof parsed.sessions !== "object") {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

export function saveNotificationState(desktopId: string, state: NotificationState): void {
  const store = storage();
  if (!store || !desktopId) return;
  try {
    store.setItem(`${STATE_PREFIX}${desktopId}`, JSON.stringify(state));
  } catch {
    // Best effort.
  }
}

function isCompleted(status: string): boolean {
  return status === "succeeded" || status === "completed";
}

/**
 * Compute the notifications for a fresh snapshot. `previous` is null the first
 * time a desktop is seen: the whole backlog is recorded without notifying, so a
 * new install never dumps a pile of alerts for work that finished long ago.
 */
export function diffNotifications(
  previous: NotificationState | null,
  sessions: SessionSummary[],
  approvals: ApprovalSummary[],
): { events: NotificationEvent[]; state: NotificationState } {
  const state: NotificationState = {
    approvals: approvals.map((approval) => approval.id),
    sessions: Object.fromEntries(sessions.map((session) => [session.id, session.status])),
  };
  if (!previous) return { events: [], state };

  const knownApprovals = new Set(previous.approvals);
  const events: NotificationEvent[] = [];
  for (const approval of approvals) {
    if (knownApprovals.has(approval.id)) continue;
    events.push({
      kind: "question",
      title:
        approval.kind === "question" ? "Agent has a question" : "Permission needed",
      body: approval.title,
    });
  }
  for (const session of sessions) {
    const before = previous.sessions[session.id];
    if (before === undefined || before === session.status) continue;
    if (isCompleted(session.status)) {
      events.push({ kind: "completed", title: "Task completed", body: session.title });
    } else if (session.status === "failed") {
      events.push({ kind: "failed", title: "Task failed", body: session.title });
    }
  }
  return { events, state };
}

/** Which events a settings object allows. */
export function allows(settings: NotificationSettings, kind: NotificationKind): boolean {
  if (!settings.enabled) return false;
  if (kind === "question") return settings.onQuestions;
  if (kind === "completed") return settings.onCompleted;
  return settings.onFailed;
}

let audioContext: AudioContext | null = null;

/**
 * Play a short, distinct chime for an event. This is the in-app sound whose
 * volume the user controls; the OS notification has its own system sound. It is
 * best-effort: a blocked AudioContext must never break the alert.
 */
export function playTone(kind: NotificationKind, volume: number): void {
  if (volume <= 0) return;
  const Ctor =
    typeof window !== "undefined" && "AudioContext" in window ? window.AudioContext : undefined;
  if (!Ctor) return;
  try {
    audioContext ??= new Ctor();
    const context = audioContext;
    const now = context.currentTime;
    const frequency = kind === "failed" ? 320 : kind === "question" ? 660 : 880;
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.type = "sine";
    oscillator.frequency.setValueAtTime(frequency, now);
    gain.gain.setValueAtTime(Math.min(1, Math.max(0, volume)) * 0.2, now);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.3);
    oscillator.connect(gain).connect(context.destination);
    oscillator.start(now);
    oscillator.stop(now + 0.3);
  } catch {
    // Audio is optional feedback.
  }
}
