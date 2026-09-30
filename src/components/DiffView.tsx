import { useState } from "react";
import type { DiffResult } from "../types";
import { Icon } from "./Icons";

/** One collapsible file inside a diff. */
export function DiffFile({
  path,
  status,
  diff,
}: {
  path: string;
  status: string;
  diff: string;
}) {
  const [open, setOpen] = useState(true);
  return (
    <div className="overflow-hidden rounded-lg border border-line">
      <button
        className="flex w-full items-center gap-2 bg-well px-3 py-1.5 text-left text-[11.5px]"
        onClick={() => setOpen((v) => !v)}
      >
        <Icon
          name="chevron"
          className={`h-3 w-3 text-ink-subtle transition ${open ? "rotate-90" : ""}`}
        />
        <span className="mono truncate text-ink">{path}</span>
        <span className="mono ml-auto text-[10px] text-ink-subtle">{status.trim()}</span>
      </button>
      {open && (
        <pre className="mono max-h-72 overflow-auto bg-well-strong p-2 text-[11px] leading-relaxed">
          {diff.split("\n").map((line, i) => (
            <div
              key={i}
              className={
                line.startsWith("+")
                  ? "text-success"
                  : line.startsWith("-")
                    ? "text-danger"
                    : line.startsWith("@@")
                      ? "text-accent-text"
                      : "text-ink-muted"
              }
            >
              {line || " "}
            </div>
          ))}
        </pre>
      )}
    </div>
  );
}

/** Count a diff's files as added, modified, or deleted. */
export function summarizeDiff(files: { status: string }[]): {
  added: number;
  modified: number;
  deleted: number;
} {
  let added = 0;
  let modified = 0;
  let deleted = 0;
  for (const f of files) {
    const status = f.status.trim();
    if (status.startsWith("A") || status === "??" || status === "?") {
      added += 1;
    } else if (status.startsWith("D")) {
      deleted += 1;
    } else {
      modified += 1;
    }
  }
  return { added, modified, deleted };
}

/** The "N files changed / X added / Y modified / Z deleted" summary row. */
export function DiffStats({ files }: { files: { status: string }[] }) {
  const { added, modified, deleted } = summarizeDiff(files);
  return (
    <div className="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
      <span className="text-ink-muted">{files.length} files changed</span>
      <span className="text-success">{added} added</span>
      <span className="text-accent-text">{modified} modified</span>
      {deleted > 0 && <span className="text-danger">{deleted} deleted</span>}
    </div>
  );
}

/** The stat summary plus the list of changed files for a diff result. */
export function DiffBody({
  result,
  empty = "No changes.",
}: {
  result: DiffResult;
  empty?: string;
}) {
  if (result.files.length === 0) {
    return <p className="text-xs text-ink-subtle">{empty}</p>;
  }
  return (
    <>
      {result.stat.trim() && (
        <pre className="mono mb-3 whitespace-pre-wrap rounded-lg border border-line bg-well-strong p-2 text-[11px] text-ink-muted">
          {result.stat.trim()}
        </pre>
      )}
      <div className="space-y-3">
        {result.files.map((f) => (
          <DiffFile key={f.path} path={f.path} status={f.status} diff={f.diff} />
        ))}
      </div>
    </>
  );
}
