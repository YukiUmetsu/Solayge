/**
 * On-device cache for task data, with a per-project retention window.
 *
 * The phone keeps only the minimum it needs to show a useful offline view:
 * projects, task summaries, and pending approvals. Retention defaults to one
 * week per project and is configurable per project; expired data is dropped
 * when read, never shown stale beyond its window. Writes are best-effort: a
 * missing or full browser storage must never break the live app.
 */
import type { ApprovalSummary, ProjectSummary, SessionSummary } from "@solayge/shared-types";

/** The task data a cached snapshot holds (mirrors the client's `ClientData`). */
export interface CachedSnapshot {
  projects: ProjectSummary[];
  sessions: SessionSummary[];
  approvals: ApprovalSummary[];
}

/** A stored snapshot plus when it was fetched. */
export interface CacheEntry extends CachedSnapshot {
  savedAt: number;
}

/** The read/write seam the client context uses; tests can pass a fake. */
export interface TaskCache {
  read(desktopId: string, now: number): CacheEntry | null;
  write(desktopId: string, snapshot: CachedSnapshot, now: number): void;
}

export const DEFAULT_RETENTION_DAYS = 7;
export const MAX_RETENTION_DAYS = 3650;

const RETENTION_KEY = "solayge.task-cache.retention.v1";
const ENTRY_PREFIX = "solayge.task-cache.v1.";
const SECONDS_PER_DAY = 24 * 60 * 60;

/** Per-project retention in days, keyed by project id. Missing means default. */
export type RetentionConfig = Record<string, number>;

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    // Access can throw in hardened webviews; treat it as "no cache".
    return null;
  }
}

/** Load the per-project retention configuration (defensive against bad data). */
export function loadRetention(): RetentionConfig {
  const store = storage();
  if (!store) return {};
  try {
    const raw = store.getItem(RETENTION_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return {};
    const out: RetentionConfig = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof value === "number" && Number.isFinite(value)) {
        out[key] = clampRetention(value);
      }
    }
    return out;
  } catch {
    return {};
  }
}

/** Persist the retention for one project. */
export function setProjectRetention(projectId: string, days: number): RetentionConfig {
  const config = loadRetention();
  config[projectId] = clampRetention(days);
  const store = storage();
  if (store) {
    try {
      store.setItem(RETENTION_KEY, JSON.stringify(config));
    } catch {
      // Keep the in-memory result even if persistence fails.
    }
  }
  return config;
}

/** Clamp a retention value to the allowed range. */
export function clampRetention(days: number): number {
  if (!Number.isFinite(days)) return DEFAULT_RETENTION_DAYS;
  return Math.min(MAX_RETENTION_DAYS, Math.max(0, Math.round(days)));
}

/** The retention for a project, defaulting to one week. */
export function retentionFor(projectId: string, config: RetentionConfig): number {
  const value = config[projectId];
  return value == null ? DEFAULT_RETENTION_DAYS : value;
}

/**
 * Drop entries older than the owning project's retention window. Projects are
 * kept (they are tiny, and their names still make the offline list readable);
 * sessions and their approvals are dropped once expired.
 */
export function applyRetention(
  entry: CacheEntry,
  now: number,
  config: RetentionConfig,
): CacheEntry {
  const sessions = entry.sessions.filter((session) => {
    const days = retentionFor(session.project_id, config);
    return now - entry.savedAt <= days * SECONDS_PER_DAY;
  });
  const liveSessionIds = new Set(sessions.map((session) => session.id));
  const approvals = entry.approvals.filter((approval) => liveSessionIds.has(approval.session_id));
  return { ...entry, sessions, approvals };
}

function entryKey(desktopId: string): string {
  return `${ENTRY_PREFIX}${desktopId}`;
}

/** A `localStorage`-backed cache. Safe to call when storage is unavailable. */
export function createLocalStorageCache(): TaskCache {
  return {
    read(desktopId: string, now: number): CacheEntry | null {
      const store = storage();
      if (!store || !desktopId) return null;
      try {
        const raw = store.getItem(entryKey(desktopId));
        if (!raw) return null;
        const parsed = JSON.parse(raw) as CacheEntry;
        if (
          !parsed ||
          typeof parsed.savedAt !== "number" ||
          !Array.isArray(parsed.projects) ||
          !Array.isArray(parsed.sessions) ||
          !Array.isArray(parsed.approvals)
        ) {
          return null;
        }
        return applyRetention(parsed, now, loadRetention());
      } catch {
        return null;
      }
    },
    write(desktopId: string, snapshot: CachedSnapshot, now: number): void {
      const store = storage();
      if (!store || !desktopId) return;
      try {
        const entry: CacheEntry = { ...snapshot, savedAt: now };
        store.setItem(entryKey(desktopId), JSON.stringify(entry));
      } catch {
        // Best effort: a full/denied store must not break the app.
      }
    },
  };
}

/** Remove cached task data for one desktop, or every desktop when omitted. */
export function clearCachedData(desktopId?: string): void {
  const store = storage();
  if (!store) return;
  try {
    if (desktopId) {
      store.removeItem(entryKey(desktopId));
      return;
    }
    const keys: string[] = [];
    for (let i = 0; i < store.length; i += 1) {
      const key = store.key(i);
      if (key && key.startsWith(ENTRY_PREFIX)) keys.push(key);
    }
    for (const key of keys) store.removeItem(key);
  } catch {
    // Nothing to do.
  }
}
