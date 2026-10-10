import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import type { ConnectedDevice, PairingOffer, PairingStatus } from "../types";
import { clock, relTime } from "../lib/format";
import { ErrorNote } from "./Field";
import { Icon } from "./Icons";
import { QrCode } from "./QrCode";

/** Friendly names for the mobile spec's scopes. */
const SCOPE_LABELS: Record<string, string> = {
  "sessions.read": "View",
  "sessions.message": "Message",
  "sessions.control": "Control",
  "approvals.respond": "Approve requests",
  "devices.manage": "Manage devices",
};

/** The default grant for a new phone: View + Message. */
const DEFAULT_SCOPES = ["sessions.read", "sessions.message"];

/** What the desktop may grant here. `devices.manage` is deliberately absent. */
const OFFERED_SCOPES = [
  "sessions.read",
  "sessions.message",
  "sessions.control",
  "approvals.respond",
];

function scopeLabel(scope: string): string {
  return SCOPE_LABELS[scope] ?? scope;
}

/** Render a verification code as `123 456`. */
function formatVerificationCode(code: string): string {
  const digits = code.replace(/\D/g, "").slice(0, 6);
  return digits.length <= 3 ? digits : `${digits.slice(0, 3)} ${digits.slice(3)}`;
}

/** The step-by-step pairing walkthrough, mirrored by the phone wizard. */
const PAIRING_STEPS = [
  "Install Solayge Mobile on your phone and open it.",
  "Choose “Add a desktop”, then “Scan QR code”.",
  "Scan the code below — or copy the pairing link and paste it on the phone.",
  "Compare the six-digit code shown on both devices.",
  "Approve the phone here.",
];

/**
 * Settings → Connected devices. Lists approved phones and pairs new ones with a
 * QR code (or a copyable pairing link), defaulting the grant to View + Message.
 * Revoking a device invalidates it on every transport.
 */
export function ConnectedDevices() {
  const [devices, setDevices] = useState<ConnectedDevice[] | null>(null);
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  const [scopes, setScopes] = useState<string[]>(DEFAULT_SCOPES);
  const [status, setStatus] = useState<PairingStatus>({ state: "pending" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setDevices(await api.devicesList());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Poll the in-flight challenge while the Add-phone dialog is open.
  useEffect(() => {
    if (!offer) return;
    const poll = () => {
      api
        .pairingStatus(offer.pairing_id)
        .then((next) => {
          if (next) setStatus(next);
        })
        .catch(() => {});
    };
    poll();
    const timer = window.setInterval(poll, 1500);
    return () => window.clearInterval(timer);
  }, [offer]);

  async function addPhone() {
    setBusy(true);
    setError(null);
    setCopied(false);
    try {
      setScopes(DEFAULT_SCOPES);
      setStatus({ state: "pending" });
      setOffer(await api.pairingStart());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function approve() {
    if (!offer) return;
    setBusy(true);
    setError(null);
    try {
      await api.pairingApprove(offer.pairing_id, scopes);
      setOffer(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    const current = offer;
    setOffer(null);
    if (!current) return;
    try {
      await api.pairingReject(current.pairing_id);
    } catch {
      /* a challenge that already expired is fine to drop */
    }
  }

  async function revoke(deviceId: string) {
    setBusy(true);
    setError(null);
    try {
      await api.deviceRevoke(deviceId);
      setConfirmRevoke(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  function toggleScope(scope: string) {
    setScopes((current) =>
      current.includes(scope)
        ? current.filter((s) => s !== scope)
        : [...current, scope],
    );
  }

  async function copyLink() {
    if (!offer) return;
    try {
      await navigator.clipboard.writeText(offer.qr_string);
      setCopied(true);
    } catch {
      setError("Could not copy the pairing link. Select the code instead.");
    }
  }

  const phoneConnected =
    status.state === "pending" && Boolean(status.verification_code);
  const approved = status.state === "approved";

  return (
    <section className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <div className="text-[13px] font-medium text-ink">Connected devices</div>
          <p className="mt-0.5 text-[11px] leading-relaxed text-ink-subtle">
            Phones you have approved. Revoking one ends its sessions and blocks it
            on every transport immediately.
          </p>
        </div>
        <button className="btn btn-primary" onClick={addPhone} disabled={busy}>
          <Icon name="plus" className="h-3.5 w-3.5" />
          Add phone
        </button>
      </div>

      {offer && (
        <div className="space-y-3 rounded-lg border border-line bg-well p-4">
          <div className="flex items-start justify-between gap-4">
            <div>
              <div className="text-[12.5px] font-medium text-ink">Add a phone</div>
              <p className="mt-0.5 text-[11px] leading-relaxed text-ink-subtle">
                Pair Solayge Mobile with this computer in five short steps.
              </p>
            </div>
            <span
              className="shrink-0 text-[11px] text-ink-subtle"
              role="status"
              aria-live="polite"
            >
              {approved
                ? "Approved"
                : status.state === "rejected"
                  ? "Rejected"
                  : phoneConnected
                    ? "Phone connected"
                    : "Waiting for phone"}
            </span>
          </div>

          {!offer.reachable && (
            <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-[11px] leading-relaxed text-ink">
              <div className="font-medium">The desktop mobile host isn&apos;t ready</div>
              <p className="mt-0.5 text-ink-subtle">
                This code has no connection address yet, so a phone can&apos;t dial
                it. Keep Solayge running and try again; restart the app if this
                persists.
              </p>
            </div>
          )}

          <ol className="space-y-1.5 text-[11px] leading-relaxed text-ink-muted">
            {PAIRING_STEPS.map((step, index) => {
              const reached = approved ? true : phoneConnected ? index <= 2 : index <= 1;
              return (
                <li key={step} className="flex gap-2">
                  <span
                    className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[9px] font-semibold ${
                      reached
                        ? "bg-success/15 text-success"
                        : "bg-well text-ink-faint ring-1 ring-line"
                    }`}
                    aria-hidden="true"
                  >
                    {reached ? "✓" : index + 1}
                  </span>
                  <span className={reached ? "text-ink" : undefined}>{step}</span>
                </li>
              );
            })}
          </ol>

          <div className="flex flex-wrap items-start gap-5">
            <QrCode value={offer.qr_string} size={200} />
            <div className="min-w-[220px] flex-1 space-y-3">
              <div>
                <div className="text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                  Or enter this code
                </div>
                <div className="mono mt-1 text-lg font-semibold tracking-[0.15em] text-ink">
                  {offer.manual_code}
                </div>
              </div>

              {phoneConnected ? (
                <div className="rounded-lg border border-line bg-surface p-3">
                  <div className="text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                    Verification code
                  </div>
                  <div className="mono mt-1 text-2xl font-bold tracking-[0.2em] text-ink">
                    {formatVerificationCode(status.verification_code ?? "")}
                  </div>
                  <p className="mt-1 text-[11px] leading-relaxed text-ink-subtle">
                    {status.device_name ? `${status.device_name} · ` : ""}
                    {status.device_platform ?? "phone"} — this must match the
                    phone exactly.
                  </p>
                </div>
              ) : (
                <p className="text-[11px] leading-relaxed text-ink-subtle">
                  The six-digit code appears here as soon as the phone scans this
                  code and starts pairing.
                </p>
              )}

              <div>
                <div className="text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                  Desktop fingerprint
                </div>
                <div className="mono mt-1 break-all text-[11px] text-ink-muted">
                  {offer.desktop_fingerprint}
                </div>
              </div>
              <button className="btn btn-ghost !px-2.5 !py-1 text-[11px]" onClick={copyLink}>
                <Icon name="layers" className="h-3 w-3" />
                {copied ? "Link copied" : "Copy pairing link"}
              </button>
            </div>
          </div>

          <div className="space-y-2">
            <div className="text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
              Permissions
            </div>
            <div className="flex flex-wrap gap-3">
              {OFFERED_SCOPES.map((scope) => (
                <label
                  key={scope}
                  className="flex items-center gap-2 text-[12px] text-ink"
                >
                  <input
                    type="checkbox"
                    checked={scopes.includes(scope)}
                    disabled={busy}
                    onChange={() => toggleScope(scope)}
                  />
                  {scopeLabel(scope)}
                </label>
              ))}
            </div>
            <p className="text-[11px] leading-relaxed text-ink-subtle">
              A new phone gets View + Message. Device administration is never
              granted during pairing.
            </p>
          </div>

          <div className="flex justify-end gap-2">
            <button className="btn btn-ghost" onClick={cancel} disabled={busy}>
              Cancel
            </button>
            <button className="btn btn-primary" onClick={approve} disabled={busy}>
              <Icon name="check" className="h-3.5 w-3.5" />
              Approve phone
            </button>
          </div>
        </div>
      )}

      <ErrorNote error={error} />

      <div className="overflow-hidden rounded-lg border border-line bg-well">
        {devices === null ? (
          <div className="px-3 py-4 text-[12px] text-ink-subtle">Loading…</div>
        ) : devices.length === 0 ? (
          <div className="px-3 py-4 text-[12px] text-ink-subtle">
            No phones are connected. Use <strong>Add phone</strong> to pair one.
          </div>
        ) : (
          <ul className="divide-y divide-line">
            {devices.map((device) => (
              <li key={device.device_id} className="flex items-start gap-3 px-3 py-3">
                <span
                  aria-hidden="true"
                  className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${
                    device.online ? "bg-success" : "bg-ink-faint"
                  }`}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-[12.5px] font-medium text-ink">
                      {device.name}
                    </span>
                    <span className="rounded border border-line px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-ink-subtle">
                      {device.platform}
                    </span>
                    <span className="text-[11px] text-ink-subtle">
                      {device.online ? "Online" : "Offline"}
                    </span>
                  </div>
                  <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-ink-subtle">
                    <span>
                      {device.scopes.map(scopeLabel).join(" · ") || "No permissions"}
                    </span>
                    <span>Paired {clock(device.paired_at)}</span>
                    <span>Last seen {relTime(device.last_seen)}</span>
                  </div>
                  <div className="mono mt-1 break-all text-[10px] text-ink-faint">
                    {device.fingerprint}
                  </div>
                </div>
                {confirmRevoke === device.device_id ? (
                  <div className="flex shrink-0 items-center gap-2">
                    <button
                      className="btn btn-ghost !px-2 !py-1 text-[11px]"
                      onClick={() => setConfirmRevoke(null)}
                      disabled={busy}
                    >
                      Keep
                    </button>
                    <button
                      className="btn btn-danger !px-2 !py-1 text-[11px]"
                      onClick={() => void revoke(device.device_id)}
                      disabled={busy}
                    >
                      Revoke
                    </button>
                  </div>
                ) : (
                  <button
                    className="btn btn-ghost shrink-0 !px-2 !py-1 text-[11px]"
                    onClick={() => setConfirmRevoke(device.device_id)}
                    disabled={busy}
                  >
                    <Icon name="trash" className="h-3 w-3" />
                    Revoke access
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      <p className="text-[11px] leading-relaxed text-ink-subtle">
        &ldquo;Online&rdquo; means a phone currently has an authenticated session — not
        merely that it is registered.
      </p>
    </section>
  );
}
