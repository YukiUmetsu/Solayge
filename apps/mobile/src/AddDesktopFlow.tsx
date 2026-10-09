import { useCallback, useEffect, useState } from "react";

import {
  formatVerificationCode,
  isExpired,
  pairingApi,
  scanErrorMessage,
  type PairStart,
  type PairedDesktopRecord,
} from "./pairing";

type Phase = "intro" | "prep" | "scan" | "confirm" | "paste" | "done";

/** The four milestones shown as an accomplishment trail. */
const STEPS = [
  { id: "prep", label: "Get the desktop ready" },
  { id: "scan", label: "Scan the QR code" },
  { id: "confirm", label: "Approve on the desktop" },
  { id: "done", label: "Connected" },
] as const;

const PHASE_STEP: Record<Phase, number> = {
  intro: 0,
  prep: 0,
  scan: 1,
  paste: 1,
  confirm: 2,
  done: 3,
};

/**
 * The "Add a desktop" onboarding wizard.
 *
 * It walks the user through preparing the desktop, scanning the QR, comparing
 * the verification code, approving the phone on the desktop, and celebrating the
 * finished connection. Every failure degrades to the paste-a-link fallback, so a
 * missing camera or denied permission is never a dead end.
 */
export function AddDesktopFlow({
  onClose,
  onPaired,
}: {
  onClose: () => void;
  onPaired?: (desktop: PairedDesktopRecord) => void;
}) {
  const [phase, setPhase] = useState<Phase>("intro");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [code, setCode] = useState<PairStart | null>(null);
  const [paired, setPaired] = useState<PairedDesktopRecord | null>(null);
  const [pasteValue, setPasteValue] = useState("");

  const handleQr = useCallback(async (qr: string) => {
    setError(null);
    setBusy(true);
    try {
      const parsed = await pairingApi.parsePairingQr(qr);
      if (isExpired(parsed.expires_at)) {
        setError("This pairing code has expired. Show a new one on the desktop and try again.");
        setPhase("prep");
        return;
      }
      if (!parsed.reachable) {
        setError(
          "This code has no connection address. On the desktop, open Solayge and keep it running, then show a new code.",
        );
        setPhase("prep");
        return;
      }
      const start = await pairingApi.pairWithQr(qr);
      setCode(start);
      setPhase("confirm");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const startScan = useCallback(async () => {
    setError(null);
    setBusy(true);
    setPhase("scan");
    try {
      const { scan, Format, checkPermissions, requestPermissions } = await import(
        "@tauri-apps/plugin-barcode-scanner"
      );
      let permission = await checkPermissions();
      if (permission !== "granted") {
        permission = await requestPermissions();
      }
      if (permission !== "granted") {
        setError("Camera access was denied. Paste the pairing link instead.");
        setPhase("paste");
        return;
      }
      const result = await scan({ formats: [Format.QRCode], cameraDirection: "back" });
      await handleQr(result.content);
    } catch (e) {
      setError(scanErrorMessage(e));
      setPhase("paste");
    } finally {
      setBusy(false);
    }
  }, [handleQr]);

  // Poll for desktop approval while the confirmation step is visible.
  useEffect(() => {
    if (phase !== "confirm") return undefined;
    let cancelled = false;
    const poll = async () => {
      try {
        const status = await pairingApi.pairPoll();
        if (cancelled) return;
        if (status.state === "approved" && status.desktop) {
          setPaired(status.desktop);
          setPhase("done");
        } else if (status.state === "error") {
          setError(status.message ?? "The desktop became unreachable. Check both devices and try again.");
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [phase]);

  const cancelPairing = useCallback(async () => {
    try {
      await pairingApi.pairCancel();
    } catch {
      /* an expired challenge is fine to drop */
    }
  }, []);

  const back = useCallback(() => {
    setError(null);
    if (phase === "done") {
      if (paired) onPaired?.(paired);
      onClose();
      return;
    }
    if (phase === "confirm") void cancelPairing();
    setPhase(phase === "confirm" || phase === "paste" ? "prep" : "intro");
  }, [phase, paired, onPaired, onClose, cancelPairing]);

  const finish = useCallback(() => {
    if (paired) onPaired?.(paired);
    onClose();
  }, [paired, onPaired, onClose]);

  return (
    <section className="onboarding" aria-label="Add a desktop">
      <header className="onboarding__header">
        <button className="onboarding__back" type="button" onClick={back}>
          {phase === "intro" ? "Cancel" : "Back"}
        </button>
        <h1 className="onboarding__title">Connect to a desktop</h1>
      </header>

      <StepTrail current={PHASE_STEP[phase]} />

      {phase === "intro" && (
        <div className="onboarding__body">
          <p className="onboarding__lead">
            Leave the desk without leaving your agents unattended. Pair this
            phone once, then monitor and answer your agents from anywhere on your
            own encrypted connection.
          </p>
          <ul className="wizard__points">
            <li>The desktop stays in charge of running tasks.</li>
            <li>You approve this phone on the computer — never automatically.</li>
            <li>The connection is end-to-end encrypted by Solayge.</li>
          </ul>
          <button className="btn btn-primary" type="button" onClick={() => setPhase("prep")}>
            Get started
          </button>
          <button className="btn" type="button" onClick={onClose}>
            Not now
          </button>
        </div>
      )}

      {phase === "prep" && (
        <div className="onboarding__body">
          <p className="onboarding__lead">On your computer, do this first:</p>
          <ol className="wizard__steps">
            <li>Open the Solayge desktop app.</li>
            <li>
              Go to <strong>Settings → Devices</strong>.
            </li>
            <li>
              Choose <strong>Add phone</strong>. Keep that screen open — it shows
              the QR code you&apos;ll scan next.
            </li>
          </ol>
          <p className="onboarding__note">
            Both devices need to be online. You can use different networks; the
            connection is relayed securely when a direct path isn&apos;t available.
          </p>
          <button className="btn btn-primary" type="button" onClick={startScan} disabled={busy}>
            Scan QR code
          </button>
          <button className="btn" type="button" onClick={() => setPhase("paste")} disabled={busy}>
            Paste pairing link
          </button>
        </div>
      )}

      {phase === "scan" && (
        <div className="onboarding__body" role="status">
          <p className="onboarding__lead">Point the camera at the QR code on your desktop…</p>
        </div>
      )}

      {phase === "paste" && (
        <div className="onboarding__body">
          <label className="onboarding__label" htmlFor="pairing-link">
            Paste pairing link
          </label>
          <p className="onboarding__note">
            On the desktop choose <strong>Copy pairing link</strong>, then paste it
            here. Useful when the camera is unavailable.
          </p>
          <textarea
            id="pairing-link"
            className="onboarding__textarea mono"
            value={pasteValue}
            rows={4}
            placeholder='{"protocol_version":1,…}'
            spellCheck={false}
            onChange={(e) => setPasteValue(e.target.value)}
          />
          <button
            className="btn btn-primary"
            type="button"
            onClick={() => void handleQr(pasteValue.trim())}
            disabled={busy || pasteValue.trim().length === 0}
          >
            Continue
          </button>
          <button className="btn" type="button" onClick={startScan} disabled={busy}>
            Scan QR code instead
          </button>
        </div>
      )}

      {phase === "confirm" && code && (
        <div className="onboarding__body">
          <p className="onboarding__lead">Check that these match, then approve this phone.</p>
          <div className="verification" aria-label="Verification code">
            <span className="verification__code">
              {formatVerificationCode(code.verification_code)}
            </span>
          </div>
          <p className="onboarding__note">
            On the desktop, the same six digits appear. They must match exactly —
            if they don&apos;t, tap Back and start again. This protects you from
            connecting to the wrong computer.
          </p>
          <p className="onboarding__note">
            Then choose <strong>Approve phone</strong> on the desktop.
          </p>
          <div className="wizard__waiting" role="status" aria-live="polite">
            <span className="wizard__spinner" aria-hidden="true" />
            Waiting for approval…
          </div>
          <button className="btn" type="button" onClick={back}>
            Cancel
          </button>
        </div>
      )}

      {phase === "done" && paired && (
        <div className="onboarding__body celebrate" role="status">
          <div className="celebrate__badge" aria-hidden="true">
            ✓
          </div>
          <h2 className="celebrate__title">You&apos;re connected!</h2>
          <p className="onboarding__lead">
            This phone is now trusted by <strong>{paired.name}</strong>.
          </p>
          <dl className="onboarding__facts">
            <div>
              <dt>Platform</dt>
              <dd>{paired.platform || "unknown"}</dd>
            </div>
            <div>
              <dt>Desktop fingerprint</dt>
              <dd className="mono">{paired.fingerprint}</dd>
            </div>
          </dl>
          <button className="btn btn-primary" type="button" onClick={finish}>
            Start using Solayge
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

function StepTrail({ current }: { current: number }) {
  return (
    <ol className="wizard__trail" aria-label="Setup progress">
      {STEPS.map((step, index) => {
        const state =
          index < current ? "done" : index === current ? "current" : "pending";
        return (
          <li key={step.id} className={`wizard__trail-item wizard__trail-item--${state}`}>
            <span className="wizard__trail-mark" aria-hidden="true">
              {state === "done" ? "✓" : index + 1}
            </span>
            <span className="wizard__trail-label">{step.label}</span>
          </li>
        );
      })}
    </ol>
  );
}
