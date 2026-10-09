import { useEffect, useMemo, useState } from "react";
import type { ApprovalDecision, ApprovalSummary, CommandStatusName } from "@solayge/shared-types";

import { useClientContext } from "../client/ClientContext";
import {
  affectedScope,
  classification,
  exactOperation,
  limitations,
  requiresConfirmation,
  riskLabel,
} from "../client/approval";
import { formatDateTime, formatExpiry } from "../client/format";
import { commandIdFor } from "../client/mock";
import { RemoteError } from "../client/remote";
import { isDefiniteRejection, sendStatusView } from "../client/status";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";
import { StatusPill } from "../components/StatusPill";

interface AnswerSendState {
  status: CommandStatusName;
  errorCode: string | null;
  errorMessage: string | null;
}

/**
 * The decision detail. For a permission request, Deny and Approve once are
 * equally clear and exactly one definitive decision is recorded. For a
 * question, answering sends a normal message and is explicitly *not*
 * authorization (mobile spec section 7.2 D).
 */
export function ApprovalDetail({ approvalId }: { approvalId: string }) {
  const { client, data, desktop, isReadOnly, now, refresh } = useClientContext();
  const nowSec = now();

  const [fetched, setFetched] = useState<ApprovalSummary | null>(null);
  const [resolved, setResolved] = useState<ApprovalDecision | "gone" | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [confirming, setConfirming] = useState<ApprovalDecision | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [answer, setAnswer] = useState("");
  const [answerState, setAnswerState] = useState<AnswerSendState | null>(null);

  const approval = useMemo(
    () => data.approvals.find((entry) => entry.id === approvalId) ?? fetched,
    [data.approvals, fetched, approvalId],
  );

  useEffect(() => {
    if (approval || isReadOnly || resolved) return;
    let cancelled = false;
    client
      .getApproval(approvalId)
      .then((result) => {
        if (!cancelled) setFetched(result);
      })
      .catch(() => {
        if (!cancelled) setResolved("gone");
      });
    return () => {
      cancelled = true;
    };
  }, [approval, approvalId, client, isReadOnly, resolved]);

  const projectName = useMemo(() => {
    const match = data.projects.find((project) => project.id === approval?.project_id);
    return match?.name ?? approval?.project_id ?? "";
  }, [data.projects, approval]);

  const agent = useMemo(() => {
    const session = data.sessions.find((entry) => entry.id === approval?.session_id);
    return session?.agent ?? "agent";
  }, [data.sessions, approval]);

  async function decide(decision: ApprovalDecision) {
    if (!approval || resolved || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await client.respondApproval(approval.id, decision);
      setResolved(decision);
      setConfirming(null);
      await refresh();
    } catch (caught) {
      // A request that is already gone is treated as resolved, not as a
      // successful replay.
      if (caught instanceof RemoteError && caught.code === "not_found") {
        setResolved("gone");
      } else {
        setError(caught instanceof Error ? caught.message : String(caught));
      }
    } finally {
      setSubmitting(false);
    }
  }

  async function sendAnswer() {
    if (!approval || !answer.trim() || isReadOnly) return;
    setAnswerState({ status: "created", errorCode: null, errorMessage: null });
    try {
      const receipt = await client.sendPrompt(
        approval.session_id,
        answer.trim(),
        commandIdFor(client.desktopId, approval.session_id, answer.trim()),
      );
      setAnswerState({ status: receipt.status, errorCode: null, errorMessage: null });
      setAnswer("");
    } catch (caught) {
      const code = caught instanceof RemoteError ? caught.code : "internal";
      setAnswerState({
        status: isDefiniteRejection(code) ? "failed" : "unknown",
        errorCode: code,
        errorMessage: caught instanceof Error ? caught.message : String(caught),
      });
    }
  }

  if (resolved) {
    const question = approval?.kind === "question";
    return (
      <article className="detail">
        <EmptyState
          icon={resolved === "deny" ? "⊘" : "✓"}
          title={
            resolved === "deny" ? "Denied" : resolved === "gone" ? "Request resolved" : "Approved once"
          }
          body={
            question
              ? "This question has been answered."
              : "This request has one definitive decision. It cannot be changed here."
          }
        />
      </article>
    );
  }

  if (!approval) {
    // Offline with nothing cached: we cannot know whether the request is still
    // pending, so never claim it was resolved.
    return isReadOnly ? (
      <EmptyState
        icon="⚠"
        title="Request unavailable offline"
        body="Reconnect to the desktop to review this request."
      />
    ) : (
      <EmptyState
        icon="✓"
        title="Request resolved"
        body="This request is no longer pending on the desktop."
      />
    );
  }

  const isQuestion = approval.kind === "question";
  const operation = exactOperation(approval);
  const cls = classification(approval);
  const expiry = formatExpiry(approval.expires_at, nowSec);
  const expired = expiry === "expired";

  return (
    <article className="detail">
      <header className="detail__head">
        <span className={`kind-badge kind-badge--${isQuestion ? "question" : "permission"}`}>
          <span aria-hidden="true">{isQuestion ? "?" : "⚠"} </span>
          {isQuestion ? "Question" : "Permission request"}
        </span>
        <h1 className="detail__title">{approval.title}</h1>
      </header>

      <div className="decision-flags">
        <StatusPill view={{ label: cls.label, icon: cls.tone === "warning" ? "✎" : "👁", tone: cls.tone }} size="sm" />
        <StatusPill view={{ label: riskLabel(approval.risk), icon: "◆", tone: approval.risk === "high" ? "danger" : approval.risk === "medium" ? "warning" : "neutral" }} size="sm" />
        {expiry ? (
          <StatusPill
            view={{ label: `Expiry ${expiry}`, icon: expired ? "!" : "◷", tone: expired ? "danger" : "neutral" }}
            size="sm"
          />
        ) : null}
      </div>

      <dl className="facts">
        <div>
          <dt>Desktop</dt>
          <dd>{desktop?.name ?? "This desktop"}</dd>
        </div>
        <div>
          <dt>Project / workspace</dt>
          <dd>
            {projectName}
            <span className="facts__sub mono">{approval.project_id}</span>
          </dd>
        </div>
        <div>
          <dt>Agent</dt>
          <dd>{agent}</dd>
        </div>
        {!isQuestion ? (
          <div>
            <dt>Exact operation</dt>
            <dd>
              {operation ? (
                <code className="command-block">{operation}</code>
              ) : (
                "No command string supplied"
              )}
            </dd>
          </div>
        ) : null}
        <div>
          <dt>Reason</dt>
          <dd>{approval.message ?? "No reason supplied."}</dd>
        </div>
        <div>
          <dt>Affected scope</dt>
          <dd>{affectedScope(approval)}</dd>
        </div>
        <div>
          <dt>Limitations</dt>
          <dd>{limitations(approval)}</dd>
        </div>
        <div>
          <dt>Requested</dt>
          <dd>{formatDateTime(approval.requested_at)}</dd>
        </div>
      </dl>

      {isReadOnly ? (
        <p className="detail__notice" role="status">
          You're offline. Decisions are disabled so no authorization is sent without a live, verified
          connection.
        </p>
      ) : null}
      {expired && !isReadOnly ? (
        <p className="detail__notice" role="status">
          This request expired. Ask the agent to raise it again if you still want to proceed.
        </p>
      ) : null}
      {error ? (
        <p className="detail__notice detail__notice--error" role="alert">
          {error}
        </p>
      ) : null}

      {isQuestion ? (
        <section className="detail__section" aria-label="Answer the question">
          <p className="question-note" role="note">
            This is a question, not a permission request. Answering sends a normal message and does not
            authorize any action.
          </p>
          <label className="sr-only" htmlFor="answer-input">
            Your answer
          </label>
          <textarea
            id="answer-input"
            className="composer__input"
            rows={3}
            placeholder="Type your answer…"
            value={answer}
            disabled={isReadOnly}
            onChange={(event) => setAnswer(event.target.value)}
          />
          {answerState ? (
            <p className="composer__status" role="status">
              <span className={`pill pill--${sendStatusView(answerState.status).tone} pill--sm`}>
                <span className="pill__icon" aria-hidden="true">
                  {sendStatusView(answerState.status).icon}
                </span>
                <span className="pill__label">{sendStatusView(answerState.status).label}</span>
              </span>
              {answerState.errorMessage ? (
                <span className="composer__error">{answerState.errorMessage}</span>
              ) : null}
            </p>
          ) : null}
          <button
            className="btn btn-primary"
            type="button"
            onClick={() => void sendAnswer()}
            disabled={isReadOnly || answer.trim().length === 0}
          >
            Send answer
          </button>
        </section>
      ) : (
        <div className="decision-actions">
          <button
            className="btn decision-actions__deny"
            type="button"
            onClick={() => void decide("deny")}
            disabled={isReadOnly || expired || submitting}
          >
            Deny
          </button>
          <button
            className="btn btn-primary decision-actions__approve"
            type="button"
            onClick={() => {
              if (requiresConfirmation(approval)) setConfirming("approve_once");
              else void decide("approve_once");
            }}
            disabled={isReadOnly || expired || submitting}
          >
            Approve once
          </button>
        </div>
      )}

      <Modal
        open={confirming != null}
        title="Confirm approval"
        onClose={() => setConfirming(null)}
        footer={
          <>
            <button className="btn" type="button" onClick={() => setConfirming(null)}>
              Cancel
            </button>
            <button
              className="btn btn-danger"
              type="button"
              onClick={() => void decide("approve_once")}
              disabled={submitting}
            >
              {submitting ? "Approving…" : "Confirm approve once"}
            </button>
          </>
        }
      >
        <p>{riskLabel(approval.risk)}. This allows exactly one operation:</p>
        <code className="command-block">{operation ?? approval.title}</code>
        <p>{limitations(approval)}</p>
      </Modal>
    </article>
  );
}
