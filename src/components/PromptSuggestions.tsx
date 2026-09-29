import { useEffect, useState } from "react";
import type { PromptEntry } from "../types";
import { api } from "../api";
import { Icon } from "./Icons";

/**
 * Cached prompts you've used before, as one-click chips. Project prompts come
 * first, then prompts from other projects.
 */
export function PromptSuggestions({
  projectPath,
  limit = 12,
  onPick,
}: {
  projectPath?: string | null;
  limit?: number;
  onPick: (entry: PromptEntry) => void;
}) {
  const [items, setItems] = useState<PromptEntry[]>([]);

  useEffect(() => {
    let alive = true;
    api
      .promptHistory(null, 80)
      .then((all) => {
        if (!alive) return;
        const mine = all.filter((p) => p.project_path === projectPath);
        const rest = all.filter((p) => p.project_path !== projectPath);
        setItems([...mine, ...rest].slice(0, limit));
      })
      .catch(() => alive && setItems([]));
    return () => {
      alive = false;
    };
  }, [projectPath, limit]);

  if (items.length === 0) return null;

  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5 text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
        <Icon name="clock" className="h-3 w-3" />
        Reuse a cached prompt
      </div>
      <div className="flex flex-wrap gap-1.5">
        {items.map((it) => (
          <button
            key={it.id}
            type="button"
            title={it.prompt}
            onClick={() => onPick(it)}
            className="flex max-w-[240px] items-center gap-1 rounded-full border border-line bg-well px-2.5 py-1 text-[11px] text-ink-muted transition hover:border-accent-line hover:text-accent-text"
          >
            <span className="truncate">
              {it.title || it.prompt.slice(0, 40)}
            </span>
            {it.uses > 1 && (
              <span className="shrink-0 text-[10px] text-ink-faint">
                ×{it.uses}
              </span>
            )}
          </button>
        ))}
      </div>
    </div>
  );
}
