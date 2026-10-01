import { memo } from "react";
import type { LogEntry, LogKind } from "../types";

/**
 * Strip an optional leading log clock stamp (`MM-DD HH:MM:SS ` or `HH:MM:SS `)
 * so the line's real marker is at the start for classification. The stamp is
 * still shown; this only affects how the line is colored.
 */
export function stripLogStamp(line: string): string {
  const m = line.match(/^(?:\d{2}-\d{2} )?\d{2}:\d{2}:\d{2} /);
  return m ? line.slice(m[0].length) : line;
}

/** App-generated notes that indicate something went wrong. */
const NOTEISH = /\b(error|failed|interrupt|stalled?|blocked|cannot|refused)\b/i;

/**
 * Classify one log line. `hint` is the coarse kind the backend already attached
 * (`text` | `tool` | `note`); when it is absent (e.g. a reloaded log) the line's
 * own markers are used. Prose is never painted red just for mentioning "error":
 * only unambiguous markers are.
 */
export function classifyLine(line: string, hint?: string | null): LogKind {
  const body = stripLogStamp(line);
  // Markdown headings in agent prose stand out.
  if (/^#{1,6}\s+\S/.test(body)) return "heading";
  if (hint === "tool" || body.startsWith("[tool] ")) {
    return /\((error|failed)\)\s*$/.test(body) ? "error" : "tool";
  }
  if (hint === "note" || body.startsWith("[Solayge] ")) {
    return NOTEISH.test(body) ? "error" : "note";
  }
  if (/\bREVIEW:\s*PASS\b/i.test(body)) return "success";
  if (/\bREVIEW:\s*ISSUES\b/i.test(body)) return "warn";
  if (
    /^(error|Error|ERROR)[:\s]/.test(body) ||
    /\bpanicked at\b/.test(body) ||
    /\btest result:\s*FAILED\b/.test(body)
  ) {
    return "error";
  }
  return "text";
}

const TONE_CLASS: Record<LogKind, string> = {
  // Prose carries the color; tool chatter is muted so it stays out of the way.
  text: "text-ink",
  tool: "text-ink-subtle",
  note: "text-info",
  error: "text-danger",
  warn: "text-warning",
  success: "text-success",
  heading: "text-accent-text font-semibold",
};

/** One log line. Memoized: a growing log must not re-render older lines. */
const LogLine = memo(function LogLine({ text, kind }: LogEntry) {
  return <div className={TONE_CLASS[kind]}>{text === "" ? " " : text}</div>;
});

/** The task log body: one colored line per entry, optionally hiding tool calls. */
export function LogBody({
  entries,
  hideTools,
}: {
  entries: LogEntry[];
  hideTools: boolean;
}) {
  if (entries.length === 0) {
    return <span className="text-ink-faint">No output yet.</span>;
  }
  const shown = hideTools ? entries.filter((e) => e.kind !== "tool") : entries;
  if (shown.length === 0) {
    return <span className="text-ink-faint">Tool calls hidden.</span>;
  }
  return (
    <>
      {shown.map((e, i) => (
        <LogLine key={i} text={e.text} kind={e.kind} />
      ))}
    </>
  );
}
