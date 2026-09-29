import { useState } from "react";
import type {
  Isolation,
  PermissionProfile,
  PlanTask,
  Project,
  Separation,
  Snapshot,
} from "../types";
import { api } from "../api";
import {
  DEFAULT_SEPARATION,
  SEPARATIONS,
  separationConfig,
  separationFromIsolation,
  separationMeta,
} from "../lib/providers";
import { Modal } from "./Modal";
import { ErrorNote } from "./Field";
import { Icon } from "./Icons";
import { PromptSuggestions } from "./PromptSuggestions";

export function PlannerModal({
  project,
  onClose,
  onCreated,
}: {
  project: Project;
  onClose: () => void;
  onCreated: (s: Snapshot) => void;
}) {
  const [goal, setGoal] = useState("");
  const [maxTasks, setMaxTasks] = useState(8);
  const [separation, setSeparation] = useState<Separation>(DEFAULT_SEPARATION);
  const [profile, setProfile] = useState<PermissionProfile>(
    project.default_profile ?? "autonomous",
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summary, setSummary] = useState<string | null>(null);
  const [tasks, setTasks] = useState<PlanTask[]>([]);
  const [included, setIncluded] = useState<Set<number>>(new Set());

  async function generate() {
    if (!goal.trim()) return;
    setLoading(true);
    setError(null);
    setSummary(null);
    setTasks([]);
    try {
      const res = await api.plan(project.path, goal, maxTasks);
      setTasks(res.tasks);
      setSummary(res.summary);
      setIncluded(new Set(res.tasks.map((_, i) => i)));
      if (res.tasks.length === 0) setError("The planner returned no tasks.");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function create() {
    const chosen = tasks.filter((_, i) => included.has(i));
    if (chosen.length === 0) return;
    setLoading(true);
    setError(null);
    const defaultIsolation: Isolation = separationConfig(separation).isolation;
    try {
      const snap = await api.createTasks(
        project.path,
        chosen.map((t) => {
          const { isolation, branch_mode } = separationConfig(
            separationFromIsolation(t.isolation, separation),
          );
          return {
            local_id: t.id ?? null,
            title: t.title,
            prompt: t.prompt,
            isolation,
            branch_mode,
            new_branch: null,
            profile,
            after: t.after,
            delay_seconds: t.delay_seconds ?? 0,
          };
        }),
        defaultIsolation,
      );
      onCreated(snap);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  function toggle(i: number) {
    setIncluded((prev) => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  }

  return (
    <Modal
      title="Plan with AI"
      subtitle="Describe a goal. opencode drafts a task graph you can run in sequence or in parallel."
      onClose={onClose}
      width="max-w-3xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose} disabled={loading}>
            Cancel
          </button>
          {tasks.length > 0 ? (
            <button className="btn btn-primary" onClick={create} disabled={loading}>
              <Icon name="plus" className="h-3.5 w-3.5" />
              Create {included.size} task(s)
            </button>
          ) : (
            <button
              className="btn btn-primary"
              onClick={generate}
              disabled={loading || !goal.trim()}
            >
              <Icon name="sparkles" className="h-3.5 w-3.5" />
              {loading ? "Planning…" : "Generate plan"}
            </button>
          )}
        </>
      }
    >
      <div className="space-y-4">
        <div>
          <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
            Goal
          </label>
          <textarea
            className="textarea"
            rows={4}
            placeholder="e.g. Add rate limiting to the public API, with tests and docs, and a migration-safe rollout plan."
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
          />
        </div>

        <PromptSuggestions
          projectPath={project.path}
          limit={8}
          onPick={(h) => setGoal(h.prompt)}
        />

        <div className="grid grid-cols-3 gap-3">
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Max tasks
            </label>
            <input
              type="number"
              min={1}
              max={30}
              className="input"
              value={maxTasks}
              onChange={(e) => setMaxTasks(Number(e.target.value))}
            />
          </div>
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Default permissions
            </label>
            <select
              className="select"
              value={profile}
              onChange={(e) => setProfile(e.target.value as PermissionProfile)}
            >
              <option value="autonomous">Autonomous (pre-granted)</option>
              <option value="supervised">Supervised (ask, notify)</option>
              <option value="readonly">Read-only</option>
            </select>
          </div>
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Default separation
            </label>
            <select
              className="select"
              value={separation}
              onChange={(e) => setSeparation(e.target.value as Separation)}
            >
              {SEPARATIONS.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
          </div>
        </div>

        <p className="text-[11px] leading-relaxed text-ink-subtle">
          {separationMeta(separation).desc} The plan's own worktree/shared hints
          still take precedence per task.
        </p>

        <ErrorNote error={error} />

        {summary && (
          <div className="rounded-lg border border-accent-line bg-accent-soft p-3 text-[12px] leading-relaxed text-ink">
            {summary}
          </div>
        )}

        {tasks.length > 0 && (
          <div className="space-y-2">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Proposed tasks ({included.size}/{tasks.length})
            </div>
            {tasks.map((t, i) => (
              <div
                key={i}
                className={`card cursor-pointer rounded-lg p-3 transition ${
                  included.has(i) ? "" : "opacity-45"
                }`}
                onClick={() => toggle(i)}
              >
                <div className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    checked={included.has(i)}
                    onChange={() => toggle(i)}
                    onClick={(e) => e.stopPropagation()}
                    className="mt-0.5 accent-accent"
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-[13px] font-medium text-ink">
                        {t.title}
                      </span>
                      <span className="rounded border border-line bg-well px-1.5 text-[10px] text-ink-muted">
                        {separationMeta(
                          separationFromIsolation(t.isolation, DEFAULT_SEPARATION),
                        ).short}
                      </span>
                      {t.after && t.after.length > 0 && (
                        <span className="text-[10px] text-ink-subtle">
                          after {t.after.join(", ")}
                        </span>
                      )}
                      {t.delay_seconds ? (
                        <span className="text-[10px] text-ink-subtle">
                          delay {t.delay_seconds}s
                        </span>
                      ) : null}
                    </div>
                    <pre className="mono mt-1 line-clamp-3 whitespace-pre-wrap text-[11px] leading-relaxed text-ink-muted">
                      {t.prompt}
                    </pre>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </Modal>
  );
}
