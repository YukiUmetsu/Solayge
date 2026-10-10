import type { NotificationKind } from "../client/notifications";

export interface ToastView {
  id: number;
  kind: NotificationKind;
  title: string;
  body: string;
}

const ICONS: Record<NotificationKind, string> = {
  question: "?",
  completed: "✓",
  failed: "!",
};

/**
 * In-app banners for new agent questions, completions, and failures. They are
 * the always-available feedback: even when the OS notification is denied or the
 * app is in the foreground, the user still sees what happened.
 */
export function ToastViewport({
  toasts,
  onDismiss,
}: {
  toasts: ToastView[];
  onDismiss: (id: number) => void;
}) {
  if (toasts.length === 0) return null;
  return (
    <div className="toast-viewport" aria-live="polite" aria-label="Notifications">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast toast--${toast.kind}`} role="status">
          <span className="toast__icon" aria-hidden="true">
            {ICONS[toast.kind]}
          </span>
          <span className="toast__body">
            <span className="toast__title">{toast.title}</span>
            <span className="toast__text">{toast.body}</span>
          </span>
          <button
            className="toast__close"
            type="button"
            aria-label="Dismiss notification"
            onClick={() => onDismiss(toast.id)}
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
