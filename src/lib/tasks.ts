import type { Task, TaskStatus } from "../types";

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
