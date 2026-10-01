import type { Task } from "../types";
import Markdown from "./Markdown";

/**
 * The agent's final markdown summary, rendered for reading. Kept in its own
 * module and lazy-loaded so the markdown renderer stays out of the main bundle
 * until a task result is actually opened.
 */
export default function ResultView({ task }: { task: Task }) {
  const markdown = task.result?.trim();
  if (!markdown) {
    return (
      <div className="flex flex-1 items-center justify-center px-8 text-center">
        <p className="text-xs leading-relaxed text-ink-subtle">
          This task has no captured result.
        </p>
      </div>
    );
  }
  return (
    <div className="scroll min-h-0 flex-1 p-4">
      <Markdown>{markdown}</Markdown>
    </div>
  );
}
