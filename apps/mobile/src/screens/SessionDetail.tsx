import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  CommandStatusName,
  FilesChangedSummary,
  SessionDetail as SessionDetailDto,
  SessionEvent,
} from "@solayge/shared-types";

import { useClientContext } from "../client/ClientContext";
import { formatChange, formatElapsed, relativeTime } from "../client/format";
import { commandIdFor } from "../client/mock";
import { RemoteError } from "../client/remote";
import {
  agentLabel,
  currentStep,
  isDefiniteRejection,
  sessionStatusView,
  summarizeEvent,
} from "../client/status";
import { Composer, type SendState } from "../components/Composer";
import { EmptyState } from "../components/EmptyState";
import { ErrorState } from "../components/ErrorState";
import { Markdown } from "../components/Markdown";
import { Modal } from "../components/Modal";
import { StatusPill } from "../components/StatusPill";

interface OutboxEntry extends SendState {
  text: string;
  commandId: string;
}

const TERMINAL: CommandStatusName[] = ["succeeded", "failed", "canceled"];

/**
 * Session/chat detail: summary, current step, activity timeline, an expandable
 * full transcript, and a sticky composer (mobile spec section 7.2 C).
 */
export function SessionDetail({
  sessionId,
  onOpenApproval,
  pollMs = 1500,
}: {
  sessionId: string;
  onOpenApproval: (approvalId: string) => void;
  pollMs?: number;
}) {
  const { client, data, isReadOnly, now, refresh } = useClientContext();
  const [session, setSession] = useState<SessionDetailDto | null>(null);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [files, setFiles] = useState<FilesChangedSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [showTranscript, setShowTranscript] = useState(false);
  const [outbox, setOutbox] = useState<OutboxEntry | null>(null);
  const [confirmStop, setConfirmStop] = useState(false);
  const [stopping, setStopping] = useState(false);
  const pollRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cached = useMemo(
    () => data.sessions.find((entry) => entry.id === sessionId) ?? null,
    [data.sessions, sessionId],
  );
  const approvals = useMemo(
    () => data.approvals.filter((approval) => approval.session_id === sessionId),
    [data.approvals, sessionId],
  );

  const load = useCallback(async () => {
    if (isReadOnly) {
      setLoading(false);
      return;
    }
    setLoading(true);
    setLoadError(null);
    try {
      const [detail, batch, changed] = await Promise.all([
        client.getSession(sessionId),
        client.subscribeEvents(sessionId, 0),
        client.filesChangedSummary(sessionId),
      ]);
      setSession(detail);
      setEvents(batch.events);
      setFiles(changed);
    } catch (caught) {
      setLoadError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setLoading(false);
    }
  }, [client, isReadOnly, sessionId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(
    () => () => {
      if (pollRef.current) clearTimeout(pollRef.current);
    },
    [],
  );

  const reloadEvents = useCallback(async () => {
    try {
      const from = events.length > 0 ? events[events.length - 1].seq + 1 : 0;
      const batch = await client.subscribeEvents(sessionId, from);
      if (batch.events.length > 0) setEvents((previous) => [...previous, ...batch.events]);
    } catch {
      // A refresh failure leaves the last-known transcript in place.
    }
  }, [client, events, sessionId]);

  const pollCommand = useCallback(
    (commandId: string) => {
      const tick = async () => {
        try {
          const payload = await client.getCommandStatus(commandId);
          setOutbox((previous) =>
            previous && previous.commandId === commandId
              ? {
                  ...previous,
                  status: payload.status,
                  errorCode: payload.error?.code ?? null,
                  errorMessage: payload.error?.message ?? null,
                }
              : previous,
          );
          if (TERMINAL.includes(payload.status)) return;
        } catch {
          // Keep showing the last known send status; do not fabricate success.
        }
        pollRef.current = setTimeout(() => void tick(), pollMs);
      };
      pollRef.current = setTimeout(() => void tick(), pollMs);
    },
    [client, pollMs],
  );

  const dispatch = useCallback(
    async (text: string, commandId: string) => {
      setOutbox({ text, commandId, status: "created", errorCode: null, errorMessage: null });
      try {
        const receipt = await client.sendPrompt(sessionId, text, commandId);
        setOutbox({
          text,
          commandId,
          status: receipt.status,
          errorCode: null,
          errorMessage: null,
        });
        void reloadEvents();
        if (pollMs > 0) pollCommand(commandId);
      } catch (caught) {
        const code = caught instanceof RemoteError ? caught.code : "internal";
        const message = caught instanceof Error ? caught.message : String(caught);
        // A definite rejection is safe to retry; a transport failure is not.
        const status: CommandStatusName = isDefiniteRejection(code) ? "failed" : "unknown";
        setOutbox({ text, commandId, status, errorCode: code, errorMessage: message });
      }
    },
    [client, pollMs, pollCommand, reloadEvents, sessionId],
  );

  async function confirmStopAction() {
    setStopping(true);
    try {
      await client.cancelSession(sessionId);
      setConfirmStop(false);
      await load();
      await refresh();
    } catch (caught) {
      setLoadError(caught instanceof Error ? caught.message : String(caught));
      setConfirmStop(false);
    } finally {
      setStopping(false);
    }
  }

  const header = session ?? cached;
  if (loading && !header) {
    return (
      <p className="loading" role="status">
        Loading session…
      </p>
    );
  }
  if (!header) {
    if (loadError) return <ErrorState message={loadError} onRetry={() => void load()} />;
    return <EmptyState icon="•" title="Session unavailable" body="This session is no longer on the desktop." />;
  }

  const statusView = sessionStatusView(header.status, header.pending_decisions);
  const step = currentStep(events);
  const timeline = events.filter((event) => event.kind !== "message");
  const canStop =
    !isReadOnly && (header.status === "running" || header.status === "waiting") && !stopping;

  return (
    <article className="detail">
      <header className="detail__head">
        <h1 className="detail__title">{header.title}</h1>
        <div className="detail__meta">
          <span className="detail__agent">{agentLabel(header.agent)}</span>
          <StatusPill view={statusView} size="sm" />
          <button
            className="detail__refresh"
            type="button"
            onClick={() => void load()}
            disabled={loading || isReadOnly}
            aria-label="Reload this task from the desktop"
            title="Reload this task from the desktop"
          >
            {loading ? "…" : "⟳"}
          </button>
        </div>
      </header>

      {loadError ? (
        <p className="detail__notice" role="status">
          {loadError}
        </p>
      ) : null}

      <section className="detail__section" aria-label="Latest summary">
        <h2 className="detail__section-title">Latest summary</h2>
        {session?.summary ? (
          <Markdown>{session.summary}</Markdown>
        ) : (
          <p className="detail__sub">No summary reported yet.</p>
        )}
        <p className="detail__sub">Updated {relativeTime(header.updated_at, now())}</p>
      </section>

      {session && (session.prompt || session.log_tail) ? (
        <details className="disclosure detail__details">
          <summary>Task details</summary>
          {session.prompt ? (
            <div className="detail__block">
              <h3 className="detail__block-title">Prompt</h3>
              <pre className="detail__code mono">{session.prompt}</pre>
            </div>
          ) : null}
          {session.log_tail ? (
            <div className="detail__block">
              <h3 className="detail__block-title">Recent log</h3>
              <pre className="detail__code mono">{session.log_tail}</pre>
            </div>
          ) : null}
        </details>
      ) : null}

      <section className="detail__section" aria-label="Current step">
        <h2 className="detail__section-title">Current step</h2>
        {step ? (
          <p className="detail__step">
            <span aria-hidden="true">{step.icon} </span>
            {step.label}
            {step.detail ? <span className="detail__step-detail mono">{step.detail}</span> : null}
          </p>
        ) : (
          <p className="detail__sub">No active step reported.</p>
        )}
        {header.status === "running" ? (
          <p className="detail__sub">{formatElapsed(header.started_at, now())}</p>
        ) : null}
      </section>

      {approvals.length > 0 ? (
        <section className="detail__section" aria-label="Pending decisions">
          <h2 className="detail__section-title">Waiting on you</h2>
          <ul className="approval-mini-list">
            {approvals.map((approval) => (
              <li key={approval.id}>
                <button
                  className="approval-mini"
                  type="button"
                  onClick={() => onOpenApproval(approval.id)}
                >
                  <span aria-hidden="true">{approval.kind === "permission" ? "⚠" : "?"} </span>
                  {approval.title}
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="detail__section" aria-label="Activity timeline">
        <h2 className="detail__section-title">Activity</h2>
        {timeline.length === 0 ? (
          <p className="detail__sub">No activity recorded yet.</p>
        ) : (
          <ol className="timeline">
            {timeline.map((event) => {
              const summary = summarizeEvent(event);
              return (
                <li key={event.event_id} className="timeline__item">
                  <span className="timeline__icon" aria-hidden="true">
                    {summary.icon}
                  </span>
                  <span className="timeline__body">
                    <span className="timeline__label">{summary.label}</span>
                    {summary.detail ? (
                      <span className="timeline__detail mono">{summary.detail}</span>
                    ) : null}
                  </span>
                  <span className="timeline__time">{relativeTime(event.timestamp, now())}</span>
                </li>
              );
            })}
          </ol>
        )}
      </section>

      <button
        className="btn detail__toggle"
        type="button"
        aria-expanded={showTranscript}
        onClick={() => setShowTranscript((value) => !value)}
      >
        {showTranscript ? "Hide full conversation" : "View full conversation"}
      </button>

      {showTranscript ? (
        <section className="detail__section" aria-label="Full conversation">
          {events.length === 0 ? (
            <p className="detail__sub">The transcript is not available offline.</p>
          ) : (
            <ol className="transcript">
              {events.map((event) => {
                const summary = summarizeEvent(event);
                if (summary.isMessage) {
                  return (
                    <li
                      key={event.event_id}
                      className={`transcript__message transcript__message--${summary.role}`}
                    >
                      <span className="transcript__role">
                        {summary.role === "user" ? "You" : summary.role === "agent" ? "Agent" : "System"}
                      </span>
                      <span className="transcript__text">{summary.label}</span>
                      <span className="transcript__time">{relativeTime(event.timestamp, now())}</span>
                    </li>
                  );
                }
                return (
                  <li key={event.event_id} className="transcript__event">
                    <span aria-hidden="true">{summary.icon} </span>
                    {summary.label}
                    {summary.detail ? <span className="mono"> {summary.detail}</span> : null}
                  </li>
                );
              })}
            </ol>
          )}
        </section>
      ) : null}

      <section className="detail__section" aria-label="Files changed">
        <h2 className="detail__section-title">Files changed</h2>
        {files && files.files.length > 0 ? (
          <ul className="file-list">
            {files.files.map((file) => (
              <li key={file.path} className="file-row">
                <span className="file-row__path mono">{file.path}</span>
                <span className="file-row__status">{file.status}</span>
                <span className="file-row__change">
                  {formatChange(file.additions, file.deletions)}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="detail__sub">
            {files
              ? "No files changed."
              : isReadOnly
                ? "Change details are unavailable offline."
                : "No files changed."}
          </p>
        )}
      </section>

      <div className="detail__composer-dock">
        <Composer
          onSend={(text) => void dispatch(text, commandIdFor(client.desktopId, sessionId, text))}
          onRetry={() => {
            if (outbox) void dispatch(outbox.text, outbox.commandId);
          }}
          onStop={() => setConfirmStop(true)}
          sendState={outbox}
          canStop={canStop}
          readOnly={isReadOnly}
          disabledReason="You're offline. Sending and stopping are disabled so nothing dangerous is queued."
        />
      </div>

      <Modal
        open={confirmStop}
        title="Stop this task?"
        onClose={() => setConfirmStop(false)}
        footer={
          <>
            <button className="btn" type="button" onClick={() => setConfirmStop(false)}>
              Keep running
            </button>
            <button
              className="btn btn-danger"
              type="button"
              onClick={() => void confirmStopAction()}
              disabled={stopping}
            >
              {stopping ? "Stopping…" : "Stop task"}
            </button>
          </>
        }
      >
        <p>
          The desktop will be asked to cancel this task. Agent work already completed is not undone.
        </p>
      </Modal>
    </article>
  );
}
