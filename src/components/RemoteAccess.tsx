import { useCallback, useEffect, useState } from "react";

import { api } from "../api";
import type { RemoteStatus } from "../types";
import { ErrorNote } from "./Field";
import { Icon } from "./Icons";
import { SectionLabel } from "./AgentConfigForm";

/** Human-readable labels for the manager state. */
const STATE_LABELS: Record<string, string> = {
  disabled: "Off",
  listening: "Listening (Iroh only)",
  active: "Ready",
  degraded: "Backup needs attention",
  error: "Error",
  unavailable: "Unavailable",
};

function stateLabel(state: string): string {
  return STATE_LABELS[state] ?? state;
}

/**
 * Settings → Connected devices → Remote access.
 *
 * Enables the desktop's remote listeners: an authenticated loopback WebSocket
 * listener (always) and an optional Solayge-owned Tailscale Serve endpoint that
 * exposes that loopback listener to the user's tailnet. The manager removes only
 * its own Serve entry, never unrelated configuration and never Funnel.
 */
export function RemoteAccess() {
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.remoteStatus());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Keep the live client count and Tailscale status fresh while the panel is
  // open. The listener itself is unaffected by the UI closing.
  useEffect(() => {
    const id = window.setInterval(() => {
      if (!busy) void refresh();
    }, 4000);
    return () => window.clearInterval(id);
  }, [refresh, busy]);

  async function apply(patch: Partial<RemoteStatus> & { enabled?: boolean; tailscale_backup?: boolean }) {
    if (!status) return;
    setBusy(true);
    setError(null);
    try {
      const next = await api.remoteApply({
        enabled: patch.enabled ?? status.enabled,
        tailscale_backup: patch.tailscale_backup ?? status.tailscale_backup,
        https_port: status.https_port,
        local_port: status.local_port,
      });
      setStatus(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function retry() {
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.remoteRetry());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const enabled = status?.enabled ?? false;
  const backup = status?.tailscale_backup ?? false;
  const ts = status?.tailscale;
  const [httpsPort, setHttpsPort] = useState("");

  // TEMPORARY (dev-only): state for the background-push test controls.
  const [testSessionId, setTestSessionId] = useState("");
  const [testPushResult, setTestPushResult] = useState<string | null>(null);
  const [testPushBusy, setTestPushBusy] = useState(false);

  useEffect(() => {
    if (status) setHttpsPort(String(status.https_port));
    // Only reset when the manager reports a different port (after apply/retry).
  }, [status?.https_port]);

  // TEMPORARY (dev-only): send a synthetic question push to every paired phone.
  async function sendTestQuestion() {
    setTestPushBusy(true);
    setTestPushResult(null);
    try {
      setTestPushResult(await api.pushTestQuestion(testSessionId.trim() || null));
    } catch (e) {
      setTestPushResult(String(e));
    } finally {
      setTestPushBusy(false);
    }
  }

  async function applyHttpsPort() {
    if (!status) return;
    const parsed = Number(httpsPort);
    if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
      setError("Enter a port between 1 and 65535");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      setStatus(
        await api.remoteApply({
          enabled: status.enabled,
          tailscale_backup: status.tailscale_backup,
          https_port: parsed,
          local_port: status.local_port,
        }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="space-y-4">
      <div>
        <SectionLabel icon="layers">Remote access</SectionLabel>
        <p className="mt-0.5 text-[11px] leading-relaxed text-ink-subtle">
          Let Solayge Mobile reach this computer. The phone connects over Iroh
          first and can fall back to a private Tailscale connection. Every
          transport still requires phone approval and identity verification.
        </p>
      </div>

      <div className="space-y-3 rounded-lg border border-line bg-well p-4">
        <label className="flex items-start gap-3">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={enabled}
            disabled={busy || !status}
            onChange={() => void apply({ enabled: !enabled })}
          />
          <span>
            <span className="text-[12.5px] font-medium text-ink">
              Enable mobile connections
            </span>
            <span className="block text-[11px] leading-relaxed text-ink-subtle">
              Starts an authenticated listener on 127.0.0.1. It stays running
              while enabled, even with no phones connected.
            </span>
          </span>
        </label>

        <label className="flex items-start gap-3">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={backup}
            disabled={busy || !status || !enabled}
            onChange={() => void apply({ tailscale_backup: !backup })}
          />
          <span>
            <span className="text-[12.5px] font-medium text-ink">
              Enable Tailscale backup
            </span>
            <span className="block text-[11px] leading-relaxed text-ink-subtle">
              Uses the Tailscale you already installed and signed in to. Solayge
              configures only its own Serve endpoint — never Funnel and never
              your other services.
            </span>
          </span>
        </label>

        {enabled && backup && (
          <div className="flex flex-wrap items-end gap-2 border-t border-line pt-3">
            <label className="text-[11px] text-ink-subtle">
              <span className="block">HTTPS port</span>
              <input
                type="number"
                min={1}
                max={65535}
                value={httpsPort}
                disabled={busy}
                onChange={(e) => setHttpsPort(e.target.value)}
                className="mono mt-1 w-24 rounded border border-line bg-surface px-2 py-1 text-[12px] text-ink"
              />
            </label>
            <button
              className="btn btn-ghost !px-2.5 !py-1 text-[11px]"
              onClick={() => void applyHttpsPort()}
              disabled={busy || !status}
            >
              Apply port
            </button>
            <p className="basis-full text-[11px] leading-relaxed text-ink-subtle">
              Use 443 for <span className="mono">https://&lt;device&gt;.ts.net/</span>, or
              another port if 443 is already serving something else.
            </p>
          </div>
        )}
      </div>

      {status && (
        <div className="overflow-hidden rounded-lg border border-line bg-well">
          <StatusRow
            label="WebSocket listener"
            value={
              status.listener_running
                ? `Running on 127.0.0.1:${status.listener_port}`
                : "Stopped"
            }
            ok={status.listener_running}
          />
          <StatusRow
            label="Connected phones"
            value={`${status.connected_clients} ${
              status.connected_clients === 1 ? "phone" : "phones"
            }`}
            ok={status.connected_clients > 0}
          />
          <StatusRow
            label="Tailscale"
            value={tailscaleSummary(status)}
            ok={Boolean(ts?.installed && ts.online)}
          />
          <StatusRow
            label="Serve endpoint"
            value={
              status.serve_owned && status.serve_url
                ? status.serve_url
                : status.tailscale_backup
                  ? "Not configured"
                  : "Backup disabled"
            }
            ok={status.serve_owned}
          />
          <StatusRow label="State" value={stateLabel(status.state)} ok={status.state === "active" || status.state === "listening"} />
        </div>
      )}

      {(status?.last_error || ts?.message) && (
        <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-[11px] leading-relaxed text-ink">
          <div className="font-medium">Needs attention</div>
          <p className="mt-0.5 text-ink-subtle">{status?.last_error ?? ts?.message}</p>
          {ts?.installed === false && (
            <p className="mt-1 text-ink-subtle">
              Install Tailscale from tailscale.com/download, sign in, then retry.
            </p>
          )}
          {ts?.installed && !ts.authenticated && (
            <p className="mt-1 text-ink-subtle">
              Open Tailscale on this computer and sign in, then retry.
            </p>
          )}
        </div>
      )}

      <div className="flex items-center gap-2">
        <button
          className="btn btn-ghost !px-2.5 !py-1 text-[11px]"
          onClick={() => void retry()}
          disabled={busy || !status}
        >
          <Icon name="refresh" className="h-3 w-3" />
          Retry &amp; re-check
        </button>
        <button
          className="btn btn-ghost !px-2.5 !py-1 text-[11px]"
          onClick={() => void refresh()}
          disabled={busy}
        >
          Refresh status
        </button>
      </div>

      <ErrorNote error={error} />

      {/* TEMPORARY (dev-only): fire a synthetic "Agent has a question" push at
          every paired phone to test FCM delivery and tap deep-linking. */}
      <div className="rounded-lg border border-dashed border-line p-3">
        <div className="text-[11px] font-medium text-ink">
          Developer: test background push
        </div>
        <p className="mt-0.5 text-[11px] leading-relaxed text-ink-subtle">
          Sends a synthetic &ldquo;Agent has a question&rdquo; push to every paired
          phone. Requires the gateway URL/token in the desktop environment and a
          phone that has registered a push token.
        </p>
        <div className="mt-2 flex flex-wrap items-end gap-2">
          <label className="text-[11px] text-ink-subtle">
            <span className="block">Session id (optional)</span>
            <input
              value={testSessionId}
              disabled={testPushBusy}
              onChange={(e) => setTestSessionId(e.target.value)}
              placeholder="ses_…"
              className="mono mt-1 w-56 rounded border border-line bg-surface px-2 py-1 text-[12px] text-ink"
            />
          </label>
          <button
            className="btn btn-ghost !px-2.5 !py-1 text-[11px]"
            onClick={() => void sendTestQuestion()}
            disabled={testPushBusy || !status}
          >
            <Icon name="zap" className="h-3 w-3" />
            Send test question push
          </button>
        </div>
        {testPushResult && (
          <p className="mt-2 text-[11px] leading-relaxed text-ink-subtle">
            {testPushResult}
          </p>
        )}
      </div>
    </section>
  );
}

function StatusRow({
  label,
  value,
  ok,
}: {
  label: string;
  value: string;
  ok: boolean;
}) {
  return (
    <div className="flex items-center gap-3 border-b border-line px-3 py-2 last:border-b-0">
      <span
        aria-hidden="true"
        className={`h-2 w-2 shrink-0 rounded-full ${ok ? "bg-success" : "bg-ink-faint"}`}
      />
      <span className="w-36 shrink-0 text-[11px] uppercase tracking-wide text-ink-subtle">
        {label}
      </span>
      <span className="min-w-0 flex-1 break-all text-[12px] text-ink">{value}</span>
    </div>
  );
}

function tailscaleSummary(status: RemoteStatus): string {
  // `tailscale` is absent in a build without the network feature (the stub
  // reports only a subset of the status), so tolerate it rather than crash.
  const ts = status.tailscale;
  if (!ts?.installed) return "Not installed";
  const name = ts.dns_name ?? "no MagicDNS name";
  const state = ts.backend_state || "unknown";
  return `${state} · ${name}${ts.version ? ` · v${ts.version}` : ""}`;
}
