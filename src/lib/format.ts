import type { PermissionProfile, TaskStatus } from "../types";

export interface StatusMeta {
  label: string;
  dot: string;
  chip: string;
  text: string;
}

export const STATUS_META: Record<TaskStatus, StatusMeta> = {
  draft: {
    label: "Draft",
    dot: "bg-ink-faint",
    chip: "bg-ink-subtle-soft border-ink-subtle-line",
    text: "text-ink-subtle",
  },
  waiting: {
    label: "Waiting",
    dot: "bg-neutral",
    chip: "bg-neutral-soft border-neutral-line",
    text: "text-neutral",
  },
  ready: {
    label: "Ready",
    dot: "bg-info",
    chip: "bg-info-soft border-info-line",
    text: "text-info",
  },
  running: {
    label: "Running",
    dot: "bg-warning running-dot",
    chip: "bg-warning-soft border-warning-line",
    text: "text-warning",
  },
  succeeded: {
    label: "Succeeded",
    dot: "bg-success",
    chip: "bg-success-soft border-success-line",
    text: "text-success",
  },
  failed: {
    label: "Failed",
    dot: "bg-danger",
    chip: "bg-danger-soft border-danger-line",
    text: "text-danger",
  },
  canceled: {
    label: "Canceled",
    dot: "bg-ink-subtle",
    chip: "bg-ink-subtle-soft border-ink-subtle-line",
    text: "text-ink-subtle",
  },
  blocked: {
    label: "Blocked",
    dot: "bg-violet",
    chip: "bg-violet-soft border-violet-line",
    text: "text-violet",
  },
};

export const PROFILE_META: Record<
  PermissionProfile,
  { label: string; short: string; desc: string; chip: string; text: string; dot: string }
> = {
  autonomous: {
    label: "Autonomous",
    short: "auto",
    desc: "Pre-granted; runs with --auto so asks are auto-approved. Hard rails still block sudo, force-push, and SSH key reads.",
    chip: "bg-accent-soft border-accent-line",
    text: "text-accent-text",
    dot: "bg-accent",
  },
  supervised: {
    label: "Supervised",
    short: "ask",
    desc: "Reads and edits allowed; shell, network, and external folders ask. With no approver they are auto-rejected and raised as notifications (approve/reject arrives with the remote runner).",
    chip: "bg-warning-soft border-warning-line",
    text: "text-warning",
    dot: "bg-warning",
  },
  readonly: {
    label: "Read-only",
    short: "read",
    desc: "Analysis only: reads, glob, grep, and search. No edits, no shell.",
    chip: "bg-info-soft border-info-line",
    text: "text-info",
    dot: "bg-info",
  },
};

export function relTime(epochSec?: number | null): string {
  if (!epochSec) return "—";
  const diff = Math.floor(Date.now() / 1000) - epochSec;
  const abs = Math.abs(diff);
  if (abs < 5) return "just now";
  if (abs < 60) return `${abs}s ${diff >= 0 ? "ago" : "from now"}`;
  if (abs < 3600) return `${Math.floor(abs / 60)}m ${diff >= 0 ? "ago" : "from now"}`;
  if (abs < 86400) return `${Math.floor(abs / 3600)}h ${diff >= 0 ? "ago" : "from now"}`;
  return `${Math.floor(abs / 86400)}d ${diff >= 0 ? "ago" : "from now"}`;
}

export function duration(start?: number | null, end?: number | null): string {
  if (!start) return "—";
  const stop = end ?? Math.floor(Date.now() / 1000);
  const secs = Math.max(0, stop - start);
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return `${m}m ${s}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

export function clock(epochSec?: number | null): string {
  if (!epochSec) return "—";
  return new Date(epochSec * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Duration of a task, live while it is still running. */
export function runDuration(
  task: { started_at?: number | null; finished_at?: number | null },
  now: number,
): string {
  if (!task.started_at) return "—";
  return duration(task.started_at, task.finished_at ?? now);
}

/** True while a task is still running (no finish timestamp yet). */
export function isLive(task: {
  started_at?: number | null;
  finished_at?: number | null;
}): boolean {
  return !!task.started_at && !task.finished_at;
}

export function shortId(id: string): string {
  return id.slice(0, 8);
}
