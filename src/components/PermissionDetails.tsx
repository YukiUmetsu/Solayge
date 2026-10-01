import type { TaskAsk } from "../types";
import { Icon } from "./Icons";

/** Friendly label for a permission decision button. */
export const DECISION_LABELS: Record<string, string> = {
  once: "Allow once",
  always: "Always allow",
  reject: "Reject",
};

/** One-line explanation of what each decision does, shown as a tooltip. */
export const DECISION_HINTS: Record<string, string> = {
  once: "Allow just this one request",
  always: "Remember this exact request for the session",
  reject: "Deny this request and tell the agent",
};

/** The heading for the resource list, based on what the action operates on. */
const RESOURCE_LABELS: Record<string, string> = {
  shell: "Command",
  bash: "Command",
  execute: "Command",
  read: "File",
  write: "File",
  edit: "File",
  patch: "File",
  external_directory: "Directory",
  webfetch: "URL",
  fetch: "URL",
  websearch: "Query",
  grep: "Search",
  glob: "Pattern",
  list: "Directory",
  ls: "Directory",
};

/** Render an arbitrary metadata value as one compact line. */
function metaText(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/**
 * The full picture of a permission request: why it was raised, exactly what it
 * touches, the provider's own details, and what "Always allow" would remember.
 * Shared by the modal prompt and the in-place task panel so the two never drift.
 */
export function PermissionDetails({ ask }: { ask: TaskAsk }) {
  const action = ask.action ?? "";
  const resourceLabel = RESOURCE_LABELS[action] ?? "Target";
  const metadata =
    ask.metadata && typeof ask.metadata === "object" && !Array.isArray(ask.metadata)
      ? Object.entries(ask.metadata as Record<string, unknown>).filter(
          ([, v]) => v !== null && v !== undefined && v !== "",
        )
      : [];

  return (
    <div className="space-y-3">
      <div className="flex items-start gap-2 rounded-lg border border-warning-line bg-warning-soft p-3 text-[12px] text-warning">
        <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        <div>
          <div className="font-medium">
            {ask.purpose ?? "This task is asking for permission"}
          </div>
          <div className="text-[11px] text-ink-muted">
            The task is paused until you answer.
          </div>
        </div>
      </div>

      {ask.message && (
        <div>
          <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
            Why
          </div>
          <p className="whitespace-pre-wrap rounded-lg border border-line bg-well p-2 text-[11.5px] leading-relaxed text-ink-muted">
            {ask.message}
          </p>
        </div>
      )}

      {ask.resources.length > 0 && (
        <div>
          <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
            {resourceLabel}
            {ask.resources.length > 1 ? "s" : ""}
          </div>
          <div className="space-y-1">
            {ask.resources.map((resource, i) => (
              <pre
                key={`${i}-${resource}`}
                className="mono whitespace-pre-wrap break-all rounded-lg border border-line bg-well p-2 text-[11px] leading-relaxed text-ink"
              >
                {resource}
              </pre>
            ))}
          </div>
        </div>
      )}

      {metadata.length > 0 && (
        <div>
          <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
            Details
          </div>
          <dl className="divide-y divide-line overflow-hidden rounded-lg border border-line">
            {metadata.map(([key, value]) => (
              <div key={key} className="flex gap-3 bg-well px-2 py-1.5 text-[11px]">
                <dt className="w-24 shrink-0 text-ink-subtle">{key}</dt>
                <dd className="mono min-w-0 flex-1 whitespace-pre-wrap break-all text-ink-muted">
                  {metaText(value)}
                </dd>
              </div>
            ))}
          </dl>
        </div>
      )}

      <div className="text-[11px] text-ink-subtle">
        {ask.save.length > 0 ? (
          <>
            “Always allow” will remember:{" "}
            <span className="mono text-ink-muted">{ask.save.join("; ")}</span>
          </>
        ) : (
          <>“Always allow” remembers this request for the rest of the session.</>
        )}
      </div>
    </div>
  );
}
