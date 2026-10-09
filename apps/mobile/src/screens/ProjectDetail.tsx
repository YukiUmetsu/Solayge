import { useEffect, useMemo, useState } from "react";
import type { ChangedFileSummary } from "@solayge/shared-types";

import { useClientContext, useNow } from "../client/ClientContext";
import { formatChange, relativeTime } from "../client/format";
import { EmptyState } from "../components/EmptyState";
import { SessionRow } from "../components/SessionRow";

/**
 * Project detail: tasks and a changed-files summary (mobile spec section 7.2 E).
 */
export function ProjectDetail({
  projectId,
  onOpenSession,
}: {
  projectId: string;
  onOpenSession: (id: string) => void;
}) {
  const { client, data, isReadOnly } = useClientContext();
  const nowFn = useNow();
  const [files, setFiles] = useState<ChangedFileSummary[]>([]);
  const [filesLoading, setFilesLoading] = useState(false);
  const [filesError, setFilesError] = useState(false);

  const project = data.projects.find((entry) => entry.id === projectId) ?? null;
  const sessions = useMemo(
    () =>
      data.sessions
        .filter((session) => session.project_id === projectId)
        .sort((a, b) => b.updated_at - a.updated_at),
    [data.sessions, projectId],
  );

  useEffect(() => {
    if (isReadOnly || sessions.length === 0) {
      setFiles([]);
      return undefined;
    }
    let cancelled = false;
    setFilesLoading(true);
    setFilesError(false);
    // Keep per-session failures visible instead of silently reporting "no
    // changes": a failed read is not the same as an empty diff.
    Promise.all(
      sessions.map((session) =>
        client
          .filesChangedSummary(session.id)
          .then((summary) => ({ ok: true, files: summary.files }))
          .catch(() => ({ ok: false, files: [] as ChangedFileSummary[] })),
      ),
    )
      .then((results) => {
        if (cancelled) return;
        const seen = new Set<string>();
        const merged: ChangedFileSummary[] = [];
        for (const file of results.flatMap((result) => result.files)) {
          const key = `${file.path}:${file.status}`;
          if (seen.has(key)) continue;
          seen.add(key);
          merged.push(file);
        }
        setFiles(merged);
        setFilesError(results.some((result) => !result.ok));
      })
      .catch(() => {
        if (!cancelled) setFilesError(true);
      })
      .finally(() => {
        if (!cancelled) setFilesLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client, isReadOnly, sessions]);

  if (!project) {
    return (
      <EmptyState
        icon="▦"
        title="Project unavailable"
        body="This project is no longer reported by the desktop."
      />
    );
  }

  return (
    <article className="detail">
      <header className="detail__head">
        <h1 className="detail__title">{project.name}</h1>
        <p className="project-row__path mono">{project.id}</p>
      </header>

      <section className="detail__section" aria-label="Tasks">
        <h2 className="detail__section-title">Tasks</h2>
        {sessions.length === 0 ? (
          <p className="detail__sub">No tasks for this project.</p>
        ) : (
          <ul className="session-list">
            {sessions.map((session) => (
              <SessionRow
                key={session.id}
                session={session}
                projectName={project.name}
                onOpen={onOpenSession}
              />
            ))}
          </ul>
        )}
      </section>

      <section className="detail__section" aria-label="Changed files">
        <h2 className="detail__section-title">Changed files</h2>
        {files.length > 0 ? (
          <ul className="file-list">
            {files.map((file, index) => (
              <li key={`${file.path}-${index}`} className="file-row">
                <span className="file-row__path mono">{file.path}</span>
                <span className="file-row__status">{file.status}</span>
                <span className="file-row__change">{formatChange(file.additions, file.deletions)}</span>
              </li>
            ))}
          </ul>
        ) : filesLoading ? (
          <p className="detail__sub" role="status">
            Loading changed files…
          </p>
        ) : (
          <p className="detail__sub">
            {filesError
              ? "Couldn't load changed files."
              : isReadOnly && sessions.length > 0
                ? "Change details are unavailable offline."
                : "No changes in this project."}
          </p>
        )}
      </section>
      <p className="detail__sub">Updated {relativeTime(project.updated_at, nowFn())}</p>
    </article>
  );
}
