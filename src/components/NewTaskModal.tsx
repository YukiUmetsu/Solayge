import { useState } from "react";
import type { Project, Snapshot, Task } from "../types";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { ErrorNote } from "./Field";
import {
  TaskFormFields,
  emptyTaskForm,
  type TaskFormState,
} from "./TaskFormFields";
import { separationConfig } from "../lib/providers";

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
  const [form, setForm] = useState<TaskFormState>(() =>
    emptyTaskForm(project.default_profile ?? "autonomous"),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const patch = (p: Partial<TaskFormState>) =>
    setForm((f) => ({ ...f, ...p }));

  async function create() {
    const shell = form.kind === "shell";
    const script = form.command.trim();
    if (shell && !script) {
      setError("A command is required.");
      return;
    }
    if (!shell && !form.prompt.trim()) {
      setError("A prompt is required.");
      return;
    }
    const text = shell ? script : form.prompt;
    const { isolation, branch_mode } = separationConfig(form.separation);
    setBusy(true);
    setError(null);
    try {
      const snap = await api.createTasks(
        project.path,
        [
          {
            title: form.title.trim() || text.trim().slice(0, 48),
            prompt: text,
            kind: shell ? "shell" : "agent",
            command: shell ? script : null,
            isolation,
            branch_mode,
            new_branch:
              form.separation === "branch" ? form.newBranch.trim() || null : null,
            profile: form.profile,
            after: [...form.deps],
            delay_seconds: Math.max(0, Math.round(form.delayMinutes * 60)),
            base_ref: form.baseRef.trim() || null,
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
        <TaskFormFields
          project={project}
          tasks={tasks}
          value={form}
          onChange={patch}
        />
        <ErrorNote error={error} />
      </div>
    </Modal>
  );
}
