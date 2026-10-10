/**
 * An error state that never hides the cause and offers a retry. Errors are
 * plain language; protocol detail is available but secondary.
 */
export function ErrorState({
  message,
  detail,
  onRetry,
}: {
  message: string;
  detail?: string | null;
  onRetry?: () => void;
}) {
  return (
    <div className="error-state" role="alert">
      <div className="error-state__icon" aria-hidden="true">
        !
      </div>
      <h2 className="error-state__title">Something went wrong</h2>
      <p className="error-state__body">{message}</p>
      {detail ? <p className="error-state__detail mono">{detail}</p> : null}
      {onRetry ? (
        <button className="btn" type="button" onClick={onRetry}>
          Retry
        </button>
      ) : null}
    </div>
  );
}
