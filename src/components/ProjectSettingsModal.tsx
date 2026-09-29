import { useEffect, useRef, useState } from "react";
import type {
  AgentConfig,
  ConflictMode,
  EnvValue,
  Project,
  ProjectSkill,
  Settings,
  Snapshot,
  SystemPrompt,
  SystemPromptPosition,
} from "../types";
import { api } from "../api";
import {
  CONFLICT_MODES,
  effectiveConfig,
  editorLabel,
  providerLabel,
  reviewModeLabel,
  secretStoreLabel,
} from "../lib/providers";
import { AgentConfigForm, SectionLabel } from "./AgentConfigForm";
import { SaveButton, SavedPill, type SaveState } from "./SaveButton";
import { Modal } from "./Modal";
import { Icon, type IconName } from "./Icons";

type Tab = "general" | "agent" | "environment" | "prompt" | "skills" | "git";

const TABS: { id: Tab; label: string; icon: IconName }[] = [
  { id: "general", label: "General", icon: "folder" },
  { id: "agent", label: "Agent", icon: "layers" },
  { id: "environment", label: "Environment", icon: "terminal" },
  { id: "prompt", label: "Prompt", icon: "sparkles" },
  { id: "skills", label: "Skills", icon: "zap" },
  { id: "git", label: "Git", icon: "branch" },
];

function TabBar({ tab, onChange }: { tab: Tab; onChange: (t: Tab) => void }) {
  return (
    <div className="flex gap-0.5 rounded-lg border border-line p-0.5">
      {TABS.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => onChange(t.id)}
          className={`flex flex-1 items-center justify-center gap-1.5 rounded-md px-2 py-1.5 text-[11.5px] font-medium transition ${
            tab === t.id
              ? "bg-accent-soft text-accent-text"
              : "text-ink-muted hover:bg-hover"
          }`}
        >
          <Icon name={t.icon} className="h-3.5 w-3.5" />
          {t.label}
        </button>
      ))}
    </div>
  );
}

const PROMPT_VARIABLES = [
  ["{{project_name}}", "the project folder name"],
  ["{{project_path}}", "its full path"],
  ["{{current_branch}}", "the checked-out branch"],
  ["{{env.NAME}}", "a project environment variable"],
] as const;

function newId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `sk-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  }
}

function newSkill(): ProjectSkill {
  return {
    id: newId(),
    name: "",
    command: "",
    description: "",
  };
}

/** The task prompt a skill turns into when added as a task. */
function skillPrompt(skill: ProjectSkill): string {
  const command = skill.command.trim();
  const description = skill.description.trim();
  if (!command) return description;
  return description ? `${command}\n\n${description}` : command;
}

export function ProjectSettingsModal({
  project,
  settings,
  onClose,
  onSaved,
}: {
  project: Project;
  settings: Settings;
  onClose: () => void;
  onSaved: (s: Snapshot) => void;
}) {
  const [cfg, setCfg] = useState<AgentConfig>(() => ({
    provider: project.provider ?? null,
    model: project.model ?? null,
    fallback_provider: project.fallback_provider ?? null,
    fallback_model: project.fallback_model ?? null,
    review_provider: project.review_provider ?? null,
    review_model: project.review_model ?? null,
    review_mode: project.review_mode ?? null,
    editor: project.editor ?? null,
  }));
  const [envVars, setEnvVars] = useState<EnvValue[]>([]);
  const [secretsLoaded, setSecretsLoaded] = useState(false);
  const [revealed, setRevealed] = useState<Record<number, boolean>>({});
  const [skills, setSkills] = useState<ProjectSkill[]>(() => project.skills ?? []);
  const [systemPrompt, setSystemPrompt] = useState<SystemPrompt>(
    () =>
      project.system_prompt ?? {
        position: "prefix",
        text: "",
        enabled: false,
      },
  );
  const [conflictMode, setConflictMode] = useState<ConflictMode>(
    () => project.conflict_mode ?? "user",
  );
  const [remote, setRemote] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [flash, setFlash] = useState<string | null>(null);
  const flashTimer = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("general");

  function flashSaved(key: string) {
    setFlash(key);
    if (flashTimer.current) window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(null), 2000);
  }

  useEffect(() => {
    let alive = true;
    api
      .projectRemote(project.path)
      .then((r) => alive && setRemote(r))
      .catch(() => alive && setRemote(null));
    api
      .projectSecrets(project.path)
      .then((vars) => {
        if (!alive) return;
        setEnvVars(vars);
        setSecretsLoaded(true);
      })
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [project.path]);

  const eff = effectiveConfig(cfg, settings);

  function patch(p: Partial<AgentConfig>) {
    setCfg((c) => ({ ...c, ...p }));
    setDirty(true);
  }

  // ---- environment variables ----

  function addEnvVar() {
    setEnvVars((v) => [...v, { key: "", value: "", secret: false }]);
    setDirty(true);
  }

  function patchEnv(index: number, p: Partial<EnvValue>) {
    setEnvVars((v) => v.map((e, i) => (i === index ? { ...e, ...p } : e)));
    setDirty(true);
  }

  function removeEnvVar(index: number) {
    setEnvVars((v) => v.filter((_, i) => i !== index));
    setDirty(true);
  }

  // ---- skills ----

  function addSkill() {
    setSkills((s) => [...s, newSkill()]);
    setDirty(true);
  }

  function patchSkill(index: number, p: Partial<ProjectSkill>) {
    setSkills((s) => s.map((sk, i) => (i === index ? { ...sk, ...p } : sk)));
    setDirty(true);
  }

  function removeSkill(index: number) {
    setSkills((s) => s.filter((_, i) => i !== index));
    setDirty(true);
  }

  async function runSkillAsTask(skill: ProjectSkill) {
    setError(null);
    try {
      onSaved(
        await api.createTasks(
          project.path,
          [
            {
              title: skill.name.trim() || "Project skill",
              prompt: skillPrompt(skill),
              after: [],
              isolation: "worktree",
            },
          ],
          "worktree",
        ),
      );
      flashSaved("skill-task");
    } catch (e) {
      setError(String(e));
    }
  }

  async function save() {
    setSaveState("saving");
    setError(null);
    try {
      onSaved(
        await api.updateProjectConfig(project.path, {
          ...cfg,
          // Only rewrite secrets once they have loaded, so a failed keychain
          // read can never wipe stored values.
          env_vars: secretsLoaded ? envVars : undefined,
          skills,
          system_prompt: systemPrompt,
          conflict_mode: conflictMode,
        }),
      );
      setDirty(false);
      setSaveState("saved");
      window.setTimeout(() => setSaveState("idle"), 2000);
    } catch (e) {
      setError(String(e));
      setSaveState("idle");
    }
  }

  async function openEditor() {
    setError(null);
    try {
      await api.openInEditor(project.path, cfg.editor ?? null);
      flashSaved("editor");
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <Modal
      title={`${project.name} — settings`}
      subtitle="Agent, code review, environment, skills, and prompt for this project."
      onClose={onClose}
      width="max-w-2xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose}>
            Done
          </button>
          <SaveButton
            dirty={dirty}
            state={saveState}
            onClick={save}
            label="Save"
          />
        </>
      }
    >
      <div className="space-y-4">
        <TabBar tab={tab} onChange={setTab} />

        {tab === "general" && (
          <div className="space-y-5">
            <section className="space-y-2">
              <div className="flex items-center justify-between">
                <SectionLabel icon="folder">Project</SectionLabel>
                <SavedPill show={flash === "editor"} label="Opened" />
              </div>
              <div className="flex items-center gap-2">
                <button className="btn btn-ghost" onClick={openEditor}>
                  <Icon name="code" className="h-3.5 w-3.5" />
                  Open in {editorLabel(eff.editor)}
                </button>
              </div>
              <span className="mono block truncate text-[11px] text-ink-subtle">
                {project.path}
              </span>
            </section>

            <section className="space-y-2">
              <SectionLabel icon="github">Remote</SectionLabel>
              {remote ? (
                <button
                  className="mono flex max-w-full items-center gap-2 rounded-lg border border-line bg-well px-3 py-2 text-[11.5px] text-ink-muted hover:border-accent-line hover:text-accent-text"
                  onClick={() => void api.openExternal(remote).catch(() => {})}
                  title="Open in browser"
                >
                  <Icon name="github" className="h-3.5 w-3.5 shrink-0" />
                  <span className="truncate">{remote}</span>
                  <Icon name="external" className="h-3 w-3 shrink-0" />
                </button>
              ) : (
                <p className="text-[11px] text-ink-subtle">
                  No <span className="mono">origin</span> remote on this
                  repository.
                </p>
              )}
            </section>
          </div>
        )}

        {tab === "agent" && (
          <div className="space-y-5">
            <AgentConfigForm
              value={cfg}
              onChange={patch}
              disabled={saveState === "saving"}
              inherit
            />

            <div className="rounded-lg border border-line bg-well p-3 text-[11.5px] text-ink-muted">
              <span className="text-ink-subtle">Effective: </span>
              {providerLabel(eff.provider)}
              {eff.model ? ` · ${eff.model}` : " · provider default"}
              {eff.fallback_provider
                ? ` · backup ${providerLabel(eff.fallback_provider)}`
                : ""}
              {` · review ${reviewModeLabel(eff.review_mode)}`}
            </div>
          </div>
        )}

        {tab === "environment" && (
          <EnvVarsSection
            vars={envVars}
            revealed={revealed}
            storeLabel={secretStoreLabel(settings.secret_store)}
            disabled={saveState === "saving"}
            onAdd={addEnvVar}
            onPatch={patchEnv}
            onRemove={removeEnvVar}
            onToggleReveal={(i) => setRevealed((r) => ({ ...r, [i]: !r[i] }))}
          />
        )}

        {tab === "prompt" && (
          <SystemPromptSection
            value={systemPrompt}
            disabled={saveState === "saving"}
            onChange={(p) => {
              setSystemPrompt((s) => ({ ...s, ...p }));
              setDirty(true);
            }}
          />
        )}

        {tab === "skills" && (
          <SkillsSection
            skills={skills}
            disabled={saveState === "saving"}
            taskAdded={flash === "skill-task"}
            onAdd={addSkill}
            onPatch={patchSkill}
            onRemove={removeSkill}
            onRunAsTask={(s) => void runSkillAsTask(s)}
          />
        )}

        {tab === "git" && (
          <section className="space-y-2">
            <SectionLabel icon="branch">Merge conflicts</SectionLabel>
            <p className="text-[11px] leading-relaxed text-ink-subtle">
              What happens when a Combine task, or a merge in the Ship workflow,
              hits a conflict.
            </p>
            <select
              className="select"
              disabled={saveState === "saving"}
              value={conflictMode}
              onChange={(e) => {
                setConflictMode(e.target.value as ConflictMode);
                setDirty(true);
              }}
            >
              {CONFLICT_MODES.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </select>
            <p className="text-[11px] leading-relaxed text-ink-subtle">
              {CONFLICT_MODES.find((m) => m.id === conflictMode)?.desc}
            </p>
            <p className="text-[11px] leading-relaxed text-ink-subtle">
              "Stop and wait for me" blocks the task: resolve the conflict in the
              project folder, then press{" "}
              <span className="text-ink-muted">Retry</span> to continue. The agent
              modes need a working agent (see the Agent tab).
            </p>
          </section>
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

function EnvVarsSection({
  vars,
  revealed,
  storeLabel,
  disabled,
  onAdd,
  onPatch,
  onRemove,
  onToggleReveal,
}: {
  vars: EnvValue[];
  revealed: Record<number, boolean>;
  storeLabel: string;
  disabled?: boolean;
  onAdd: () => void;
  onPatch: (index: number, p: Partial<EnvValue>) => void;
  onRemove: (index: number) => void;
  onToggleReveal: (index: number) => void;
}) {
  return (
    <section className="space-y-2">
      <SectionLabel icon="terminal">Environment variables</SectionLabel>
      <p className="text-[11px] leading-relaxed text-ink-subtle">
        Injected into every agent run in this project (tasks, reviews, and
        planning). Values are encrypted at rest in {storeLabel}.
      </p>
      {vars.length > 0 && (
        <div className="space-y-1.5">
          {vars.map((v, i) => (
            <div key={i} className="flex items-center gap-1.5">
              <input
                className="input mono !w-40 shrink-0 !text-[11.5px]"
                placeholder="NAME"
                disabled={disabled}
                value={v.key}
                onChange={(e) => onPatch(i, { key: e.target.value })}
              />
              <input
                className="input mono !text-[11.5px]"
                placeholder="value"
                disabled={disabled}
                type={revealed[i] ? "text" : "password"}
                value={v.value}
                onChange={(e) => onPatch(i, { value: e.target.value })}
              />
              <button
                type="button"
                className="btn btn-ghost shrink-0 !px-2"
                disabled={disabled}
                title={revealed[i] ? "Hide value" : "Reveal value"}
                aria-label={revealed[i] ? "Hide value" : "Reveal value"}
                onClick={() => onToggleReveal(i)}
              >
                <Icon name="eye" className="h-3.5 w-3.5" />
              </button>
              <label
                className="flex shrink-0 items-center gap-1 text-[10.5px] text-ink-subtle"
                title="Mask this value in the UI"
              >
                <input
                  type="checkbox"
                  disabled={disabled}
                  checked={v.secret}
                  onChange={(e) => onPatch(i, { secret: e.target.checked })}
                />
                mask
              </label>
              <button
                type="button"
                className="btn btn-ghost shrink-0 !px-2"
                disabled={disabled}
                title="Remove variable"
                aria-label="Remove variable"
                onClick={() => onRemove(i)}
              >
                <Icon name="trash" className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}
      <button className="btn btn-ghost" disabled={disabled} onClick={onAdd}>
        <Icon name="plus" className="h-3.5 w-3.5" />
        Add variable
      </button>
    </section>
  );
}

function SystemPromptSection({
  value,
  disabled,
  onChange,
}: {
  value: SystemPrompt;
  disabled?: boolean;
  onChange: (p: Partial<SystemPrompt>) => void;
}) {
  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between">
        <SectionLabel icon="sparkles">System prompt</SectionLabel>
        <label className="flex items-center gap-1.5 text-[11px] text-ink-muted">
          <input
            type="checkbox"
            disabled={disabled}
            checked={value.enabled}
            onChange={(e) => onChange({ enabled: e.target.checked })}
          />
          Enabled
        </label>
      </div>
      <p className="text-[11px] leading-relaxed text-ink-subtle">
        Added to every task prompt in this project. Variables:{" "}
        {PROMPT_VARIABLES.map(([token, desc], i) => (
          <span key={token}>
            {i > 0 && " · "}
            <span className="mono text-ink-muted">{token}</span>{" "}
            <span>({desc})</span>
          </span>
        ))}
        .
      </p>
      <div className="flex items-center gap-2">
        <select
          className="select !w-auto"
          disabled={disabled || !value.enabled}
          value={value.position}
          onChange={(e) =>
            onChange({ position: e.target.value as SystemPromptPosition })
          }
        >
          <option value="prefix">Before the prompt</option>
          <option value="suffix">After the prompt</option>
        </select>
      </div>
      <textarea
        className="input mono min-h-[84px] resize-y !text-[11.5px]"
        placeholder="e.g. This project uses pnpm. Always run tests on {{current_branch}} before finishing."
        disabled={disabled || !value.enabled}
        value={value.text}
        onChange={(e) => onChange({ text: e.target.value })}
      />
    </section>
  );
}

function SkillsSection({
  skills,
  disabled,
  taskAdded,
  onAdd,
  onPatch,
  onRemove,
  onRunAsTask,
}: {
  skills: ProjectSkill[];
  disabled?: boolean;
  taskAdded: boolean;
  onAdd: () => void;
  onPatch: (index: number, p: Partial<ProjectSkill>) => void;
  onRemove: (index: number) => void;
  onRunAsTask: (skill: ProjectSkill) => void;
}) {
  return (
    <section className="space-y-2">
      <div className="flex items-center justify-between">
        <SectionLabel icon="zap">Skills</SectionLabel>
        <SavedPill show={taskAdded} label="Task added" />
      </div>
      <p className="text-[11px] leading-relaxed text-ink-subtle">
        Reusable commands for this project. They are offered to the planner, and
        any skill can be added as a task that runs like the rest.
      </p>
      {skills.length > 0 && (
        <div className="space-y-2">
          {skills.map((s, i) => (
            <div
              key={s.id}
              className="space-y-1.5 rounded-lg border border-line bg-well p-2.5"
            >
              <div className="flex items-center gap-1.5">
                <input
                  className="input !text-[12px]"
                  placeholder="Skill name"
                  disabled={disabled}
                  value={s.name}
                  onChange={(e) => onPatch(i, { name: e.target.value })}
                />
                <button
                  type="button"
                  className="btn btn-ghost shrink-0"
                  disabled={disabled || !s.command.trim()}
                  title="Create a task from this skill"
                  onClick={() => onRunAsTask(s)}
                >
                  <Icon name="play" className="h-3.5 w-3.5" />
                  Add as task
                </button>
                <button
                  type="button"
                  className="btn btn-ghost shrink-0 !px-2"
                  disabled={disabled}
                  title="Remove skill"
                  aria-label="Remove skill"
                  onClick={() => onRemove(i)}
                >
                  <Icon name="trash" className="h-3.5 w-3.5" />
                </button>
              </div>
              <input
                className="input mono !text-[11.5px]"
                placeholder="command, e.g. pnpm test"
                disabled={disabled}
                value={s.command}
                onChange={(e) => onPatch(i, { command: e.target.value })}
              />
              <input
                className="input !text-[11.5px]"
                placeholder="Description (optional)"
                disabled={disabled}
                value={s.description}
                onChange={(e) => onPatch(i, { description: e.target.value })}
              />
            </div>
          ))}
        </div>
      )}
      <button className="btn btn-ghost" disabled={disabled} onClick={onAdd}>
        <Icon name="plus" className="h-3.5 w-3.5" />
        Add skill
      </button>
    </section>
  );
}
