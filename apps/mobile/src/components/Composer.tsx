import { useState, type FormEvent, type KeyboardEvent } from "react";
import type { CommandStatusName } from "@solayge/shared-types";

import { canRetryCommand, sendStatusView } from "../client/status";

export interface SendState {
  status: CommandStatusName;
  errorCode: string | null;
  errorMessage: string | null;
}

/**
 * The sticky message composer. It is visually distinct from Stop, keeps the
 * send status explicit, and only offers retry when the command is known not to
 * have executed (mobile spec sections 3.2 and 7.2 C).
 */
export function Composer({
  onSend,
  onStop,
  onRetry,
  sendState,
  canStop,
  readOnly,
  disabledReason,
}: {
  onSend: (text: string) => void;
  onStop: () => void;
  onRetry: () => void;
  sendState: SendState | null;
  canStop: boolean;
  readOnly: boolean;
  disabledReason?: string;
}) {
  const [text, setText] = useState("");
  const view = sendState ? sendStatusView(sendState.status) : null;
  const retryable = sendState
    ? canRetryCommand(sendState.status, sendState.errorCode)
    : false;

  function submit(event?: FormEvent) {
    event?.preventDefault();
    const trimmed = text.trim();
    if (!trimmed || readOnly) return;
    onSend(trimmed);
    setText("");
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      submit();
    }
  }

  return (
    <form className="composer" onSubmit={submit} aria-label="Message composer">
      {view ? (
        <div className="composer__status" role="status">
          <span className={`pill pill--${view.tone} pill--sm`}>
            <span className="pill__icon" aria-hidden="true">
              {view.icon}
            </span>
            <span className="pill__label">{view.label}</span>
          </span>
          {sendState?.errorMessage ? (
            <span className="composer__error">{sendState.errorMessage}</span>
          ) : null}
          {retryable ? (
            <button className="btn btn--small" type="button" onClick={onRetry}>
              Tap to retry
            </button>
          ) : null}
          {sendState?.status === "unknown" ? (
            <span className="composer__hint">Checking with the desktop before retrying.</span>
          ) : null}
        </div>
      ) : null}

      <label className="sr-only" htmlFor="composer-input">
        Message your agent
      </label>
      <textarea
        id="composer-input"
        className="composer__input"
        value={text}
        rows={2}
        placeholder={readOnly ? "Offline — messaging is disabled" : "Message your agent…"}
        disabled={readOnly}
        aria-describedby={readOnly && disabledReason ? "composer-disabled" : undefined}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
      />
      {readOnly && disabledReason ? (
        <p className="composer__disabled" id="composer-disabled">
          {disabledReason}
        </p>
      ) : null}

      <div className="composer__actions">
        <button
          className="btn btn-primary composer__send"
          type="submit"
          disabled={readOnly || text.trim().length === 0}
        >
          Send
        </button>
        <button
          className="btn composer__stop"
          type="button"
          onClick={onStop}
          disabled={!canStop}
        >
          Stop task
        </button>
      </div>
    </form>
  );
}
