import { invoke } from "@tauri-apps/api/core";

/**
 * Mobile pairing surface.
 *
 * The first three commands work on identity material alone (offline preview and
 * manual-code shaping). The `pair*` commands run the real handshake over the
 * Iroh transport and wait for the desktop user to approve the phone.
 */

export interface IdentityView {
  device_id: string;
  public_key: string;
  fingerprint: string;
}

export interface ParsedPairing {
  desktop_id: string;
  desktop_public_key: string;
  desktop_fingerprint: string;
  pairing_id: string;
  expires_at: number;
  verification_code: string;
  device_id: string;
  /** Whether the QR carried an address the phone can dial. */
  reachable: boolean;
}

export interface ManualPairing {
  device_id: string;
  manual_code: string;
}

export interface PairStart {
  pairing_id: string;
  device_id: string;
  desktop_id: string;
  verification_code: string;
}

export interface PairedDesktopRecord {
  id: string;
  name: string;
  platform: string;
  fingerprint: string;
}

export interface PairStatus {
  state: "none" | "pending" | "approved" | "error";
  desktop?: PairedDesktopRecord;
  message?: string;
}

export const pairingApi = {
  mobileIdentity: () => invoke<IdentityView>("mobile_identity"),
  parsePairingQr: (qr: string) =>
    invoke<ParsedPairing>("parse_pairing_qr", { qr }),
  manualPairingRequest: (code: string) =>
    invoke<ManualPairing>("manual_pairing_request", { code }),
  pairWithQr: (qr: string) => invoke<PairStart>("pair_with_qr", { qr }),
  pairPoll: () => invoke<PairStatus>("pair_poll"),
  pairCancel: () => invoke<void>("pair_cancel"),
  pairedDesktops: () => invoke<PairedDesktopRecord[]>("paired_desktops"),
  forgetDesktop: (desktopId: string) =>
    invoke<void>("forget_desktop", { desktopId }),
};

/**
 * Normalize a typed pairing code: uppercase, keep only A–Z/0–9, and group it as
 * `ABCD-EFGH`. The backend accepts either the grouped or ungrouped form.
 */
export function normalizeManualCode(input: string): string {
  const cleaned = input.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 8);
  if (cleaned.length <= 4) return cleaned;
  return `${cleaned.slice(0, 4)}-${cleaned.slice(4)}`;
}

/** Whether a typed code is a complete 8-character pairing code. */
export function isManualCodeComplete(input: string): boolean {
  return /^[A-Z0-9]{4}-[A-Z0-9]{4}$/.test(normalizeManualCode(input));
}

/** Render a verification code as `123 456` regardless of supplied spacing. */
export function formatVerificationCode(code: string): string {
  const digits = code.replace(/\D/g, "").slice(0, 6);
  if (digits.length <= 3) return digits;
  return `${digits.slice(0, 3)} ${digits.slice(3)}`;
}

/** Whether a pairing challenge has passed its expiry (unix seconds vs ms now). */
export function isExpired(expiresAt: number, nowMs: number = Date.now()): boolean {
  return Number.isFinite(expiresAt) && expiresAt * 1000 <= nowMs;
}

/** A short, plain-language message for a failed scan. */
export function scanErrorMessage(error: unknown): string {
  const text = String(error ?? "").toLowerCase();
  if (text.includes("permission") || text.includes("denied")) {
    return "Camera access was denied. Enter the pairing code instead.";
  }
  if (text.includes("not found") || text.includes("unavailable")) {
    return "No camera is available. Enter the pairing code instead.";
  }
  return "Couldn't read the QR code. Enter the pairing code instead.";
}
