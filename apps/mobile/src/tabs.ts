/**
 * The three primary mobile destinations (mobile spec section 7.1). Kept as a
 * plain data model so the shell and its tests share one source of truth.
 */
export type TabId = "sessions" | "inbox" | "projects";

export interface TabDef {
  id: TabId;
  /** Short label for the bottom tab. */
  label: string;
  /** Screen heading. */
  title: string;
  /** Placeholder copy shown until a host is connected. */
  empty: string;
  /** Decorative glyph; the label carries the meaning for screen readers. */
  icon: string;
}

export const TABS: readonly TabDef[] = [
  {
    id: "sessions",
    label: "Sessions",
    title: "Sessions",
    empty: "No desktop connected. Pair one to see active sessions.",
    icon: "≣",
  },
  {
    id: "inbox",
    label: "Inbox",
    title: "Inbox",
    empty: "Nothing needs your input right now.",
    icon: "✉",
  },
  {
    id: "projects",
    label: "Projects",
    title: "Projects",
    empty: "Projects from your connected desktop appear here.",
    icon: "▦",
  },
];
