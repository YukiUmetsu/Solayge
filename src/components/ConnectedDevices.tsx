import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import type { ConnectedDevice, PairingOffer } from "../types";
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

/**
 * Settings → Connected devices. Lists approved phones and pairs new ones with a
 * QR code (or manual code), defaulting the grant to View + Message. Revoking a
 * device invalidates it on every transport.
 */
export function ConnectedDevices() {
  const [devices, setDevices] = useState<ConnectedDevice[] | null>(null);
  const [offer, setOffer] = useState<PairingOffer | null>(null);
  const [scopes, setScopes] = useState<string[]>(DEFAULT_SCOPES);
  const [status, setStatus] = useState<string>("pending");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmRevoke, setConfirmRevoke] = useState<string | null>(null);

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
    const timer = window.setInterval(() => {
      api
        .pairingStatus(offer.pairing_id)
        .then((next) => {
          if (next) setStatus(next);
        })
        .catch(() => {});
    }, 1500);
    return () => window.clearInterval(timer);
  }, [offer]);

  async function addPhone() {
    setBusy(true);
    setError(null);
    try {
      setScopes(DEFAULT_SCOPES);
      setStatus("pending");
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
                Scan with Solayge Mobile. You&apos;ll approve the request here.
              </p>
            </div>
            <span
              className="text-[11px] text-ink-subtle"
              role="status"
              aria-live="polite"
            >
              {status === "approved"
                ? "Approved"
                : status === "rejected"
                  ? "Rejected"
                  : "Waiting for phone"}
            </span>
          </div>

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
              <div>
                <div className="text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
                  Desktop fingerprint
                </div>
                <div className="mono mt-1 break-all text-[11px] text-ink-muted">
                  {offer.desktop_fingerprint}
                </div>
              </div>
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
