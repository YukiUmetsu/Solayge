import { useClientContext, useNow } from "../client/ClientContext";
import { lastUpdatedLabel } from "../client/format";

/**
 * Shown when the phone cannot reach the desktop. It states plainly that the
 * content is last-known and read-only, and offers a retry (mobile spec
 * section 7.2 A / 7.3).
 */
export function OfflineBanner() {
  const { status, data, refresh } = useClientContext();
  const now = useNow();

  if (status === "online" || status === "connecting") return null;

  const revoked = status === "revoked";
  const message = revoked
    ? "Access was revoked on the desktop. Re-pair to continue."
    : "You're offline. Actions are disabled until the desktop is reachable.";

  return (
    <div className="offline-banner" role="status">
      <div className="offline-banner__text">
        <strong>{revoked ? "Access revoked" : "Offline · read-only"}</strong>
        <span>{message}</span>
        {data.updatedAt != null ? (
          <span className="offline-banner__updated">{lastUpdatedLabel(data.updatedAt, now())}</span>
        ) : null}
      </div>
      {!revoked ? (
        <button className="btn btn--small" type="button" onClick={() => void refresh()}>
          Retry
        </button>
      ) : null}
    </div>
  );
}
