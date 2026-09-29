import { useMemo, useState } from "react";
import type { GitOp, NewTask, Project, Snapshot } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Field, SectionLabel } from "./AgentConfigForm";
import { Icon, type IconName } from "./Icons";

type StepKind =
  | "commit"
  | "tests"
  | "ci"
  | "push"
  | "pr"
  | "merge_pr"
  | "checkout"
  | "pull"
  | "skill"
  | "shell";

interface ShipStep {
  id: string;
  kind: StepKind;
  enabled: boolean;
  skillId?: string;
  command?: string;
}

const STEP_META: Record<StepKind, { label: string; icon: IconName }> = {
  commit: { label: "Commit changes", icon: "check" },
  tests: { label: "Run tests", icon: "terminal" },
  ci: { label: "Wait for PR checks", icon: "clock" },
  push: { label: "Push branch", icon: "branch" },
  pr: { label: "Create pull request", icon: "github" },
  merge_pr: { label: "Merge pull request", icon: "diff" },
  checkout: { label: "Check out default branch", icon: "branch" },
  pull: { label: "Pull latest", icon: "refresh" },
  skill: { label: "Run a skill", icon: "zap" },
  shell: { label: "Run a command", icon: "terminal" },
};

const PALETTE: StepKind[] = [
  "commit",
  "tests",
  "ci",
  "push",
  "pr",
  "merge_pr",
  "checkout",
  "pull",
  "skill",
  "shell",
];

function uid(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `s-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  }
}

function step(kind: StepKind, extra: Partial<ShipStep> = {}): ShipStep {
  return { id: uid(), kind, enabled: true, ...extra };
}

const DEFAULT_STEPS: ShipStep[] = [
  step("commit"),
  step("push"),
  step("pr"),
  step("merge_pr"),
  step("checkout"),
  step("pull"),
];

/** Builds the ordered chain of tasks from the step list. */
export function ShipModal({
  project,
  onClose,
  onCreated,
}: {
  project: Project;
  onClose: () => void;
  onCreated: (s: Snapshot) => void;
}) {
  const skills = project.skills ?? [];
  const [steps, setSteps] = useState<ShipStep[]>(DEFAULT_STEPS);
  const [commitMessage, setCommitMessage] = useState("");
  const [prTitle, setPrTitle] = useState(`Ship: ${project.name}`);
  const [mergeMethod, setMergeMethod] = useState("squash");
  const [testCommand, setTestCommand] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const hasTests = steps.some((s) => s.kind === "tests");
  const hasCi = steps.some((s) => s.kind === "ci");

  function patch(id: string, p: Partial<ShipStep>) {
    setSteps((cur) => cur.map((s) => (s.id === id ? { ...s, ...p } : s)));
  }
  function remove(id: string) {
    setSteps((cur) => cur.filter((s) => s.id !== id));
  }
  function move(id: string, delta: number) {
    setSteps((cur) => {
      const i = cur.findIndex((s) => s.id === id);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= cur.length) return cur;
      const next = [...cur];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
  }
  function addStep(kind: StepKind) {
    if (kind === "skill" && skills.length === 0) return;
    setSteps((cur) => [
      ...cur,
      step(kind, kind === "skill" ? { skillId: skills[0]?.id } : {}),
    ]);
  }
  /** Insert/remove the test gate right before the PR step. */
  function setTestGate(on: boolean) {
    setSteps((cur) => {
      if (!on) return cur.filter((s) => s.kind !== "tests");
      if (cur.some((s) => s.kind === "tests")) return cur;
      const at = cur.findIndex((s) => s.kind === "pr");
      const next = [...cur];
      next.splice(at < 0 ? next.length : at, 0, step("tests"));
      return next;
    });
  }
  /** Insert/remove the CI gate right before the merge step. */
  function setCiGate(on: boolean) {
    setSteps((cur) => {
      if (!on) return cur.filter((s) => s.kind !== "ci");
      if (cur.some((s) => s.kind === "ci")) return cur;
      const at = cur.findIndex((s) => s.kind === "merge_pr");
      const next = [...cur];
      next.splice(at < 0 ? next.length : at, 0, step("ci"));
      return next;
    });
  }

  const built = useMemo(() => buildTasks(), [steps, commitMessage, prTitle, mergeMethod, testCommand, skills]);

  function buildTasks(): NewTask[] {
    const out: NewTask[] = [];
    let prev: string | null = null;
    const git = (
      localId: string,
      title: string,
      op: GitOp,
      command?: string,
      prompt?: string,
    ): NewTask => ({
      local_id: localId,
      title,
      prompt: prompt ?? command ?? title,
      kind: "git",
      git_op: op,
      command,
      isolation: "shared",
    });

    for (const s of steps) {
      if (!s.enabled) continue;
      let task: NewTask | null = null;
      switch (s.kind) {
        case "commit":
          task = git(s.id, "Commit changes", "add_commit", commitMessage, commitMessage || "chore: update");
          break;
        case "tests":
          if (!testCommand.trim()) break;
          task = { local_id: s.id, title: "Run tests", prompt: testCommand, kind: "shell", command: testCommand, isolation: "shared" };
          break;
        case "ci":
          task = {
            local_id: s.id,
            title: "Wait for PR checks",
            prompt: "gh pr checks --watch --fail-fast",
            kind: "shell",
            command: "gh pr checks --watch --fail-fast",
            isolation: "shared",
          };
          break;
        case "push":
          task = git(s.id, "Push branch", "push");
          break;
        case "pr":
          task = git(s.id, "Create pull request", "pr_create", prTitle, prTitle);
          break;
        case "merge_pr":
          task = git(s.id, "Merge pull request", "pr_merge", mergeMethod, `gh pr merge ${mergeMethod}`);
          break;
        case "checkout":
          task = git(s.id, "Check out the default branch", "checkout");
          break;
        case "pull":
          task = git(s.id, "Pull latest", "pull");
          break;
        case "skill": {
          const sk = skills.find((x) => x.id === s.skillId);
          if (!sk || !sk.command.trim()) break;
          task = { local_id: s.id, title: sk.name, prompt: sk.command, kind: "shell", command: sk.command, isolation: "shared" };
          break;
        }
        case "shell":
          if (!s.command?.trim()) break;
          task = { local_id: s.id, title: s.command.slice(0, 48), prompt: s.command, kind: "shell", command: s.command, isolation: "shared" };
          break;
      }
      if (!task) continue;
      task.after = prev ? [prev] : [];
      out.push(task);
      prev = s.id;
    }
    return out;
  }

  async function submit() {
    setError(null);
    if (hasTests && !testCommand.trim()) {
      setError("Enter a test command for the test gate, or turn the gate off.");
      return;
    }
    if (built.length === 0) {
      setError("Add at least one step and give it a value.");
      return;
    }
    setBusy(true);
    try {
      onCreated(await api.createTasks(project.path, built, "shared"));
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Ship"
      subtitle="Compose a release chain. Each step becomes a task in the graph."
      onClose={onClose}
      width="max-w-2xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={submit} disabled={busy}>
            <Icon name="zap" className="h-3.5 w-3.5" />
            {busy
              ? "Creating…"
              : `Create ${built.length} task${built.length === 1 ? "" : "s"}`}
          </button>
        </>
      }
    >
      <div className="space-y-5">
        <section className="space-y-2">
          <SectionLabel icon="layers">Gates</SectionLabel>
          <label className="flex items-center gap-2 text-[12px] text-ink-muted">
            <input
              type="checkbox"
              checked={hasTests}
              onChange={(e) => setTestGate(e.target.checked)}
            />
            Tests must pass before the pull request is created
          </label>
          <label className="flex items-center gap-2 text-[12px] text-ink-muted">
            <input
              type="checkbox"
              checked={hasCi}
              onChange={(e) => setCiGate(e.target.checked)}
            />
            PR checks must pass before the pull request is merged
          </label>
          {(hasTests || hasCi) && (
            <Field label="Test / check command">
              <input
                className="input mono !text-[11.5px]"
                placeholder="e.g. pnpm test"
                value={testCommand}
                onChange={(e) => setTestCommand(e.target.value)}
              />
            </Field>
          )}
          <p className="text-[11px] text-ink-subtle">
            The CI gate runs <span className="mono">gh pr checks --watch --fail-fast</span>{" "}
            and needs the GitHub CLI.
          </p>
        </section>

        <section className="space-y-2">
          <SectionLabel icon="branch">Steps</SectionLabel>
          <div className="space-y-1.5">
            {steps.map((s, i) => (
              <StepRow
                key={s.id}
                step={s}
                index={i}
                total={steps.length}
                skills={skills}
                commitMessage={commitMessage}
                prTitle={prTitle}
                mergeMethod={mergeMethod}
                onCommitMessage={setCommitMessage}
                onPrTitle={setPrTitle}
                onMergeMethod={setMergeMethod}
                onPatch={(p) => patch(s.id, p)}
                onRemove={() => remove(s.id)}
                onMove={(d) => move(s.id, d)}
              />
            ))}
            {steps.length === 0 && (
              <p className="text-[11px] text-ink-subtle">
                No steps yet — add one below.
              </p>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-1.5 pt-1">
            <span className="text-[11px] text-ink-subtle">Add:</span>
            {PALETTE.map((kind) => (
              <button
                key={kind}
                className="btn btn-ghost !px-2 !py-1 !text-[11px]"
                onClick={() => addStep(kind)}
                disabled={kind === "skill" && skills.length === 0}
                title={
                  kind === "skill" && skills.length === 0
                    ? "No project skills defined"
                    : `Add "${STEP_META[kind].label}"`
                }
              >
                <Icon name={STEP_META[kind].icon} className="h-3 w-3" />
                {STEP_META[kind].label}
              </button>
            ))}
          </div>
        </section>

        <p className="rounded-lg border border-line bg-well p-3 text-[11.5px] leading-relaxed text-ink-muted">
          Steps run in order in the project folder, on its current branch. They are
          normal tasks — nothing runs until you press{" "}
          <span className="text-ink">Execute</span>.
        </p>

        {error && (
          <div className="rounded-lg border border-danger-line bg-danger-soft p-2.5 text-[12px] text-danger">
            {error}
          </div>
        )}
      </div>
    </Modal>
  );
}

function StepRow({
  step,
  index,
  total,
  skills,
  commitMessage,
  prTitle,
  mergeMethod,
  onCommitMessage,
  onPrTitle,
  onMergeMethod,
  onPatch,
  onRemove,
  onMove,
}: {
  step: ShipStep;
  index: number;
  total: number;
  skills: Project["skills"];
  commitMessage: string;
  prTitle: string;
  mergeMethod: string;
  onCommitMessage: (v: string) => void;
  onPrTitle: (v: string) => void;
  onMergeMethod: (v: string) => void;
  onPatch: (p: Partial<ShipStep>) => void;
  onRemove: () => void;
  onMove: (delta: number) => void;
}) {
  const meta = STEP_META[step.kind];
  return (
    <div
      className={`rounded-lg border px-3 py-2 transition ${
        step.enabled ? "border-line bg-well" : "border-line/60 bg-transparent opacity-60"
      }`}
    >
      <div className="flex items-center gap-2">
        <span className="mono flex h-5 w-5 items-center justify-center rounded-full bg-accent-soft text-[10px] text-accent-text">
          {index + 1}
        </span>
        <label className="flex flex-1 items-center gap-2 text-[12px]">
          <input
            type="checkbox"
            checked={step.enabled}
            onChange={(e) => onPatch({ enabled: e.target.checked })}
          />
          <Icon name={meta.icon} className="h-3.5 w-3.5 text-ink-subtle" />
          <span className="font-medium text-ink">{meta.label}</span>
        </label>
        <div className="flex items-center gap-0.5">
          <button
            className="rounded p-1 text-ink-subtle transition hover:bg-hover hover:text-ink disabled:opacity-30"
            onClick={() => onMove(-1)}
            disabled={index === 0}
            title="Move up"
          >
            <Icon name="chevron" className="h-3.5 w-3.5 -rotate-90" />
          </button>
          <button
            className="rounded p-1 text-ink-subtle transition hover:bg-hover hover:text-ink disabled:opacity-30"
            onClick={() => onMove(1)}
            disabled={index === total - 1}
            title="Move down"
          >
            <Icon name="chevron" className="h-3.5 w-3.5 rotate-90" />
          </button>
          <button
            className="rounded p-1 text-ink-subtle transition hover:bg-hover hover:text-danger"
            onClick={onRemove}
            title="Remove step"
          >
            <Icon name="x" className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {step.enabled && (
        <div className="mt-2 pl-9">
          {step.kind === "commit" && (
            <input
              className="input mono !text-[11.5px]"
              placeholder="Commit message (defaults to chore: update)"
              value={commitMessage}
              onChange={(e) => onCommitMessage(e.target.value)}
            />
          )}
          {step.kind === "pr" && (
            <input
              className="input !text-[11.5px]"
              value={prTitle}
              onChange={(e) => onPrTitle(e.target.value)}
            />
          )}
          {step.kind === "merge_pr" && (
            <select
              className="select"
              value={mergeMethod}
              onChange={(e) => onMergeMethod(e.target.value)}
            >
              <option value="squash">Squash and merge</option>
              <option value="merge">Create a merge commit</option>
              <option value="rebase">Rebase and merge</option>
            </select>
          )}
          {step.kind === "skill" && (
            <select
              className="select"
              value={step.skillId ?? ""}
              onChange={(e) => onPatch({ skillId: e.target.value })}
            >
              {skills.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name} — {s.command}
                </option>
              ))}
            </select>
          )}
          {step.kind === "shell" && (
            <input
              className="input mono !text-[11.5px]"
              placeholder="command, e.g. pnpm build"
              value={step.command ?? ""}
              onChange={(e) => onPatch({ command: e.target.value })}
            />
          )}
          {(step.kind === "push" ||
            step.kind === "checkout" ||
            step.kind === "pull" ||
            step.kind === "ci") && (
            <p className="mono text-[10.5px] text-ink-subtle">
              {step.kind === "ci"
                ? "gh pr checks --watch --fail-fast"
                : step.kind === "checkout"
                  ? "git checkout <default branch>"
                  : `git ${step.kind}`}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
