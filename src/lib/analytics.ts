import type { AnalyticsFilters, AnalyticsModelMetric } from "../types";
import { compactDuration } from "./format";

export const DAY_MS = 86_400_000;

/** Date-range presets in days; `0` is all time. */
export type RangePreset = 7 | 30 | 90 | 0;

/**
 * The `from_ms`/`to_ms` window for a preset. The lower bound is local midnight
 * `days - 1` days ago, so "Last 7 days" includes the whole of today plus the six
 * days before it. All time has no bounds.
 */
export function rangeForPreset(
  days: number,
  nowMs: number = Date.now(),
): { from_ms: number | null; to_ms: number | null } {
  if (days === 0) return { from_ms: null, to_ms: null };
  const start = new Date(nowMs);
  start.setHours(0, 0, 0, 0);
  start.setDate(start.getDate() - (days - 1));
  return { from_ms: start.getTime(), to_ms: null };
}

export function emptyFilters(): AnalyticsFilters {
  return { from_ms: null, to_ms: null, projects: [], models: [] };
}

/** Whether any non-date filter is narrowing the view. */
export function filtersActive(filters: AnalyticsFilters): boolean {
  return filters.projects.length > 0 || filters.models.length > 0;
}

/** Compact token count. `null`/`undefined` is unknown, rendered as an em dash. */
export function formatTokens(n: number | null | undefined): string {
  if (n === null || n === undefined) return "—";
  if (n < 1000) return String(n);
  if (n < 1_000_000) {
    const k = n / 1000;
    return `${k.toFixed(n < 10_000 ? 1 : 0)}K`;
  }
  if (n < 1_000_000_000) {
    const m = n / 1_000_000;
    return `${m.toFixed(n < 10_000_000 ? 2 : 1)}M`;
  }
  return `${(n / 1_000_000_000).toFixed(2)}B`;
}

/**
 * A provider-reported cost, from integer micro-USD. `null` is unknown (never
 * `$0.00`). The `~` marks that the figure is an estimate, not an actual charge.
 */
export function formatUsd(micros: number | null | undefined): string {
  if (micros === null || micros === undefined) return "—";
  if (micros === 0) return "$0.00";
  const usd = micros / 1_000_000;
  if (usd < 0.01) return `~$${usd.toFixed(4)}`;
  if (usd < 1000) return `~$${usd.toFixed(2)}`;
  return `~$${usd.toLocaleString(undefined, { maximumFractionDigits: 0 })}`;
}

/** A fraction 0..1 as a percentage; `null` is unknown. */
export function formatPercent(
  value: number | null | undefined,
  digits = 1,
): string {
  if (value === null || value === undefined) return "—";
  return `${(value * 100).toFixed(digits)}%`;
}

/** A millisecond duration using the app's compact style; `null` is unknown. */
export function formatMillis(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "—";
  return compactDuration(Math.round(ms / 1000));
}

/** A local short date for a day bucket (e.g. "Oct 6"). */
export function formatDay(bucketMs: number): string {
  return new Date(bucketMs).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/** The provider/model label for a model row. */
export function modelLabel(m: AnalyticsModelMetric): string {
  const base = m.provider_id ? `${m.provider_id}/${m.model_id}` : m.model_id;
  return m.variant && m.variant !== "default" ? `${base} (${m.variant})` : base;
}

/** A human phrase for usage coverage, or null when there is nothing to cover. */
export function coverageLabel(overview: {
  usage_coverage_runs: number;
  usage_runs_with_data: number;
}): string | null {
  if (overview.usage_coverage_runs === 0) return null;
  return `${overview.usage_runs_with_data} of ${overview.usage_coverage_runs} task runs with reported usage`;
}

/** Human labels for the backend error taxonomy. */
export const ERROR_CATEGORY_LABELS: Record<string, string> = {
  provider: "Provider",
  tool: "Tool",
  build: "Build",
  test: "Test",
  security_policy: "Security policy",
  agent_runtime: "Agent runtime",
  integration: "Integration",
  unknown: "Unknown",
};

export function errorCategoryLabel(category: string): string {
  return ERROR_CATEGORY_LABELS[category] ?? category;
}

/** Human labels for interaction categories. */
export const QUESTION_CATEGORY_LABELS: Record<string, string> = {
  missing_requirements: "Missing requirements",
  implementation_choice: "Implementation choice",
  missing_repository_instructions: "Repository instructions",
  environment_setup: "Environment setup",
  permissions: "Permissions",
  credentials: "Credentials",
  approval_deployment: "Approval / deployment",
  ambiguous_success_criteria: "Success criteria",
  other: "Other",
};

export function questionCategoryLabel(category: string): string {
  return QUESTION_CATEGORY_LABELS[category] ?? category;
}

export function interactionKindLabel(kind: string): string {
  if (kind === "question") return "Question";
  if (kind === "permission") return "Permission";
  if (kind === "manual_confirmation") return "Manual confirmation";
  return kind;
}

/**
 * A readable label for an error group, built from its category and code — never
 * from raw error text, so nothing sensitive can leak into the table.
 */
export function errorGroupLabel(g: {
  category: string;
  code: string;
  tool_name: string | null;
}): string {
  const code = g.code && g.code !== "unknown" ? g.code : null;
  const base = errorCategoryLabel(g.category);
  if (g.category === "tool" && g.tool_name) {
    return code ? `${g.tool_name} · ${code}` : g.tool_name;
  }
  return code ? `${base} · ${code}` : base;
}

/** Relative "last seen" phrasing from a millisecond timestamp. */
export function relativeDay(ms: number, nowMs: number = Date.now()): string {
  const day = 86_400_000;
  const start = (t: number) => {
    const d = new Date(t);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };
  const days = Math.round((start(nowMs) - start(ms)) / day);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days} days ago`;
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}
