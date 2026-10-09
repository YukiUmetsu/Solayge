import { useState } from "react";

import {
  formatVerificationCode,
  isExpired,
  isManualCodeComplete,
  normalizeManualCode,
  pairingApi,
  scanErrorMessage,
  type ParsedPairing,
} from "./pairing";

type Phase = "choose" | "scan" | "manual" | "verify" | "manualWaiting";

/**
 * The "Add desktop" onboarding flow.
 *
 * The camera permission is requested only when the scanner opens, and every
 * failure path (no camera, denied permission, unreadable code) degrades to the
 * manual pairing-code fallback. The verification step shows the short code the
 * user compares with the desktop; approval always happens on the desktop.
 */
export function AddDesktopFlow({ onClose }: { onClose: () => void }) {
  const [phase, setPhase] = useState<Phase>("choose");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [parsed, setParsed] = useState<ParsedPairing | null>(null);
  const [manualCode, setManualCode] = useState("");

  async function startScan() {
    setError(null);
    setPhase("scan");
    setBusy(true);
    try {
      const { scan, Format, checkPermissions, requestPermissions } = await import(
        "@tauri-apps/plugin-barcode-scanner"
      );
      let permission = await checkPermissions();
      if (permission !== "granted") {
        permission = await requestPermissions();
      }
      if (permission !== "granted") {
        setError("Camera access was denied. Enter the pairing code instead.");
        setPhase("manual");
        return;
      }
      const result = await scan({ formats: [Format.QRCode], cameraDirection: "back" });
      const next = await pairingApi.parsePairingQr(result.content);
      if (isExpired(next.expires_at)) {
        setError("This pairing code has expired. Ask your desktop to show a new one.");
        setPhase("choose");
        return;
      }
      setParsed(next);
      setPhase("verify");
    } catch (e) {
      setError(scanErrorMessage(e));
      setPhase("manual");
    } finally {
      setBusy(false);
    }
  }

  async function submitManual() {
    setError(null);
    setBusy(true);
    try {
      const result = await pairingApi.manualPairingRequest(normalizeManualCode(manualCode));
      setManualCode(result.manual_code);
      setPhase("manualWaiting");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="onboarding" aria-label="Add a desktop">
      <header className="onboarding__header">
        <button className="onboarding__back" type="button" onClick={onClose}>
          Cancel
        </button>
        <h1 className="onboarding__title">Add a desktop</h1>
      </header>

      {phase === "choose" && (
        <div className="onboarding__body">
          <p className="onboarding__lead">
            Your computer must be running Solayge. Scan the QR code it shows, or
            enter the pairing code.
          </p>
          <button className="btn btn-primary" type="button" onClick={startScan} disabled={busy}>
            Scan QR code
          </button>
          <button className="btn" type="button" onClick={() => setPhase("manual")} disabled={busy}>
            Enter pairing code
          </button>
          <details className="onboarding__help">
            <summary>How it works</summary>
            <p>
              The QR code contains only public information. You still approve this
              phone on the desktop, and both devices show the same short
              verification code.
            </p>
          </details>
        </div>
      )}

      {phase === "scan" && (
        <div className="onboarding__body" role="status">
          <p className="onboarding__lead">Point the camera at the QR code on your desktop…</p>
        </div>
      )}

      {phase === "manual" && (
        <div className="onboarding__body">
          <label className="onboarding__label" htmlFor="pairing-code">
            Pairing code
          </label>
          <input
            id="pairing-code"
            className="onboarding__input"
            value={manualCode}
            inputMode="text"
            autoCapitalize="characters"
            autoComplete="off"
            spellCheck={false}
            placeholder="ABCD-EFGH"
            onChange={(e) => setManualCode(normalizeManualCode(e.target.value))}
          />
          <button
            className="btn btn-primary"
            type="button"
            onClick={submitManual}
            disabled={busy || !isManualCodeComplete(manualCode)}
          >
            Continue
          </button>
          <button className="btn" type="button" onClick={() => setPhase("choose")} disabled={busy}>
            Back
          </button>
        </div>
      )}

      {phase === "manualWaiting" && (
        <div className="onboarding__body" role="status">
          <p className="onboarding__lead">
            On your desktop, choose <strong>Approve phone</strong>.
          </p>
          <p className="onboarding__meta">
            Code <span className="mono">{manualCode}</span>
          </p>
          <p className="onboarding__note">
            When you approve, this phone and the desktop will show the same short
            verification code. Compare them before you trust the connection.
          </p>
          <button className="btn" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      )}

      {phase === "verify" && parsed && (
        <div className="onboarding__body">
          <p className="onboarding__lead">Compare these codes on both devices.</p>
          <div className="verification" aria-label="Verification code">
            <span className="verification__code">
              {formatVerificationCode(parsed.verification_code)}
            </span>
          </div>
          <p className="onboarding__note">
            They must match exactly. If they don&apos;t, cancel and try again — this
            protects you from connecting to the wrong computer.
          </p>
          <dl className="onboarding__facts">
            <div>
              <dt>Desktop fingerprint</dt>
              <dd className="mono">{parsed.desktop_fingerprint}</dd>
            </div>
          </dl>
          <p className="onboarding__note">
            Now approve this phone on your desktop. This phone will connect once
            it is approved.
          </p>
          <button className="btn" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      )}

      {error && (
        <p className="onboarding__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
