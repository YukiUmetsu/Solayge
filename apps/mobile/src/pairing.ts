import { invoke } from "@tauri-apps/api/core";

/**
 * Mobile pairing surface. The transport (Iroh/Tailscale) is a separate layer;
 * these commands produce and verify *identity* material only:
 *
 * - `mobileIdentity` loads the installation's stable public identity.
 * - `parsePairingQr` validates a scanned QR and derives the verification code
 *   the user compares with the desktop (the QR carries the desktop's public
 *   identity and nonce, so both sides can compute it independently).
 * - `manualPairingRequest` builds a `pair.begin` for a typed code when no camera
 *   is available.
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
  begin_request: unknown;
}

export interface ManualPairing {
  device_id: string;
  manual_code: string;
  begin_request: unknown;
}

export const pairingApi = {
  mobileIdentity: () => invoke<IdentityView>("mobile_identity"),
  parsePairingQr: (qr: string) =>
    invoke<ParsedPairing>("parse_pairing_qr", { qr }),
  manualPairingRequest: (code: string) =>
    invoke<ManualPairing>("manual_pairing_request", { code }),
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
