/**
 * Android push bridge.
 *
 * The native `FirebaseMessagingService` writes the FCM token and any
 * notification tap to files the Rust shell owns; these helpers read them
 * through Tauri commands. Everything degrades to a no-op outside the Tauri
 * shell (browser dev, tests), so the UI never depends on native push.
 */
import { isTauri } from "./notifications";

export interface PushToken {
  token: string;
  platform: string;
}

/** The opaque identifiers a push carries. No content, ever. */
export interface PushTap {
  event_id?: string;
  kind?: string;
  desktop_id?: string;
  session_id?: string;
  approval_id?: string;
}

export async function pushToken(): Promise<PushToken | null> {
  if (!isTauri()) return null;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const value = await invoke<PushToken | null>("push_token");
    return value && value.token ? value : null;
  } catch {
    return null;
  }
}

/** Read and consume a notification tap made while the app was closed. */
export async function takePushTap(): Promise<PushTap | null> {
  if (!isTauri()) return null;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<PushTap | null>("take_push_tap");
  } catch {
    return null;
  }
}
