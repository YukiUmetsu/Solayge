import { useState } from "react";
import type { Project, Snapshot, Task, TaskKind, TaskPatch } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { ErrorNote } from "./Field";
import { TaskFormFields, type TaskFormState } from "./TaskFormFields";
import { separationConfig, taskKindLabel, taskSeparation } from "../lib/providers";

/** Minutes left on a task's delay, for the Delay field. */
function remainingMinutes(notBefore?: number | null): number {
  if (!notBefore) return 0;
  const secs = notBefore - Math.floor(Date.now() / 1000);
  return secs > 0 ? Math.round(secs / 60) : 0;
}

/**
 * Edit a draft task. A draft has not been released yet, so every parameter the
 * New Task form can set is editable here; the backend rejects edits to any
 * other status.
 */
export function EditTaskModal({
  task,
  project,
  tasks,
  onClose,
  onSaved,
}: {
  task: Task;
  project: Project;
  tasks: Task[];
  onClose: () => void;
  onSaved: (s: Snapshot) => void;
}) {
  const specialKind: TaskKind | null =
    task.kind === "git" || task.kind === "merge" ? task.kind : null;
  const specialLabel = specialKind ? taskKindLabel(task) ?? specialKind : undefined;

  const [form, setForm] = useState<TaskFormState>(() => ({
    kind: task.kind ?? "agent",
    title: task.title,
    prompt: task.prompt,
    command: task.command ?? "",
    separation: taskSeparation(task),
    newBranch: task.new_branch ?? "",
    profile: task.profile,
    delayMinutes: remainingMinutes(task.not_before),
    baseRef: task.base_ref ?? "",
    deps: new Set(task.depends_on.filter((d) => d !== task.id)),
  }));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const patch = (p: Partial<TaskFormState>) =>
    setForm((f) => ({ ...f, ...p }));
  const dependencyOptions = tasks.filter((t) => t.id !== task.id);

  async function save() {
    // A git/merge task's action lives in the Ship / Combine flow; it can also be
    // switched here to a prompt or command.
    const special = form.kind === "git" || form.kind === "merge";
    const shell = form.kind === "shell";
    if (!special && shell && !form.command.trim()) {
      setError("A command is required.");
      return;
    }
    if (!special && !shell && !form.prompt.trim()) {
      setError("A prompt is required.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const { isolation, branch_mode } = separationConfig(form.separation);
      const patch: TaskPatch = {
        title: form.title.trim(),
        prompt: shell ? form.command.trim() : form.prompt,
        isolation,
        branch_mode,
        // Only a "new branch" task uses a branch name; clear it otherwise.
        new_branch: form.separation === "branch" ? form.newBranch.trim() : "",
        profile: form.profile,
        delay_seconds: Math.max(0, Math.round(form.delayMinutes * 60)),
        base_ref: form.baseRef.trim(),
        depends_on: [...form.deps],
      };
      if (!special) {
        patch.kind = shell ? "shell" : "agent";
        patch.command = shell ? form.command.trim() : "";
      }
      const snap = await api.updateTask(task.id, patch);
      onSaved(snap);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Edit draft task"
      subtitle="Change any parameter before the task is released."
      onClose={onClose}
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={save} disabled={busy}>
            <Icon name="check" className="h-3.5 w-3.5" />
            Save changes
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <TaskFormFields
          project={project}
          tasks={dependencyOptions}
          value={form}
          onChange={patch}
          specialKind={specialKind}
          specialLabel={specialLabel}
        />
        <ErrorNote error={error} />
      </div>
    </Modal>
  );
}
