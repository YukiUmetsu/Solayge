import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { apiMock, saveMock } = vi.hoisted(() => ({
  apiMock: {
    analyticsOverview: vi.fn(),
    analyticsTimeseries: vi.fn(),
    analyticsProjects: vi.fn(),
    analyticsModels: vi.fn(),
    analyticsReliability: vi.fn(),
    analyticsInteractions: vi.fn(),
    analyticsCollectionStatus: vi.fn(),
    analyticsTaskDetail: vi.fn(),
    exportAnalytics: vi.fn(),
  },
  saveMock: vi.fn(),
}));
vi.mock("../../api", () => ({ api: apiMock }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveMock }));

import { AnalyticsPage } from "./AnalyticsPage";
import type {
  AnalyticsOverview,
  AnalyticsProjectMetric,
  InteractionResult,
  ReliabilityResult,
} from "../../types";
import { makeProject, makeSnapshot, makeTask } from "../../test/fixtures";

function overview(overrides: Partial<AnalyticsOverview> = {}): AnalyticsOverview {
  return {
    has_data: true,
    tasks_total: 34,
    tasks_completed: 12,
    tasks_failed: 1,
    tasks_canceled: 0,
    tasks_blocked: 0,
    tasks_interrupted: 0,
    success_rate: 12 / 13,
    success_sample: 13,
    avg_active_ms: 600_000,
    median_active_ms: 540_000,
    duration_sample: 13,
    avg_wait_ms: 1000,
    total_tokens: 1_500_000,
    input_tokens: 100_000,
    output_tokens: 200_000,
    reasoning_tokens: 50_000,
    cache_read_tokens: 1_150_000,
    cache_write_tokens: 0,
    reported_cost_micros: 730_000,
    inference_count: 42,
    usage_coverage_runs: 20,
    usage_runs_with_data: 17,
    currency: "USD",
    cost_basis: "provider_reported_not_a_bill",
    runs_with_errors: 2,
    error_events: 3,
    retry_rate: 0.1,
    first_pass_success_rate: 0.9,
    autonomy_rate: 0.75,
    autonomy_sample: 8,
    generated_at_ms: 1_700_000_000_000,
    ...overrides,
  };
}

function reliability(
  overrides: Partial<ReliabilityResult> = {},
): ReliabilityResult {
  return {
    summary: {
      has_data: true,
      tasks_failed: 1,
      failed_sample: 13,
      task_failure_rate: 1 / 13,
      runs_with_errors: 2,
      observed_runs: 10,
      runs_with_errors_rate: 0.2,
      error_events: 3,
      recoverable_errors: 3,
      recovered_errors: 2,
      recovery_rate: 2 / 3,
      retry_tasks: 1,
      attempt_tasks: 10,
      retry_rate: 0.1,
      first_pass_success: 11,
      first_pass_sample: 13,
      first_pass_success_rate: 11 / 13,
    },
    categories: [
      {
        category: "build",
        occurrences: 2,
        tasks: 1,
        recovered: 2,
        recoverable: 2,
        recovery_rate: 1,
      },
    ],
    groups: [
      {
        fingerprint: "build:TS2322",
        category: "build",
        code: "TS2322",
        stage: "run",
        tool_name: null,
        occurrences: 2,
        tasks: 1,
        recovered: 2,
        recoverable: 2,
        unresolved: 0,
        first_seen_ms: 1_700_000_000_000,
        last_seen_ms: 1_700_000_100_000,
        sample: null,
        task_ids: ["task-1"],
      },
    ],
    stages: [{ name: "run", occurrences: 2, tasks: 1, recovered: 2 }],
    tools: [],
    series: [
      { bucket_ms: 1_700_000_000_000, errors: 2, tasks_failed: 1 },
    ],
    ...overrides,
  };
}

function interactions(
  overrides: Partial<InteractionResult> = {},
): InteractionResult {
  return {
    summary: {
      has_data: true,
      questions: 3,
      permissions: 1,
      manual_confirmations: 0,
      blocking_total: 4,
      resolved: 3,
      unresolved: 1,
      total_wait_ms: 120_000,
      avg_wait_ms: 40_000,
      median_wait_ms: 30_000,
      p95_wait_ms: 60_000,
      autonomy_rate: 0.5,
      autonomy_autonomous: 4,
      autonomy_sample: 8,
      first_pass_success_rate: 0.9,
    },
    groups: [
      {
        fingerprint: "question:environment",
        kind: "question",
        category: "environment_setup",
        occurrences: 3,
        tasks: 2,
        blocking: 3,
        unresolved: 1,
        avg_wait_ms: 40_000,
        last_seen_ms: 1_700_000_100_000,
        task_ids: ["task-1"],
      },
    ],
    categories: [
      {
        category: "environment_setup",
        kind: "question",
        occurrences: 3,
        tasks: 2,
        unresolved: 1,
        avg_wait_ms: 40_000,
      },
    ],
    recommendations: [
      {
        title: "Describe the dev environment",
        detail: "Repeated setup questions suggest documenting versions.",
        count: 3,
      },
    ],
    ...overrides,
  };
}

function projectMetric(): AnalyticsProjectMetric {
  return {
    project_path: "/tmp/solayge-project",
    project_name: "solayge-project",
    tasks_total: 13,
    tasks_completed: 12,
    tasks_failed: 1,
    success_rate: 12 / 13,
    success_sample: 13,
    avg_active_ms: 600_000,
    median_active_ms: 540_000,
    total_tokens: 1_500_000,
    reported_cost_micros: 730_000,
    inference_count: 42,
  };
}

beforeEach(() => {
  apiMock.analyticsOverview.mockReset().mockResolvedValue(overview());
  apiMock.analyticsTimeseries.mockReset().mockResolvedValue([]);
  apiMock.analyticsProjects.mockReset().mockResolvedValue([projectMetric()]);
  apiMock.analyticsModels.mockReset().mockResolvedValue([]);
  apiMock.analyticsReliability.mockReset().mockResolvedValue(reliability());
  apiMock.analyticsInteractions.mockReset().mockResolvedValue(interactions());
  apiMock.analyticsCollectionStatus.mockReset().mockResolvedValue({
    enabled: true,
    retention_days: 90,
    run_count: 13,
    usage_count: 42,
    error_count: 3,
    interaction_count: 4,
    usage_complete: 40,
    usage_partial: 2,
    capture_error_examples: false,
    oldest_at_ms: 1_690_000_000_000,
    newest_at_ms: 1_700_000_000_000,
  });
  apiMock.analyticsTaskDetail.mockReset().mockResolvedValue(null);
  apiMock.exportAnalytics.mockReset().mockResolvedValue(undefined);
  saveMock.mockReset();
});

function renderPage() {
  return render(
    <AnalyticsPage
      snapshot={makeSnapshot({
        projects: [makeProject()],
        tasks: [],
      })}
      initialProject={null}
      onOpenTask={vi.fn()}
      onOpenProject={vi.fn()}
    />,
  );
}

describe("AnalyticsPage", () => {
  it("shows the key metrics and labels cost as a provider estimate", async () => {
    renderPage();
    expect(await screen.findByText("Tasks completed")).toBeInTheDocument();
    // Values also appear in the project table, so assert presence, not uniqueness.
    expect(screen.getAllByText("12").length).toBeGreaterThan(0);
    expect(screen.getAllByText("92.3%").length).toBeGreaterThan(0);
    // Tokens use the compact form and cost is marked as an estimate.
    expect(screen.getAllByText("1.50M").length).toBeGreaterThan(0);
    expect(screen.getAllByText("~$0.73").length).toBeGreaterThan(0);
    expect(
      screen.getByText("provider-calculated · not a bill"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("17 of 20 task runs with reported usage"),
    ).toBeInTheDocument();
  });

  it("shows an honest empty state when no task data exists", async () => {
    apiMock.analyticsOverview.mockResolvedValue(
      overview({ has_data: false, tasks_total: 0, tasks_completed: 0 }),
    );
    renderPage();
    expect(await screen.findByText("No task data yet")).toBeInTheDocument();
    // No fake zero KPI tiles.
    expect(screen.queryByText("Tasks completed")).not.toBeInTheDocument();
  });

  it("reports analytics failures without crashing", async () => {
    apiMock.analyticsOverview.mockRejectedValue(new Error("backend offline"));
    renderPage();
    expect(
      await screen.findByText(/Analytics could not load: Error: backend offline/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("opens the Projects & Models tab and renders model usage", async () => {
    const user = userEvent.setup();
    apiMock.analyticsModels.mockResolvedValue([
      {
        provider_id: "opencode-go",
        model_id: "deepseek-v4.1-flash",
        variant: null,
        inference_count: 42,
        task_runs: 8,
        input_tokens: 100_000,
        output_tokens: 200_000,
        reasoning_tokens: 50_000,
        cache_read_tokens: 1_150_000,
        cache_write_tokens: 0,
        total_tokens: 1_500_000,
        reported_cost_micros: 730_000,
        exclusive_task_runs: 8,
        exclusive_success: 7,
        exclusive_success_rate: 7 / 8,
        mixed_task_runs: 1,
      },
    ]);
    renderPage();
    await user.click(
      await screen.findByRole("tab", { name: /projects & models/i }),
    );
    expect(
      screen.getAllByText("opencode-go/deepseek-v4.1-flash").length,
    ).toBeGreaterThan(0);
    expect(screen.getAllByText("Reported cost").length).toBeGreaterThan(0);
  });

  it("filters to a project when its row is clicked", async () => {
    const user = userEvent.setup();
    renderPage();
    // The project name appears as a filter button in the top table.
    await user.click(await screen.findByRole("button", { name: "solayge-project" }));
    await waitFor(() =>
      expect(apiMock.analyticsProjects).toHaveBeenLastCalledWith(
        expect.objectContaining({ projects: ["/tmp/solayge-project"] }),
      ),
    );
  });

  it("resets the project filter when the global shortcut clears it", async () => {
    const { rerender } = render(
      <AnalyticsPage
        snapshot={makeSnapshot({ projects: [makeProject()], tasks: [] })}
        initialProject="/tmp/solayge-project"
        onOpenTask={vi.fn()}
        onOpenProject={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(apiMock.analyticsProjects).toHaveBeenLastCalledWith(
        expect.objectContaining({ projects: ["/tmp/solayge-project"] }),
      ),
    );
    rerender(
      <AnalyticsPage
        snapshot={makeSnapshot({ projects: [makeProject()], tasks: [] })}
        initialProject={null}
        onOpenTask={vi.fn()}
        onOpenProject={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(apiMock.analyticsProjects).toHaveBeenLastCalledWith(
        expect.objectContaining({ projects: [] }),
      ),
    );
  });

  it("sorts the project table by the column whose header was clicked", async () => {
    const user = userEvent.setup();
    apiMock.analyticsProjects.mockResolvedValue([
      {
        ...projectMetric(),
        project_path: "/tmp/a",
        project_name: "proj-a",
        tasks_completed: 5,
        tasks_failed: 1,
      },
      {
        ...projectMetric(),
        project_path: "/tmp/b",
        project_name: "proj-b",
        tasks_completed: 1,
        tasks_failed: 9,
      },
    ]);
    renderPage();
    await user.click(
      await screen.findByRole("tab", { name: /projects & models/i }),
    );
    // Default sort is completed descending: a (5) before b (1).
    let names = screen
      .getAllByRole("button", { name: /^proj-/ })
      .map((b) => b.textContent);
    expect(names).toEqual(["proj-a", "proj-b"]);

    // Clicking "Failed" must sort by failed tasks, not by completed.
    await user.click(screen.getByRole("button", { name: "Sort by Failed" }));
    names = screen
      .getAllByRole("button", { name: /^proj-/ })
      .map((b) => b.textContent);
    expect(names).toEqual(["proj-b", "proj-a"]);
  });

  it("lists a failed task even when many newer tasks exist", async () => {
    const user = userEvent.setup();
    const nowSecs = Math.floor(Date.now() / 1000);
    const tasks = Array.from({ length: 11 }, (_, i) =>
      makeTask({
        id: `ok-${i}`,
        title: `ok ${i}`,
        status: "succeeded",
        created_at: nowSecs - i,
        finished_at: nowSecs - i,
      }),
    );
    tasks.push(
      makeTask({
        id: "bad",
        title: "Broken task",
        status: "failed",
        created_at: nowSecs - 1000,
        finished_at: nowSecs - 1000,
      }),
    );
    render(
      <AnalyticsPage
        snapshot={makeSnapshot({ projects: [makeProject()], tasks })}
        initialProject={null}
        onOpenTask={vi.fn()}
        onOpenProject={vi.fn()}
      />,
    );
    await user.click(
      await screen.findByRole("tab", { name: /reliability/i }),
    );
    expect(await screen.findByText("Broken task")).toBeInTheDocument();
  });

  it("shows categorized reliability metrics and lets a group be inspected", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(await screen.findByRole("tab", { name: /reliability/i }));
    expect(await screen.findByText("Task failure rate")).toBeInTheDocument();
    expect(screen.getByText("Recovery rate")).toBeInTheDocument();
    // The group label is built from category + code, never raw error text.
    const group = await screen.findByRole("button", {
      name: /Inspect Build · TS2322/,
    });
    await user.click(group);
    expect(await screen.findByText("Affected tasks")).toBeInTheDocument();
  });

  it("shows interaction categories, human wait, and advisory recommendations", async () => {
    const user = userEvent.setup();
    renderPage();
    await user.click(await screen.findByRole("tab", { name: /interactions/i }));
    expect(await screen.findByText("Human wait")).toBeInTheDocument();
    expect(screen.getByText("Autonomy")).toBeInTheDocument();
    expect(
      screen.getAllByText("Environment setup").length,
    ).toBeGreaterThan(0);
    expect(
      screen.getByText("Describe the dev environment"),
    ).toBeInTheDocument();
    // Recurring-question grouping is structural.
    expect(screen.getByText("question:environment")).toBeInTheDocument();
  });
});
