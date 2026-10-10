import { open } from "@tauri-apps/plugin-dialog";

/** The last path segment of a file path, for display. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/**
 * Merge newly-picked files into an existing list, dropping blanks and
 * duplicates (the same file attached twice must appear once).
 */
export function mergeAttachments(
  existing: string[],
  incoming: string[],
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of [...existing, ...incoming]) {
    const path = raw.trim();
    if (!path || seen.has(path)) continue;
    seen.add(path);
    out.push(path);
  }
  return out;
}

/** Drop one file from the list. */
export function removeAttachment(list: string[], path: string): string[] {
  return list.filter((p) => p !== path);
}

/**
 * Open the OS file picker for one or more files and return the chosen absolute
 * paths. Returns an empty list when the user cancels.
 */
export async function pickFiles(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    directory: false,
    title: "Attach files",
  });
  if (!picked) return [];
  const list = Array.isArray(picked) ? picked : [picked];
  return list.filter((p): p is string => typeof p === "string" && p.length > 0);
}
