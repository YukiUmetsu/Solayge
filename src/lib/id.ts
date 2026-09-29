/**
 * A unique id for a locally-created item (a skill, a ship step). Uses the
 * platform UUID generator when it is available and falls back to a
 * timestamp/random string otherwise.
 */
export function newId(prefix = "id"): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  }
}
