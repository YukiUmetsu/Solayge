import {
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { api } from "../api";
import type { NotificationSettings, NotifyEvent, NotifyKind } from "../types";

/**
 * The bundled notification tones. `id` is what `NotificationSettings` stores;
 * `src` is the public path served by the webview.
 */
export const SOUND_PRESETS: { id: string; label: string; src: string }[] = [
  { id: "complete", label: "Chime (rising)", src: "/sounds/complete.wav" },
  { id: "failed", label: "Thud (falling)", src: "/sounds/failed.wav" },
  { id: "review", label: "Ping", src: "/sounds/review.wav" },
  { id: "attention", label: "Double beep", src: "/sounds/attention.wav" },
];

/** A reasonable default preset for each event kind. */
export const DEFAULT_SOUND_FOR: Record<NotifyKind, string> = {
  task_complete: "complete",
  task_failed: "failed",
  task_review: "review",
  needs_attention: "attention",
  system: "complete",
};

/** Mirrors the backend defaults; used when older state lacks the field. */
export const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  enabled: true,
  on_task_complete: true,
  on_task_failed: true,
  on_task_review: true,
  on_needs_attention: true,
  sound_enabled: true,
  volume: 0.8,
  complete_sound: "complete",
  failed_sound: "failed",
  review_sound: "review",
  attention_sound: "attention",
};

/** Which setting field holds the sound for a given event kind. */
export function soundValue(
  kind: NotifyKind,
  n: NotificationSettings,
): string | null {
  switch (kind) {
    case "task_complete":
    case "system":
      return n.complete_sound;
    case "task_failed":
      return n.failed_sound;
    case "task_review":
      return n.review_sound;
    case "needs_attention":
      return n.attention_sound;
  }
}

const urlCache = new Map<string, string>();

/** Resolve a stored sound value to a URL the webview can play. */
async function resolveSoundUrl(value: string): Promise<string | null> {
  const cached = urlCache.get(value);
  if (cached) return cached;
  let url: string | null = null;
  if (value.startsWith("file:")) {
    try {
      url = await api.readSound(value.slice("file:".length));
    } catch {
      url = null;
    }
  } else {
    url = SOUND_PRESETS.find((p) => p.id === value)?.src ?? null;
  }
  if (url) urlCache.set(value, url);
  return url;
}

/** Play a stored sound value at `volume` (0..1). Never throws. */
export async function playSound(
  value: string | null,
  volume: number,
): Promise<void> {
  if (!value) return;
  const url = await resolveSoundUrl(value);
  if (!url) return;
  try {
    const audio = new Audio(url);
    audio.volume = Math.max(0, Math.min(1, volume));
    await audio.play();
  } catch {
    /* autoplay blocked or format unsupported; the notification still shows */
  }
}

/** Ask for permission up front so the first real notification can show. */
export async function primeNotificationPermission(): Promise<void> {
  try {
    if (!(await isPermissionGranted())) await requestPermission();
  } catch {
    /* notifications are optional */
  }
}

/**
 * Handle one backend notification: play the configured sound. The OS
 * notification and the in-app toast are produced elsewhere, so a denied or
 * throttled desktop notification never leaves the user without feedback.
 */
export function handleNotification(
  ev: NotifyEvent,
  n: NotificationSettings | undefined,
): void {
  if (!n || !n.enabled || !n.sound_enabled) return;
  void playSound(soundValue(ev.kind, n), n.volume);
}
