import type { Task, TaskStatus } from "../types";
import { STATUS_META, type StatusMeta } from "./format";

/**
 * Status rules shared by the task card, the tree, the project header, and the
 * sidebar. They mirror the backend (`TaskStatus::is_terminal` and
 * `commands::is_clearable`); keeping them here means the UI has one definition.
 */

/** Finished statuses: no further change without a user action. */
export const TERMINAL_STATUSES: TaskStatus[] = [
  "succeeded",
  "failed",
  "canceled",
  "blocked",
  "interrupted",
];

const RUNNABLE_STATUSES: TaskStatus[] = ["draft", "waiting", "ready", "blocked"];
const CANCELABLE_STATUSES: TaskStatus[] = ["waiting", "ready", "running"];
/** What Execute re-queues: failed, canceled, blocked, and interrupted tasks. */
export const RETRYABLE_STATUSES: TaskStatus[] = [
  "failed",
  "canceled",
  "blocked",
  "interrupted",
];

export function isTerminalStatus(status: TaskStatus): boolean {
  return TERMINAL_STATUSES.includes(status);
}

/** A pending or paused task can be started now. */
export function canRun(task: { status: TaskStatus }): boolean {
  return RUNNABLE_STATUSES.includes(task.status);
}

export function canCancel(task: { status: TaskStatus }): boolean {
  return CANCELABLE_STATUSES.includes(task.status);
}

/** A finished task Execute would run again. */
export function isRetryable(task: { status: TaskStatus }): boolean {
  return RETRYABLE_STATUSES.includes(task.status);
}

/**
 * Mirrors the backend `is_clearable`: finished successes and failures go;
 * paused (blocked) and retryable (interrupted) tasks stay, as does a success
 * whose review has not settled.
 */
export function isClearable(task: Task): boolean {
  if (task.status === "failed" || task.status === "canceled") return true;
  if (task.status === "succeeded") {
    return (
      !task.review ||
      (task.review.status !== "pending" && task.review.status !== "running")
    );
  }
  return false;
}

/**
 * UI-only statuses: the persisted statuses plus `in_review`, shown while a
 * success's automatic review is still queued or running.
 */
export type TaskDisplayStatus = TaskStatus | "in_review";

/**
 * A run that succeeded but whose review has not settled yet. The backend holds
 * dependents back until it does (`scheduler::review_clear`), so the UI shows it
 * as a distinct in-progress state and does not count it as done.
 */
export function isReviewInProgress(task: Task): boolean {
  return (
    task.status === "succeeded" &&
    !!task.review &&
    task.review.mode !== "off" &&
    (task.review.status === "pending" || task.review.status === "running")
  );
}

export function displayStatus(task: Task): TaskDisplayStatus {
  return isReviewInProgress(task) ? "in_review" : task.status;
}

export const IN_REVIEW_META: StatusMeta = {
  label: "In review",
  dot: "bg-info running-dot",
  chip: "bg-info-soft border-info-line",
  text: "text-info",
};

export function statusMeta(status: TaskDisplayStatus): StatusMeta {
  return status === "in_review" ? IN_REVIEW_META : STATUS_META[status];
}
