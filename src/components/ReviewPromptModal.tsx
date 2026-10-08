import { useEffect, useState } from "react";
import { api } from "../api";
import { Modal } from "./Modal";
import { Icon } from "./Icons";
import { ErrorNote, Field } from "./Field";

/**
 * Edit the reviewer's instructions before re-running an auto review. The field
 * is prefilled with the effective project/account review prompt; leaving it
 * blank falls back to the built-in skeptical reviewer prompt. The edit applies
 * to this run only.
 */
export function ReviewPromptModal({
  projectPath,
  onClose,
  onStart,
}: {
  projectPath: string | null;
  onClose: () => void;
  onStart: (prompt: string) => void;
}) {
  const [prompt, setPrompt] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Prefill with the prompt this review would use if left untouched, so the
  // user edits the live instructions rather than starting from a blank box.
  useEffect(() => {
    if (!projectPath) {
      setLoading(false);
      return;
    }
    let alive = true;
    api
      .resolvedConfig(projectPath)
      .then((c) => {
        if (alive) setPrompt(c.review_prompt ?? "");
      })
      .catch((e) => {
        if (alive) setError(String(e));
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [projectPath]);

  return (
    <Modal
      title="Edit review prompt"
      subtitle="The edited instructions are used for this review run only."
      onClose={onClose}
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            onClick={() => onStart(prompt)}
            disabled={loading}
          >
            <Icon name="retry" className="h-3.5 w-3.5" />
            Start review
          </button>
        </>
      }
    >
      <div className="space-y-3">
        <Field
          label="Reviewer instructions"
          hint="Blank uses the built-in skeptical reviewer prompt. The task context and the required REVIEW: verdict line are always added."
        >
          <textarea
            className="scroll mono min-h-48 w-full resize-y rounded-lg border border-line bg-well-strong px-3 py-2 text-[12px] leading-relaxed text-ink outline-none focus:border-accent-line"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="Leave blank to use the built-in skeptical reviewer prompt."
            disabled={loading}
            autoFocus
          />
        </Field>
        <ErrorNote error={error} />
      </div>
    </Modal>
  );
}
