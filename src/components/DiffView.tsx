import { useMemo, useState } from "react";
import type { DiffResult } from "../types";
import { Icon } from "./Icons";
import { highlightLine, languageForPath } from "../lib/highlight";

type LineKind = "add" | "del" | "context" | "hunk" | "meta" | "blank";

function lineKind(line: string): LineKind {
  if (line.startsWith("@@")) return "hunk";
  // `+++`/`---` are file headers, not an added/deleted line.
  if (line.startsWith("+++") || line.startsWith("---")) return "meta";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  if (line.startsWith(" ")) return "context";
  return line.length === 0 ? "blank" : "meta";
}

/** The gutter glyph for a line: its diff marker, or blank for headers/hunks. */
function markerFor(kind: LineKind): string {
  if (kind === "add") return "+";
  if (kind === "del") return "-";
  if (kind === "context") return " ";
  return "";
}

function DiffLine({ line, language }: { line: string; language: string | null }) {
  const kind = lineKind(line);
  if (kind === "blank") {
    return <div className="diff-line diff-blank">&nbsp;</div>;
  }
  const isCode = kind === "add" || kind === "del" || kind === "context";
  // The first character is the diff marker; the rest is the actual source.
  const content = isCode ? line.slice(1) : line;
  const html = isCode ? highlightLine(content, language) : null;
  return (
    <div className={`diff-line diff-${kind}`}>
      <span className="diff-marker">{markerFor(kind)}</span>
      {html !== null ? (
        <code
          className="hljs"
          // highlight.js HTML-escapes its input, so this is safe to inject.
          dangerouslySetInnerHTML={{ __html: html || " " }}
        />
      ) : (
        <code>{content || " "}</code>
      )}
    </div>
  );
}

/** One collapsible file inside a diff, with per-file syntax highlighting. */
export function DiffFile({
  path,
  status,
  diff,
  open = true,
  onToggle,
}: {
  path: string;
  status: string;
  diff: string;
  open?: boolean;
  onToggle?: () => void;
}) {
  const language = useMemo(() => languageForPath(path), [path]);
  const lines = useMemo(
    () => (diff.length ? diff.replace(/\n$/, "").split("\n") : []),
    [diff],
  );
  return (
    <div className="overflow-hidden rounded-lg border border-line">
      <button
        className="flex w-full items-center gap-2 bg-well px-3 py-1.5 text-left text-[11.5px]"
        onClick={onToggle}
        aria-expanded={open}
      >
        <Icon
          name="chevron"
          className={`h-3 w-3 text-ink-subtle transition ${open ? "rotate-90" : ""}`}
        />
        <span className="mono truncate text-ink">{path}</span>
        {language && (
          <span className="shrink-0 rounded bg-well-strong px-1.5 py-0.5 text-[10px] text-ink-subtle">
            {language}
          </span>
        )}
        <span className="mono ml-auto text-[10px] text-ink-subtle">
          {status.trim()}
        </span>
      </button>
      {open && (
        <pre className="diff-view mono max-h-80 overflow-auto bg-well-strong p-2 text-[11px] leading-relaxed">
          {lines.map((line, i) => (
            <DiffLine key={i} line={line} language={language} />
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
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const paths = result.files.map((f) => f.path);
  const allCollapsed = paths.length > 0 && paths.every((p) => collapsed.has(p));

  if (result.files.length === 0) {
    return <p className="text-xs text-ink-subtle">{empty}</p>;
  }

  const toggle = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  return (
    <>
      {result.stat.trim() && (
        <pre className="mono mb-3 whitespace-pre-wrap rounded-lg border border-line bg-well-strong p-2 text-[11px] text-ink-muted">
          {result.stat.trim()}
        </pre>
      )}
      <div className="mb-2 flex items-center gap-1.5">
        <button
          className="btn btn-ghost !px-2 !py-1 !text-[11px]"
          onClick={() => setCollapsed(new Set())}
          disabled={collapsed.size === 0}
          title="Open all files"
        >
          <Icon name="expand" className="h-3 w-3" />
          Open all
        </button>
        <button
          className="btn btn-ghost !px-2 !py-1 !text-[11px]"
          onClick={() => setCollapsed(new Set(paths))}
          disabled={allCollapsed}
          title="Close all files"
        >
          <Icon name="collapse" className="h-3 w-3" />
          Close all
        </button>
      </div>
      <div className="space-y-3">
        {result.files.map((f) => (
          <DiffFile
            key={f.path}
            path={f.path}
            status={f.status}
            diff={f.diff}
            open={!collapsed.has(f.path)}
            onToggle={() => toggle(f.path)}
          />
        ))}
      </div>
    </>
  );
}
