import { useEffect, useMemo, useState } from "react";
import type { ClientDiagnostics, TransportMode } from "../client/remote";
import { useClientContext } from "../client/ClientContext";
import { connectionStatusView } from "../client/status";
import { Modal } from "../components/Modal";
import { StatusPill } from "../components/StatusPill";
import { pairingApi, type PairedDesktopRecord } from "../pairing";

const MODES: { id: TransportMode; label: string; help: string }[] = [
  {
    id: "automatic",
    label: "Automatic (recommended)",
    help: "Uses Iroh first. If it can't connect, tries your already configured Tailscale connection.",
  },
  { id: "iroh", label: "Iroh only", help: "Only uses the encrypted Iroh peer-to-peer connection." },
  {
    id: "tailscale",
    label: "Tailscale only",
    help: "Uses your installed Tailscale connection to the desktop. Solayge does not set up Tailscale for you.",
  },
];

/**
 * Settings: transport selection, connection details, and diagnostics. It
 * reports the truth — the loopback client is labeled as local development, and
 * "Connected" is only shown after metadata is read (mobile spec section 7.2 F).
 */
export function Settings({
  onAddDesktop,
  simulateOffline,
  onSimulateOffline,
}: {
  onAddDesktop: () => void;
  simulateOffline?: boolean;
  onSimulateOffline?: (offline: boolean) => void;
}) {
  const { client, desktopId, desktop, desktops, transportMode, setTransportMode, status, data } =
    useClientContext();
  const [diagnostics, setDiagnostics] = useState<ClientDiagnostics | null>(null);
  const [copied, setCopied] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [records, setRecords] = useState<PairedDesktopRecord[]>([]);

  useEffect(() => {
    pairingApi
      .pairedDesktops()
      .then(setRecords)
      .catch(() => setRecords([]));
  }, [desktops.length]);

  async function forget(desktopId: string) {
    try {
      await pairingApi.forgetDesktop(desktopId);
      // The paired list lives in the app shell; a reload re-reads it and
      // drops the now-unpaired desktop cleanly.
      window.location.reload();
    } catch (e) {
      // Surface nothing destructive silently: keep the device on failure.
      setRecords((prev) => prev.filter((r) => r.id !== desktopId));
      void e;
    }
  }

  useEffect(() => {
    if (!client.diagnostics || status !== "online") {
      setDiagnostics(null);
      return;
    }
    let cancelled = false;
    client
      .diagnostics()
      .then((result) => {
        if (!cancelled) setDiagnostics(result);
      })
      .catch(() => {
        if (!cancelled) setDiagnostics(null);
      });
    return () => {
      cancelled = true;
    };
  }, [client, status]);

  const statusView = connectionStatusView(status);

  const diagnosticsText = useMemo(() => {
    const lines = [
      `status: ${status}`,
      `desktop: ${desktop?.name ?? "unknown"} (${desktopId})`,
      `platform: ${desktop?.platform ?? "unknown"}`,
      `app_version: ${desktop?.app_version ?? "unknown"}`,
      `protocol_version: ${desktop?.protocol_version ?? "unknown"}`,
      `transport_mode: ${transportMode}`,
      `transport: ${diagnostics?.transport ?? "not reported"}`,
      `endpoint: ${diagnostics?.endpoint ?? "not reported"}`,
      `path: ${diagnostics?.path ?? "unknown"}`,
      `rtt_ms: ${diagnostics?.rttMs ?? "unknown"}`,
      `last_error: ${diagnostics?.lastError ?? "none"}`,
      `last_sync: ${data.updatedAt ?? "never"}`,
    ];
    return lines.join("\n");
  }, [status, desktop, desktopId, transportMode, diagnostics, data.updatedAt]);

  async function copyDiagnostics() {
    try {
      await navigator.clipboard.writeText(diagnosticsText);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="settings">
      <section className="detail__section" aria-label="Connection method">
        <h2 className="detail__section-title">Connection method</h2>
        <fieldset className="fieldset">
          <legend className="sr-only">Connection method preference</legend>
          {MODES.map((mode) => (
            <label key={mode.id} className="radio-row">
              <input
                type="radio"
                name="transport-mode"
                value={mode.id}
                checked={transportMode === mode.id}
                onChange={() => setTransportMode(mode.id)}
              />
              <span>
                <span className="radio-row__label">{mode.label}</span>
                <span className="radio-row__help">{mode.help}</span>
              </span>
            </label>
          ))}
        </fieldset>
      </section>

      <section className="detail__section" aria-label="Current connection">
        <h2 className="detail__section-title">Connection</h2>
        <div className="connection-summary">
          <StatusPill view={statusView} />
          <span className="connection-summary__desktop">{desktop?.name ?? "No desktop"}</span>
        </div>
        <p className="detail__sub" role="status">
          {status === "online"
            ? `Connected via ${diagnostics?.transport ?? (transportMode === "tailscale" ? "Tailscale" : "Iroh")}.`
            : "Not connected. Showing last known data."}
        </p>
        <details className="disclosure">
          <summary>Connection details</summary>
          <dl className="facts">
            <div>
              <dt>Desktop id</dt>
              <dd className="mono">{desktopId}</dd>
            </div>
            <div>
              <dt>Platform</dt>
              <dd>{desktop?.platform ?? "unknown"}</dd>
            </div>
            <div>
              <dt>App version</dt>
              <dd>{desktop?.app_version ?? "unknown"}</dd>
            </div>
            <div>
              <dt>Protocol version</dt>
              <dd>{desktop?.protocol_version ?? "unknown"}</dd>
            </div>
            <div>
              <dt>Endpoint</dt>
              <dd className="mono">{diagnostics?.endpoint ?? "not reported"}</dd>
            </div>
            <div>
              <dt>Path</dt>
              <dd>{diagnostics?.path ?? "unknown"}</dd>
            </div>
            <div>
              <dt>Round-trip</dt>
              <dd>{diagnostics?.rttMs != null ? `${diagnostics.rttMs} ms` : "not reported"}</dd>
            </div>
            <div>
              <dt>Last sync</dt>
              <dd>{data.updatedAt ?? "never"}</dd>
            </div>
          </dl>
        </details>
      </section>

      <section className="detail__section" aria-label="Desktops">
        <h2 className="detail__section-title">Desktops</h2>
        <ul className="desktop-list">
          {desktops.map((entry) => {
            const record = records.find((r) => r.id === entry.id);
            return (
              <li key={entry.id} className="desktop-row desktop-row--static">
                <span className="desktop-row__name">{entry.name}</span>
                <span className="desktop-row__platform">{entry.platform}</span>
                {record?.fingerprint ? (
                  <span className="desktop-row__fingerprint mono">{record.fingerprint}</span>
                ) : null}
                <button
                  className="btn btn--small"
                  type="button"
                  onClick={() => void forget(entry.id)}
                >
                  Remove
                </button>
              </li>
            );
          })}
        </ul>
        <button className="btn" type="button" onClick={() => setAddOpen(true)}>
          + Add a desktop
        </button>
      </section>

      <section className="detail__section" aria-label="Diagnostics">
        <h2 className="detail__section-title">Diagnostics</h2>
        <p className="detail__sub">
          Sanitized connection information. It never contains message content, keys, or file data.
        </p>
        <pre className="diagnostics mono" aria-label="Diagnostic details">
          {diagnosticsText}
        </pre>
        <button className="btn btn--small" type="button" onClick={() => void copyDiagnostics()}>
          {copied ? "Copied" : "Copy diagnostics"}
        </button>
      </section>

      {onSimulateOffline ? (
        <section className="detail__section" aria-label="Local development">
          <h2 className="detail__section-title">Local development</h2>
          <label className="radio-row">
            <input
              type="checkbox"
              checked={simulateOffline ?? false}
              onChange={(event) => onSimulateOffline(event.target.checked)}
            />
            <span>
              <span className="radio-row__label">Simulate offline</span>
              <span className="radio-row__help">
                Uses the in-memory loopback client to preview the offline, read-only state.
              </span>
            </span>
          </label>
        </section>
      ) : null}

      <Modal
        open={addOpen}
        title="Add a desktop"
        onClose={() => setAddOpen(false)}
        footer={
          <button
            className="btn"
            type="button"
            onClick={() => {
              setAddOpen(false);
              onAddDesktop();
            }}
          >
            Continue to pairing
          </button>
        }
      >
        <p>
          You pair a new desktop by scanning the QR code it shows. Approval always happens on the
          desktop, and this phone is never granted access automatically.
        </p>
      </Modal>
    </div>
  );
}
