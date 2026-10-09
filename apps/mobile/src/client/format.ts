/**
 * Timestamp formatting. Protocol timestamps are unix seconds (`i64`).
 */

const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** A compact "just now / 2m ago / 3h ago / 4d ago" label. */
export function relativeTime(seconds: number, now: number): string {
  if (!Number.isFinite(seconds)) return "unknown";
  const delta = Math.max(0, Math.floor(now - seconds));
  if (delta < 45) return "just now";
  if (delta < HOUR) {
    const minutes = Math.max(1, Math.round(delta / MINUTE));
    return `${minutes}m ago`;
  }
  if (delta < DAY) {
    const hours = Math.max(1, Math.round(delta / HOUR));
    return `${hours}h ago`;
  }
  const days = Math.max(1, Math.round(delta / DAY));
  return `${days}d ago`;
}

/** "Last updated 2m ago" with an explicit, honest label. */
export function lastUpdatedLabel(seconds: number | null, now: number): string {
  if (seconds == null) return "Last updated unknown";
  return `Last updated ${relativeTime(seconds, now)}`;
}

/** A wall-clock time like "10:42". */
export function formatClock(seconds: number): string {
  const date = new Date(seconds * 1000);
  return date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/** A full date/time for approval metadata. */
export function formatDateTime(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Elapsed time since a start, e.g. "12m elapsed". Never implies remaining. */
export function formatElapsed(startSeconds: number, now: number): string {
  const delta = Math.max(0, Math.floor(now - startSeconds));
  if (delta < HOUR) return `${Math.max(1, Math.round(delta / MINUTE))}m elapsed`;
  if (delta < DAY) return `${Math.floor(delta / HOUR)}h elapsed`;
  return `${Math.floor(delta / DAY)}d elapsed`;
}

/** Expiry as "in 59m" or "expired", or null when there is no expiry. */
export function formatExpiry(expiresAt: number | null | undefined, now: number): string | null {
  if (expiresAt == null) return null;
  if (expiresAt <= now) return "expired";
  const delta = expiresAt - now;
  if (delta < HOUR) return `in ${Math.max(1, Math.round(delta / MINUTE))}m`;
  return `in ${Math.round(delta / HOUR)}h`;
}

/** "+42 / −9" change counts, omitting unknown halves. */
export function formatChange(additions?: number | null, deletions?: number | null): string {
  const parts: string[] = [];
  if (additions != null) parts.push(`+${additions}`);
  if (deletions != null) parts.push(`−${deletions}`);
  return parts.length > 0 ? parts.join(" / ") : "no line counts";
}
