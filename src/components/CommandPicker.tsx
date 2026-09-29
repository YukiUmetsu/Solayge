import { useState } from "react";
import type { ProjectSkill } from "../types";

/**
 * Pick a project skill (which fills the command) or type a command directly.
 * Shared by the New Task and Edit Task forms for `shell` tasks.
 */
export function CommandPicker({
  skills,
  command,
  onPickSkill,
  onChange,
}: {
  skills: ProjectSkill[];
  command: string;
  onPickSkill: (skill: ProjectSkill) => void;
  onChange: (command: string) => void;
}) {
  const [skillId, setSkillId] = useState("");
  const selected = skills.find((s) => s.id === skillId) ?? null;

  return (
    <div className="space-y-4">
      {skills.length > 0 && (
        <div>
          <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
            Skill
          </label>
          <select
            className="select"
            value={skillId}
            onChange={(e) => {
              const id = e.target.value;
              setSkillId(id);
              const skill = skills.find((s) => s.id === id);
              if (skill) onPickSkill(skill);
            }}
          >
            <option value="">Choose a saved skill…</option>
            {skills.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
          {selected?.description && (
            <p className="mt-1 text-[11px] leading-relaxed text-ink-subtle">
              {selected.description}
            </p>
          )}
        </div>
      )}

      <div>
        <label className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
          Command
        </label>
        <textarea
          className="textarea mono"
          rows={5}
          placeholder="npm test"
          value={command}
          onChange={(e) => onChange(e.target.value)}
        />
        <p className="mt-1 text-[11px] leading-relaxed text-ink-subtle">
          Run with the platform shell in the project (or its worktree). Picking a
          skill fills this in; you can still edit it.
        </p>
      </div>
    </div>
  );
}
