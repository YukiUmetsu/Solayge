import { useEffect, useMemo, useState } from "react";
import type { ClientDiagnostics, TransportMode } from "../client/remote";
import { useClientContext } from "../client/ClientContext";
import {
  clearCachedData,
  DEFAULT_RETENTION_DAYS,
  loadRetention,
  retentionFor,
  setProjectRetention,
  type RetentionConfig,
} from "../client/cache";
import { connectionStatusView } from "../client/status";
import { useNotifications } from "../client/NotificationCenter";
import { pushToken } from "../client/push";
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
  const { settings, permission, supported, updateSettings, requestPermission, sendTest } =
    useNotifications();
  const [diagnostics, setDiagnostics] = useState<ClientDiagnostics | null>(null);
  const [copied, setCopied] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [records, setRecords] = useState<PairedDesktopRecord[]>([]);
  const [retention, setRetention] = useState<RetentionConfig>(() => loadRetention());
  const [cacheCleared, setCacheCleared] = useState(false);
  const [testSent, setTestSent] = useState(false);

  function updateRetention(projectId: string, days: number) {
    setRetention(setProjectRetention(projectId, days));
  }

  function clearCache() {
    clearCachedData(desktopId);
    setCacheCleared(true);
  }

  useEffect(() => {
    pairingApi
      .pairedDesktops()
      .then(setRecords)
      .catch(() => setRecords([]));
  }, [desktops.length]);

  async function forget(desktopId: string) {
    try {
      // Best effort: stop this phone's background alerts for that desktop.
      const token = await pushToken();
      if (token) {
        try {
          await client.unregisterPush(token.token);
        } catch {
          // The desktop may be unreachable; forgetting locally still proceeds.
        }
      }
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

      <section className="detail__section" aria-label="Notifications">
        <h2 className="detail__section-title">Notifications</h2>
        <p className="detail__sub">
          Alerts for agent questions, task completions, and failures. A background connection keeps
          watching the desktop so it can alert you while the app is not in front of you.
        </p>
        {!supported ? (
          <p className="detail__sub">Notifications are not available in this build.</p>
        ) : (
          <>
            <label className="radio-row">
              <input
                type="checkbox"
                checked={settings.enabled}
                onChange={(event) => {
                  if (!event.target.checked) {
                    updateSettings({ enabled: false });
                    return;
                  }
                  void (async () => {
                    const result = await requestPermission();
                    if (result === "granted") updateSettings({ enabled: true });
                    else if (result === "unsupported") updateSettings({ enabled: true });
                    // A denial leaves notifications off; the help text points to
                    // the system settings.
                  })();
                }}
              />
              <span>
                <span className="radio-row__label">Enable notifications</span>
                <span className="radio-row__help">
                  {permission === "denied"
                    ? "Blocked. Allow Solayge notifications in your phone's settings."
                    : "Ask this phone to alert you when something needs you."}
                </span>
              </span>
            </label>

            {settings.enabled ? (
              <>
                <fieldset className="fieldset">
                  <legend className="radio-row__label">Notify me about</legend>
                  <label className="radio-row">
                    <input
                      type="checkbox"
                      checked={settings.onQuestions}
                      onChange={(event) => updateSettings({ onQuestions: event.target.checked })}
                    />
                    <span className="radio-row__label">Questions and permission requests</span>
                  </label>
                  <label className="radio-row">
                    <input
                      type="checkbox"
                      checked={settings.onCompleted}
                      onChange={(event) => updateSettings({ onCompleted: event.target.checked })}
                    />
                    <span className="radio-row__label">Task completions</span>
                  </label>
                  <label className="radio-row">
                    <input
                      type="checkbox"
                      checked={settings.onFailed}
                      onChange={(event) => updateSettings({ onFailed: event.target.checked })}
                    />
                    <span className="radio-row__label">Task failures</span>
                  </label>
                </fieldset>

                <label className="radio-row">
                  <input
                    type="checkbox"
                    checked={settings.sound}
                    onChange={(event) => updateSettings({ sound: event.target.checked })}
                  />
                  <span>
                    <span className="radio-row__label">Sound</span>
                    <span className="radio-row__help">
                      Play a chime and the system notification sound.
                    </span>
                  </span>
                </label>

                <label className="cache-row">
                  <span className="cache-row__name">Volume</span>
                  <input
                    className="cache-row__input"
                    type="range"
                    min={0}
                    max={1}
                    step={0.1}
                    value={settings.volume}
                    onChange={(event) => updateSettings({ volume: Number(event.target.value) })}
                    aria-label="Notification volume"
                  />
                </label>

                <button
                  className="btn btn--small"
                  type="button"
                  onClick={() => {
                    void (async () => setTestSent(await sendTest()))();
                  }}
                >
                  {testSent ? "Test sent" : "Send test notification"}
                </button>
              </>
            ) : null}
          </>
        )}
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

      <section className="detail__section" aria-label="Task data caching">
        <h2 className="detail__section-title">Task data caching</h2>
        <p className="detail__sub">
          How long task data is kept on this device for offline viewing. Default {DEFAULT_RETENTION_DAYS}{" "}
          days; set 0 to keep nothing. Cached data is read-only and never used to authorize actions.
        </p>
        <ul className="cache-list">
          {data.projects.map((project) => (
            <li key={project.id} className="cache-row">
              <span className="cache-row__name">{project.name}</span>
              <label className="cache-row__days">
                <input
                  type="number"
                  className="cache-row__input"
                  min={0}
                  max={3650}
                  value={retentionFor(project.id, retention)}
                  onChange={(event) => updateRetention(project.id, Number(event.target.value))}
                  aria-label={`Retention for ${project.name} in days`}
                />
                <span>days</span>
              </label>
            </li>
          ))}
          {data.projects.length === 0 ? (
            <li className="detail__sub">Connect to a desktop to configure per-project caching.</li>
          ) : null}
        </ul>
        <button className="btn btn--small" type="button" onClick={clearCache}>
          {cacheCleared ? "Cached data cleared" : "Clear cached data"}
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
