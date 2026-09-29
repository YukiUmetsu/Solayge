import type { PermissionProfile, Project, Separation, Task, TaskKind } from "../types";
import { PromptSuggestions } from "./PromptSuggestions";
import { CommandPicker } from "./CommandPicker";
import { Label } from "./Field";
import {
  DEFAULT_SEPARATION,
  SEPARATIONS,
  separationFromIsolation,
  separationMeta,
} from "../lib/providers";

/**
 * The editable fields of a task. Shared verbatim by New Task and Edit Task so
 * the two forms can never drift apart.
 */
export interface TaskFormState {
  kind: TaskKind;
  title: string;
  prompt: string;
  command: string;
  separation: Separation;
  newBranch: string;
  profile: PermissionProfile;
  delayMinutes: number;
  baseRef: string;
  deps: Set<string>;
}

/** A blank form, with the project's default profile pre-selected. */
export function emptyTaskForm(profile: PermissionProfile): TaskFormState {
  return {
    kind: "agent",
    title: "",
    prompt: "",
    command: "",
    separation: DEFAULT_SEPARATION,
    newBranch: "",
    profile,
    delayMinutes: 0,
    baseRef: "",
    deps: new Set(),
  };
}

export function TaskFormFields({
  project,
  tasks,
  value,
  onChange,
  specialKind,
  specialLabel,
}: {
  project: Project;
  /** Tasks offered as dependencies. Callers exclude the task being edited. */
  tasks: Task[];
  value: TaskFormState;
  onChange: (patch: Partial<TaskFormState>) => void;
  /** A git/merge task's own kind, shown as a locked option when editing. */
  specialKind?: TaskKind | null;
  specialLabel?: string;
}) {
  const shell = value.kind === "shell";
  const special = specialKind != null && value.kind === specialKind;

  function toggleDep(id: string) {
    const next = new Set(value.deps);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange({ deps: next });
  }

  return (
    <div className="space-y-4">
      <div>
        <Label>Task kind</Label>
        <select
          className="select"
          value={value.kind}
          onChange={(e) => onChange({ kind: e.target.value as TaskKind })}
        >
          {specialKind && (
            <option value={specialKind} disabled>
              {specialLabel ?? specialKind}
            </option>
          )}
          <option value="agent">Prompt (coding agent)</option>
          <option value="shell">Command or skill</option>
        </select>
      </div>

      <div>
        <Label>Title</Label>
        <input
          className="input"
          placeholder="Short label (optional)"
          value={value.title}
          onChange={(e) => onChange({ title: e.target.value })}
        />
      </div>

      {value.kind === "agent" && (
        <>
          <div>
            <Label>Prompt</Label>
            <textarea
              className="textarea"
              rows={7}
              placeholder="Full instruction for the coding agent…"
              value={value.prompt}
              onChange={(e) => onChange({ prompt: e.target.value })}
            />
          </div>

          <PromptSuggestions
            projectPath={project.path}
            onPick={(h) => {
              const patch: Partial<TaskFormState> = { prompt: h.prompt };
              if (!value.title.trim() && h.title) patch.title = h.title;
              if (h.profile) patch.profile = h.profile;
              if (h.isolation) {
                patch.separation = separationFromIsolation(
                  h.isolation,
                  DEFAULT_SEPARATION,
                );
              }
              onChange(patch);
            }}
          />
        </>
      )}

      {shell && (
        <CommandPicker
          skills={project.skills}
          command={value.command}
          onChange={(command) => onChange({ command })}
          onPickSkill={(skill) => {
            const patch: Partial<TaskFormState> = { command: skill.command };
            if (!value.title.trim()) patch.title = skill.name;
            onChange(patch);
          }}
        />
      )}

      {special && (
        <div className="rounded-lg border border-line bg-well p-2.5 text-[11.5px] leading-relaxed text-ink-muted">
          This {specialLabel ?? specialKind} task is configured in the Ship /
          Combine flow. Pick a kind above to turn it into a prompt or command.
        </div>
      )}

      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label>Permissions</Label>
          <select
            className="select"
            value={value.profile}
            onChange={(e) =>
              onChange({ profile: e.target.value as PermissionProfile })
            }
          >
            <option value="autonomous">Autonomous (pre-granted)</option>
            <option value="supervised">Supervised (ask, notify)</option>
            <option value="readonly">Read-only</option>
          </select>
        </div>
        <div>
          <Label>Separation</Label>
          <select
            className="select"
            value={value.separation}
            onChange={(e) =>
              onChange({ separation: e.target.value as Separation })
            }
          >
            {SEPARATIONS.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
          <p className="mt-1 text-[11px] leading-relaxed text-ink-subtle">
            {separationMeta(value.separation).desc}
          </p>
        </div>
        <div>
          <Label>Delay (min)</Label>
          <input
            type="number"
            min={0}
            className="input"
            value={value.delayMinutes}
            onChange={(e) => onChange({ delayMinutes: Number(e.target.value) })}
          />
        </div>
        <div>
          <Label>Base ref</Label>
          <input
            className="input"
            placeholder="HEAD"
            value={value.baseRef}
            onChange={(e) => onChange({ baseRef: e.target.value })}
          />
        </div>
      </div>

      {value.separation === "branch" && (
        <div>
          <Label>Branch name</Label>
          <input
            className="input mono"
            placeholder="Leave blank for the agent to name it"
            value={value.newBranch}
            onChange={(e) => onChange({ newBranch: e.target.value })}
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
          <Label>Runs after (optional)</Label>
          <div className="scroll max-h-40 space-y-1 rounded-lg border border-line bg-well p-2">
            {tasks.map((t) => (
              <label
                key={t.id}
                className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-[12px] text-ink-muted hover:bg-hover"
              >
                <input
                  type="checkbox"
                  className="accent-accent"
                  checked={value.deps.has(t.id)}
                  onChange={() => toggleDep(t.id)}
                />
                <span className="truncate">{t.title}</span>
              </label>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
