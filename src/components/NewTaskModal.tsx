import { useState } from "react";
import type {
  PermissionProfile,
  Project,
  Separation,
  Snapshot,
  Task,
} from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { PromptSuggestions } from "./PromptSuggestions";
import {
  DEFAULT_SEPARATION,
  SEPARATIONS,
  separationConfig,
  separationFromIsolation,
  separationMeta,
} from "../lib/providers";

export function NewTaskModal({
  project,
  tasks,
  onClose,
  onCreated,
}: {
  project: Project;
  tasks: Task[];
  onClose: () => void;
  onCreated: (s: Snapshot) => void;
}) {
  const [title, setTitle] = useState("");
  const [prompt, setPrompt] = useState("");
  const [separation, setSeparation] = useState<Separation>(DEFAULT_SEPARATION);
  const [newBranch, setNewBranch] = useState("");
  const [profile, setProfile] = useState<PermissionProfile>(
    project.default_profile ?? "autonomous",
  );
  const [delayMinutes, setDelayMinutes] = useState(0);
  const [baseRef, setBaseRef] = useState("");
  const [deps, setDeps] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const { isolation, branch_mode } = separationConfig(separation);

  async function create() {
    if (!prompt.trim()) {
      setError("A prompt is required.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const snap = await api.createTasks(
        project.path,
        [
          {
            title: title.trim() || prompt.trim().slice(0, 48),
            prompt,
            isolation,
            branch_mode,
            new_branch:
              separation === "branch" ? newBranch.trim() || null : null,
            profile,
            after: [...deps],
            delay_seconds: Math.max(0, Math.round(delayMinutes * 60)),
            base_ref: baseRef.trim() || null,
          },
        ],
        isolation,
      );
      onCreated(snap);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="New task"
      subtitle={`Adds a draft task to ${project.name}; it runs when you press Execute.`}
      onClose={onClose}
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={create} disabled={busy}>
            <Icon name="plus" className="h-3.5 w-3.5" />
            Create task
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <div>
          <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
            Title
          </label>
          <input
            className="input"
            placeholder="Short label (optional)"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
        </div>

        <div>
          <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
            Prompt
          </label>
          <textarea
            className="textarea"
            rows={7}
            placeholder="Full instruction for the coding agent…"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
        </div>

        <PromptSuggestions
          projectPath={project.path}
          onPick={(h) => {
            setPrompt(h.prompt);
            if (!title.trim() && h.title) setTitle(h.title);
            if (h.profile) setProfile(h.profile);
            if (h.isolation)
              setSeparation(
                separationFromIsolation(h.isolation, DEFAULT_SEPARATION),
              );
          }}
        />

        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Permissions
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
              Separation
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
            <p className="mt-1 text-[11px] leading-relaxed text-ink-subtle">
              {separationMeta(separation).desc}
            </p>
          </div>
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Delay (min)
            </label>
            <input
              type="number"
              min={0}
              className="input"
              value={delayMinutes}
              onChange={(e) => setDelayMinutes(Number(e.target.value))}
            />
          </div>
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Base ref
            </label>
            <input
              className="input"
              placeholder="HEAD"
              value={baseRef}
              onChange={(e) => setBaseRef(e.target.value)}
            />
          </div>
        </div>

        {separation === "branch" && (
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Branch name
            </label>
            <input
              className="input mono"
              placeholder="Leave blank for the agent to name it"
              value={newBranch}
              onChange={(e) => setNewBranch(e.target.value)}
            />
            <p className="mt-1 text-[11px] leading-relaxed text-ink-subtle">
              A new branch is created in the project folder before the task runs.
              Leave it blank and the agent picks a short name that doesn't exist
              yet.
            </p>
          </div>
        )}

        {tasks.length > 0 && (
          <div>
            <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Runs after (optional)
            </label>
            <div className="scroll max-h-40 space-y-1 rounded-lg border border-line bg-well p-2">
              {tasks.map((t) => (
                <label
                  key={t.id}
                  className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-[12px] text-ink-muted hover:bg-hover"
                >
                  <input
                    type="checkbox"
                    className="accent-accent"
                    checked={deps.has(t.id)}
                    onChange={() =>
                      setDeps((prev) => {
                        const next = new Set(prev);
                        if (next.has(t.id)) next.delete(t.id);
                        else next.add(t.id);
                        return next;
                      })
                    }
                  />
                  <span className="truncate">{t.title}</span>
                </label>
              ))}
            </div>
          </div>
        )}

        {error && (
          <div className="rounded-lg border border-danger-line bg-danger-soft p-2.5 text-[12px] text-danger">
            {error}
          </div>
        )}
      </div>
    </Modal>
  );
}
