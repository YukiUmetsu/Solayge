import { describe, expect, it } from "vitest";
import {
  coverageLabel,
  emptyFilters,
  errorCategoryLabel,
  errorGroupLabel,
  filtersActive,
  formatMillis,
  formatPercent,
  formatTokens,
  formatUsd,
  modelLabel,
  questionCategoryLabel,
  rangeForPreset,
  relativeDay,
} from "./analytics";
import type { AnalyticsModelMetric } from "../types";

describe("rangeForPreset", () => {
  it("returns unbounded filters for all time", () => {
    expect(rangeForPreset(0)).toEqual({ from_ms: null, to_ms: null });
  });

  it("starts at local midnight even for a one-day window", () => {
    // 2026-02-15T13:45 local.
    const now = new Date(2026, 1, 15, 13, 45, 0, 0).getTime();
    const { from_ms, to_ms } = rangeForPreset(1, now);
    expect(to_ms).toBeNull();
    const from = new Date(from_ms as number);
    expect(from.getFullYear()).toBe(2026);
    expect(from.getMonth()).toBe(1);
    expect(from.getDate()).toBe(15);
    expect(from.getHours()).toBe(0);
  });

  it("spans the whole window including today", () => {
    const now = new Date(2026, 1, 15, 9, 0, 0, 0).getTime();
    const { from_ms } = rangeForPreset(7, now);
    const from = new Date(from_ms as number);
    expect(from.getDate(), "seven days back from the 15th").toBe(9);
  });
});

describe("filtersActive", () => {
  it("is false for a date-only filter", () => {
    expect(filtersActive(emptyFilters())).toBe(false);
  });

  it("is true once a project or model is selected", () => {
    expect(filtersActive({ ...emptyFilters(), projects: ["/p"] })).toBe(true);
    expect(filtersActive({ ...emptyFilters(), models: ["m"] })).toBe(true);
  });
});

describe("formatTokens", () => {
  it("renders unknown as an em dash, never zero", () => {
    expect(formatTokens(null)).toBe("—");
    expect(formatTokens(undefined)).toBe("—");
  });

  it("scales through thousands, millions, billions", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
    expect(formatTokens(1500)).toBe("1.5K");
    expect(formatTokens(25000)).toBe("25K");
    expect(formatTokens(2_500_000)).toBe("2.50M");
    expect(formatTokens(2_500_000_000)).toBe("2.50B");
  });
});

describe("formatUsd", () => {
  it("renders unknown as an em dash", () => {
    expect(formatUsd(null)).toBe("—");
    expect(formatUsd(undefined)).toBe("—");
  });

  it("marks estimates with a tilde and keeps tiny values visible", () => {
    expect(formatUsd(0)).toBe("$0.00");
    expect(formatUsd(730_000)).toBe("~$0.73");
    expect(formatUsd(1_500_000)).toBe("~$1.50");
    expect(formatUsd(1_000)).toBe("~$0.0010");
  });
});

describe("formatPercent", () => {
  it("handles unknown and known rates", () => {
    expect(formatPercent(null)).toBe("—");
    expect(formatPercent(0.914)).toBe("91.4%");
    expect(formatPercent(0)).toBe("0.0%");
  });
});

describe("formatMillis", () => {
  it("reuses the compact duration style", () => {
    expect(formatMillis(null)).toBe("—");
    expect(formatMillis(60_000)).toBe("1m");
    expect(formatMillis(90_000)).toBe("1m");
    expect(formatMillis(3_720_000)).toBe("1h 2m");
  });
});

describe("modelLabel", () => {
  it("combines provider and model, hiding the default variant", () => {
    const base: AnalyticsModelMetric = {
      provider_id: "opencode-go",
      model_id: "m",
      variant: null,
      inference_count: 0,
      task_runs: 0,
      input_tokens: null,
      output_tokens: null,
      reasoning_tokens: null,
      cache_read_tokens: null,
      cache_write_tokens: null,
      total_tokens: null,
      reported_cost_micros: null,
      exclusive_task_runs: 0,
      exclusive_success: 0,
      exclusive_success_rate: null,
      mixed_task_runs: 0,
    };
    expect(modelLabel(base)).toBe("opencode-go/m");
    expect(modelLabel({ ...base, variant: "default" })).toBe("opencode-go/m");
    expect(modelLabel({ ...base, variant: "high" })).toBe("opencode-go/m (high)");
  });
});

describe("coverageLabel", () => {
  it("is null when no runs could report usage", () => {
    expect(
      coverageLabel({ usage_coverage_runs: 0, usage_runs_with_data: 0 }),
    ).toBeNull();
  });

  it("reports numerator and denominator", () => {
    expect(
      coverageLabel({ usage_coverage_runs: 20, usage_runs_with_data: 17 }),
    ).toBe("17 of 20 task runs with reported usage");
  });
});

describe("error + interaction labels", () => {
  it("falls back to the raw value for an unknown category", () => {
    expect(errorCategoryLabel("build")).toBe("Build");
    expect(errorCategoryLabel("new_thing")).toBe("new_thing");
    expect(questionCategoryLabel("credentials")).toBe("Credentials");
  });

  it("builds an error group label from category and code, never raw text", () => {
    expect(
      errorGroupLabel({ category: "build", code: "TS2322", tool_name: null }),
    ).toBe("Build · TS2322");
    expect(
      errorGroupLabel({ category: "tool", code: "ENOENT", tool_name: "shell" }),
    ).toBe("shell · ENOENT");
    expect(
      errorGroupLabel({ category: "unknown", code: "unknown", tool_name: null }),
    ).toBe("Unknown");
  });

  it("phrases relative days without inventing precision", () => {
    const now = new Date(2026, 9, 8, 12, 0, 0).getTime();
    const day = 86_400_000;
    expect(relativeDay(now, now)).toBe("Today");
    expect(relativeDay(now - day, now)).toBe("Yesterday");
    expect(relativeDay(now - 3 * day, now)).toBe("3 days ago");
    expect(relativeDay(now - 40 * day, now)).toMatch(/[A-Z][a-z]{2}/);
  });
});
