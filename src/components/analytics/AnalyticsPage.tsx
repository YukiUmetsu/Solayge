import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import type {
  AnalyticsCollectionStatus,
  AnalyticsFilters,
  AnalyticsModelMetric,
  AnalyticsOverview,
  AnalyticsProjectMetric,
  AnalyticsTaskDetail,
  AnalyticsTimeSeriesPoint,
  ErrorGroup,
  InteractionGroup,
  InteractionResult,
  ReliabilityResult,
  Snapshot,
} from "../../types";
import {
  coverageLabel,
  errorCategoryLabel,
  errorGroupLabel,
  filtersActive,
  formatDay,
  formatMillis,
  formatPercent,
  formatTokens,
  formatUsd,
  interactionKindLabel,
  modelLabel,
  questionCategoryLabel,
  rangeForPreset,
  relativeDay,
  type RangePreset,
} from "../../lib/analytics";
import { displayStatus, statusMeta } from "../../lib/tasks";
import { compactDuration, shortId } from "../../lib/format";
import { Icon } from "../Icons";
import { TrendChart } from "./TrendChart";

type Tab = "overview" | "projects" | "reliability" | "interactions";

const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "projects", label: "Projects & Models" },
  { id: "reliability", label: "Reliability" },
  { id: "interactions", label: "Interactions" },
];

const RANGES: { value: RangePreset; label: string }[] = [
  { value: 7, label: "Last 7 days" },
  { value: 30, label: "Last 30 days" },
  { value: 90, label: "Last 90 days" },
  { value: 0, label: "All time" },
];

function withinWindow(filters: AnalyticsFilters, ms: number): boolean {
  return (
    (filters.from_ms === null || ms >= filters.from_ms) &&
    (filters.to_ms === null || ms < filters.to_ms)
  );
}

function formatCount(n: number): string {
  return n.toLocaleString();
}

export function AnalyticsPage({
  snapshot,
  initialProject,
  onOpenTask,
  onOpenProject,
}: {
  snapshot: Snapshot;
  initialProject: string | null;
  onOpenTask: (projectPath: string, taskId: string) => void;
  onOpenProject: (projectPath: string) => void;
}) {
  const [tab, setTab] = useState<Tab>("overview");
  const [preset, setPreset] = useState<RangePreset>(30);
  const [project, setProject] = useState<string>(initialProject ?? "");
  const [model, setModel] = useState<string>("");
  const [usageMetric, setUsageMetric] = useState<"tokens" | "cost">("tokens");
  const [overview, setOverview] = useState<AnalyticsOverview | null>(null);
  const [series, setSeries] = useState<AnalyticsTimeSeriesPoint[]>([]);
  const [projectsData, setProjectsData] = useState<AnalyticsProjectMetric[]>([]);
  const [modelsData, setModelsData] = useState<AnalyticsModelMetric[]>([]);
  const [reliability, setReliability] = useState<ReliabilityResult | null>(null);
  const [interactions, setInteractions] = useState<InteractionResult | null>(null);
  // Unfiltered-by-model list, used only to populate the model dropdown so
  // selecting one does not collapse the remaining choices.
  const [modelChoices, setModelChoices] = useState<AnalyticsModelMetric[]>([]);
  const [status, setStatus] = useState<AnalyticsCollectionStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reloadToken, setReloadToken] = useState(0);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  // A project shortcut opens analytics prefiltered to that project; the global
  // Analytics nav passes `null`, which must reset back to all projects rather
  // than leaving a previous project selection in place.
  useEffect(() => {
    setProject(initialProject ?? "");
  }, [initialProject]);

  const filters = useMemo<AnalyticsFilters>(() => {
    const { from_ms, to_ms } = rangeForPreset(preset);
    return {
      from_ms,
      to_ms,
      projects: project ? [project] : [],
      models: model ? [model] : [],
    };
  }, [preset, project, model]);

  const load = useCallback(() => {
    let alive = true;
    setLoading(true);
    Promise.all([
      api.analyticsOverview(filters),
      api.analyticsTimeseries(filters),
      api.analyticsProjects(filters),
      api.analyticsModels(filters),
      api.analyticsModels({ ...filters, models: [] }),
      api.analyticsReliability(filters),
      api.analyticsInteractions(filters),
    ])
      .then(([o, t, p, m, choices, rel, inter]) => {
        if (!alive) return;
        setOverview(o);
        setSeries(t);
        setProjectsData(p);
        setModelsData(m);
        setModelChoices(choices);
        setReliability(rel);
        setInteractions(inter);
        setError(null);
      })
      .catch((e) => alive && setError(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
    // `reloadToken` is intentionally a dependency so manual refresh refetches.
  }, [filters, reloadToken]);

  useEffect(() => load(), [load]);

  useEffect(() => {
    api
      .analyticsCollectionStatus()
      .then(setStatus)
      .catch(() => {});
  }, [reloadToken]);

  // Quietly keep the open page fresh while tasks run.
  useEffect(() => {
    const t = window.setInterval(() => setReloadToken((n) => n + 1), 15_000);
    return () => window.clearInterval(t);
  }, []);

  const projectNames = useMemo(() => {
    const m = new Map<string, string>();
    for (const p of snapshot.projects) m.set(p.path, p.name);
    return m;
  }, [snapshot.projects]);

  // Task id -> project path, so drill-downs can open the task in its workspace.
  const taskProject = useMemo(() => {
    const m = new Map<string, string>();
    for (const t of snapshot.tasks) m.set(t.id, t.project_path);
    for (const d of snapshot.deleted_tasks ?? []) m.set(d.task.id, d.task.project_path);
    return m;
  }, [snapshot.tasks, snapshot.deleted_tasks]);

  // Tasks visible in the current window, for the drill-down list.
  const recentTasks = useMemo(() => {
    const rows = snapshot.tasks.filter((t) => {
      if (project && t.project_path !== project) return false;
      if (t.status === "draft") return false;
      const at = (t.finished_at ?? t.started_at ?? t.created_at) * 1000;
      return withinWindow(filters, at);
    });
    rows.sort(
      (a, b) =>
        (b.finished_at ?? b.started_at ?? b.created_at) -
        (a.finished_at ?? a.started_at ?? a.created_at),
    );
    return rows.slice(0, 10);
  }, [snapshot.tasks, project, filters]);

  // Tasks needing attention across the whole window, not just the recent slice,
  // so the list never contradicts the failure KPI above it.
  const problems = useMemo(() => {
    const rows = snapshot.tasks.filter((t) => {
      if (project && t.project_path !== project) return false;
      if (!["failed", "interrupted", "blocked"].includes(t.status)) return false;
      const at = (t.finished_at ?? t.started_at ?? t.created_at) * 1000;
      return withinWindow(filters, at);
    });
    rows.sort(
      (a, b) =>
        (b.finished_at ?? b.started_at ?? b.created_at) -
        (a.finished_at ?? a.started_at ?? a.created_at),
    );
    return rows.slice(0, 100);
  }, [snapshot.tasks, project, filters]);

  const resetFilters = () => {
    setPreset(30);
    setProject("");
    setModel("");
  };

  async function exportData(format: "csv" | "json") {
    setExporting(true);
    try {
      const path = await save({
        defaultPath: `solayge-analytics.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (typeof path !== "string") return;
      await api.exportAnalytics(filters, format, path);
    } catch (e) {
      setError(String(e));
    } finally {
      setExporting(false);
    }
  }

  const modelOptions = useMemo(() => {
    // One option per provider/model; a variant label is informational, the
    // value the backend filters on is `provider/model`.
    const seen = new Map<string, string>();
    for (const m of modelChoices) {
      const value = m.provider_id ? `${m.provider_id}/${m.model_id}` : m.model_id;
      if (!seen.has(value)) seen.set(value, modelLabel(m));
    }
    return Array.from(seen, ([value, label]) => ({ value, label }));
  }, [modelChoices]);

  const activeFiltered = filtersActive(filters);

  return (
    <div className="scroll min-h-0 flex-1">
      <div className="mx-auto w-full max-w-[1500px] px-6 py-5">
        {/* Title + top actions */}
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold text-ink">Analytics</h1>
            <p className="mt-0.5 text-[12px] text-ink-subtle">
              Local-only task, model, token, and cost history across projects.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              className="btn btn-ghost"
              onClick={() => setReloadToken((n) => n + 1)}
              disabled={loading}
            >
              <Icon name="refresh" className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
              Refresh
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => void exportData("csv")}
              disabled={exporting || !overview?.has_data}
              title="Export the current filters as CSV"
            >
              Export CSV
            </button>
            <button
              className="btn btn-ghost"
              onClick={() => void exportData("json")}
              disabled={exporting || !overview?.has_data}
              title="Export the current filters as JSON"
            >
              Export JSON
            </button>
          </div>
        </div>

        {/* Tabs */}
        <div
          role="tablist"
          aria-label="Analytics sections"
          className="scroll-x mb-3 flex gap-1 border-b border-line"
        >
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              onClick={() => setTab(t.id)}
              className={`-mb-px whitespace-nowrap border-b-2 px-3 py-2 text-[12.5px] font-medium transition ${
                tab === t.id
                  ? "border-accent text-accent-text"
                  : "border-transparent text-ink-muted hover:text-ink"
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>

        {/* Filter bar */}
        <div className="mb-5 flex flex-wrap items-center gap-2">
          <select
            className="select !w-auto"
            value={preset}
            onChange={(e) => setPreset(Number(e.target.value) as RangePreset)}
            aria-label="Date range"
          >
            {RANGES.map((r) => (
              <option key={r.value} value={r.value}>
                {r.label}
              </option>
            ))}
          </select>
          <select
            className="select !w-auto max-w-[240px]"
            value={project}
            onChange={(e) => setProject(e.target.value)}
            aria-label="Project filter"
          >
            <option value="">All projects</option>
            {snapshot.projects.map((p) => (
              <option key={p.path} value={p.path}>
                {p.name}
              </option>
            ))}
          </select>
          <select
            className="select !w-auto max-w-[260px]"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            aria-label="Model filter"
          >
            <option value="">All models</option>
            {modelOptions.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          {(activeFiltered || preset !== 30) && (
            <button className="btn btn-ghost" onClick={resetFilters}>
              Reset filters
            </button>
          )}
        </div>

        {error && (
          <div className="card mb-4 flex items-center justify-between gap-3 rounded-xl border-danger-line p-3 text-[12px] text-danger">
            <span className="min-w-0 truncate">Analytics could not load: {error}</span>
            <button className="btn btn-ghost" onClick={() => setReloadToken((n) => n + 1)}>
              Retry
            </button>
          </div>
        )}

        {overview === null && loading ? (
          <LoadingSkeleton />
        ) : overview && !overview.has_data ? (
          <EmptyState
            title="No task data yet"
            body="Run a task and analytics will start collecting lifecycle, token, and cost data locally. Nothing is sent anywhere."
          />
        ) : overview ? (
          <>
            {tab === "overview" && (
              <OverviewTab
                overview={overview}
                series={series}
                projects={projectsData}
                usageMetric={usageMetric}
                setUsageMetric={setUsageMetric}
                activeFiltered={activeFiltered}
                onResetFilters={resetFilters}
                onShowProjects={() => setTab("projects")}
                projectNames={projectNames}
                recentTasks={recentTasks}
                onOpenTask={onOpenTask}
                onSelectTask={setSelectedTaskId}
                onFilterProject={setProject}
                onOpenProject={onOpenProject}
              />
            )}
            {tab === "projects" && (
              <ProjectsTab
                projects={projectsData}
                models={modelsData}
                onFilterProject={setProject}
                onOpenProject={onOpenProject}
              />
            )}
            {tab === "reliability" &&
              (reliability ? (
                <ReliabilityTab
                  reliability={reliability}
                  problems={problems}
                  projectNames={projectNames}
                  taskProject={taskProject}
                  onSelectTask={setSelectedTaskId}
                  onOpenTask={onOpenTask}
                />
              ) : (
                <LoadingSkeleton />
              ))}
            {tab === "interactions" &&
              (interactions ? (
                <InteractionsTab
                  interactions={interactions}
                  taskProject={taskProject}
                  onSelectTask={setSelectedTaskId}
                  onOpenTask={onOpenTask}
                />
              ) : (
                <LoadingSkeleton />
              ))}
          </>
        ) : null}

        {status && (
          <footer className="mt-8 border-t border-line pt-3 text-[11px] text-ink-subtle">
            {status.enabled
              ? "Collecting local analytics"
              : "Analytics collection is off (Settings → Advanced)"}
            {" · "}
            {formatCount(status.usage_count)} usage records,{" "}
            {formatCount(status.run_count)} task runs
            {status.retention_days > 0
              ? ` · kept ${status.retention_days} days`
              : " · kept forever"}
            .
          </footer>
        )}
      </div>

      {selectedTaskId && (
        <TaskDrawer
          taskId={selectedTaskId}
          projectName={
            projectNames.get(
              snapshot.tasks.find((t) => t.id === selectedTaskId)?.project_path ?? "",
            ) ?? ""
          }
          onClose={() => setSelectedTaskId(null)}
          onOpenTask={onOpenTask}
        />
      )}
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
      {Array.from({ length: 6 }).map((_, i) => (
        <div key={i} className="card h-[86px] animate-pulse rounded-xl" />
      ))}
    </div>
  );
}

function EmptyState({ title, body }: { title: string; body: string }) {
  return (
    <div className="card flex flex-col items-center gap-2 rounded-xl px-6 py-14 text-center">
      <Icon name="chart" className="h-8 w-8 text-ink-faint" />
      <h2 className="text-[15px] font-semibold text-ink">{title}</h2>
      <p className="max-w-md text-[12px] leading-relaxed text-ink-subtle">{body}</p>
    </div>
  );
}

function Kpi({
  label,
  value,
  hint,
  tone = "text-ink",
}: {
  label: string;
  value: string;
  hint?: string;
  tone?: string;
}) {
  return (
    <div className="card rounded-xl px-3.5 py-3">
      <div className="text-[11px] uppercase tracking-wide text-ink-subtle">
        {label}
      </div>
      <div className={`mono mt-1 text-[22px] font-semibold leading-tight ${tone}`}>
        {value}
      </div>
      {hint && <div className="mt-0.5 text-[11px] text-ink-subtle">{hint}</div>}
    </div>
  );
}

function OverviewTab({
  overview,
  series,
  projects,
  usageMetric,
  setUsageMetric,
  activeFiltered,
  onResetFilters,
  onShowProjects,
  projectNames,
  recentTasks,
  onOpenTask,
  onSelectTask,
  onFilterProject,
  onOpenProject,
}: {
  overview: AnalyticsOverview;
  series: AnalyticsTimeSeriesPoint[];
  projects: AnalyticsProjectMetric[];
  usageMetric: "tokens" | "cost";
  setUsageMetric: (m: "tokens" | "cost") => void;
  activeFiltered: boolean;
  onResetFilters: () => void;
  onShowProjects: () => void;
  projectNames: Map<string, string>;
  recentTasks: Snapshot["tasks"];
  onOpenTask: (projectPath: string, taskId: string) => void;
  onSelectTask: (id: string) => void;
  onFilterProject: (path: string) => void;
  onOpenProject: (path: string) => void;
}) {
  const coverage = coverageLabel(overview);
  const usageUnavailable = overview.inference_count === 0;

  const activityPoints = series.map((p) => ({
    label: formatDay(p.bucket_ms),
    values: [p.tasks_completed, p.tasks_failed],
  }));
  const usagePoints = series.map((p) => ({
    label: formatDay(p.bucket_ms),
    values: [usageMetric === "tokens" ? p.total_tokens : p.reported_cost_micros],
  }));

  const topProjects = projects.slice(0, 5);

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
        <Kpi
          label="Tasks completed"
          value={formatCount(overview.tasks_completed)}
          hint={`${formatCount(overview.tasks_total)} ran in range`}
          tone="text-success"
        />
        <Kpi
          label="Success rate"
          value={formatPercent(overview.success_rate)}
          hint={`${formatCount(overview.success_sample)} terminal (excl. canceled)`}
        />
        <Kpi
          label="Avg. active time"
          value={formatMillis(overview.avg_active_ms)}
          hint={`median ${formatMillis(overview.median_active_ms)} · ${formatCount(overview.duration_sample)} runs`}
        />
        <Kpi
          label="Tokens used"
          value={formatTokens(overview.total_tokens)}
          hint={coverage ?? "no usage reported"}
        />
        <Kpi
          label="Reported usage cost"
          value={formatUsd(overview.reported_cost_micros)}
          hint="provider-calculated · not a bill"
        />
        <Kpi
          label="Autonomy"
          value={formatPercent(overview.autonomy_rate)}
          hint={
            overview.autonomy_sample > 0
              ? `${formatCount(overview.autonomy_sample)} agent runs observed`
              : usageUnavailable
                ? "no usage data"
                : "no agent runs in range"
          }
        />
      </div>

      {usageUnavailable && (
        <p className="rounded-lg border border-line bg-well px-3 py-2 text-[12px] text-ink-muted">
          Token and cost data is unavailable for these runs. Only opencode agent
          tasks report usage today; shell, git, merge, review, and other providers
          do not.
        </p>
      )}

      <div className="grid gap-4 xl:grid-cols-2">
        <TrendChart
          title="Task activity (daily)"
          series={[
            { label: "Completed", color: "var(--color-success)" },
            { label: "Failed", color: "var(--color-danger)" },
          ]}
          points={activityPoints}
          formatValue={(n) => (n >= 1000 ? formatTokens(n) : String(Math.round(n)))}
        />
        <section className="card rounded-xl p-4">
          <header className="mb-2 flex items-center justify-between gap-3">
            <h3 className="text-[13px] font-semibold text-ink">Usage trend (daily)</h3>
            <div className="flex rounded-md border border-line p-0.5">
              {(["tokens", "cost"] as const).map((m) => (
                <button
                  key={m}
                  onClick={() => setUsageMetric(m)}
                  className={`rounded px-2 py-0.5 text-[11px] font-medium capitalize transition ${
                    usageMetric === m
                      ? "bg-accent-soft text-accent-text"
                      : "text-ink-muted hover:text-ink"
                  }`}
                >
                  {m}
                </button>
              ))}
            </div>
          </header>
          <TrendChart
            title={usageMetric === "tokens" ? "Tokens (daily)" : "Reported cost (daily)"}
            series={[
              {
                label: usageMetric === "tokens" ? "Tokens" : "Cost (USD)",
                color: "var(--color-accent)",
              },
            ]}
            points={usagePoints}
            formatValue={(n) =>
              usageMetric === "tokens" ? formatTokens(n) : formatUsd(n).replace(/^~/, "")
            }
            valueSuffix={usageMetric === "cost" ? "" : ""}
            emptyHint="No usage reported in this range."
          />
        </section>
      </div>

      <section className="card rounded-xl p-4">
        <header className="mb-3 flex items-center justify-between">
          <h3 className="text-[13px] font-semibold text-ink">Top projects</h3>
          <button className="text-[12px] text-accent-text hover:underline" onClick={onShowProjects}>
            View all projects
          </button>
        </header>
        {topProjects.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            {activeFiltered ? (
              <>
                No activity matches your filters.{" "}
                <button className="text-accent-text hover:underline" onClick={onResetFilters}>
                  Reset filters
                </button>
              </>
            ) : (
              "No project activity in this range."
            )}
          </p>
        ) : (
          <MiniProjectTable
            rows={topProjects}
            onFilterProject={onFilterProject}
            onOpenProject={onOpenProject}
          />
        )}
      </section>

      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">Recent tasks</h3>
        {recentTasks.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No tasks in this range.
          </p>
        ) : (
          <ul className="divide-y divide-line">
            {recentTasks.map((t) => {
              const meta = statusMeta(displayStatus(t));
              return (
                <li key={t.id} className="flex items-center gap-3 py-2">
                  <span className={`h-2 w-2 shrink-0 rounded-full ${meta.dot}`} aria-hidden="true" />
                  <button
                    className="min-w-0 flex-1 truncate text-left text-[12.5px] text-ink hover:text-accent-text"
                    onClick={() => onSelectTask(t.id)}
                    title="Open analytics for this task"
                  >
                    {t.title}
                  </button>
                  <span className="hidden shrink-0 text-[11px] text-ink-subtle sm:inline">
                    {projectNames.get(t.project_path) ?? t.project_path}
                  </span>
                  <span className={`shrink-0 text-[11px] ${meta.text}`}>{meta.label}</span>
                  <button
                    className="shrink-0 text-[11px] text-accent-text hover:underline"
                    onClick={() => onOpenTask(t.project_path, t.id)}
                  >
                    Open
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>
    </div>
  );
}

function MiniProjectTable({
  rows,
  onFilterProject,
  onOpenProject,
}: {
  rows: AnalyticsProjectMetric[];
  onFilterProject: (path: string) => void;
  onOpenProject: (path: string) => void;
}) {
  return (
    <div className="scroll-x">
      <table className="w-full min-w-[620px] text-[12px]">
        <thead>
          <tr className="border-b border-line text-left text-[11px] uppercase tracking-wide text-ink-subtle">
            <th className="py-2 pr-3 font-medium">Project</th>
            <th className="py-2 pr-3 text-right font-medium">Tasks</th>
            <th className="py-2 pr-3 text-right font-medium">Success</th>
            <th className="py-2 pr-3 text-right font-medium">Avg time</th>
            <th className="py-2 pr-3 text-right font-medium">Tokens</th>
            <th className="py-2 pr-3 text-right font-medium">Reported cost</th>
            <th className="py-2" />
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.project_path} className="border-b border-line/60">
              <td className="py-2 pr-3">
                <button
                  className="truncate text-left text-ink hover:text-accent-text"
                  title={r.project_path}
                  onClick={() => onFilterProject(r.project_path)}
                >
                  {r.project_name}
                </button>
              </td>
              <td className="mono py-2 pr-3 text-right text-ink-muted">
                {formatCount(r.tasks_completed)}
                {r.tasks_failed > 0 && (
                  <span className="text-danger"> / {formatCount(r.tasks_failed)}</span>
                )}
              </td>
              <td className="mono py-2 pr-3 text-right text-ink-muted">
                {formatPercent(r.success_rate)}
              </td>
              <td className="mono py-2 pr-3 text-right text-ink-muted">
                {formatMillis(r.avg_active_ms)}
              </td>
              <td className="mono py-2 pr-3 text-right text-ink-muted">
                {formatTokens(r.total_tokens)}
              </td>
              <td className="mono py-2 pr-3 text-right text-ink-muted">
                {formatUsd(r.reported_cost_micros)}
              </td>
              <td className="py-2 text-right">
                <button
                  className="text-[11px] text-accent-text hover:underline"
                  onClick={() => onOpenProject(r.project_path)}
                >
                  Open
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

type ProjectSortKey =
  | "project_name"
  | "tasks_completed"
  | "tasks_failed"
  | "success_rate"
  | "avg_active_ms"
  | "median_active_ms"
  | "total_tokens"
  | "reported_cost_micros";

function ProjectsTab({
  projects,
  models,
  onFilterProject,
  onOpenProject,
}: {
  projects: AnalyticsProjectMetric[];
  models: AnalyticsModelMetric[];
  onFilterProject: (path: string) => void;
  onOpenProject: (path: string) => void;
}) {
  const [sortKey, setSortKey] = useState<ProjectSortKey>("tasks_completed");
  const [dir, setDir] = useState<"asc" | "desc">("desc");

  const sorted = useMemo(() => {
    const rows = [...projects];
    const cmp = (a: AnalyticsProjectMetric, b: AnalyticsProjectMetric) => {
      const av = a[sortKey];
      const bv = b[sortKey];
      // Unknowns always sort last, regardless of direction.
      if (av === null && bv === null) return 0;
      if (av === null) return 1;
      if (bv === null) return -1;
      const base =
        typeof av === "string" && typeof bv === "string"
          ? av.localeCompare(bv)
          : Number(av) - Number(bv);
      return dir === "asc" ? base : -base;
    };
    return rows.sort(cmp);
  }, [projects, sortKey, dir]);

  function toggle(key: ProjectSortKey) {
    if (key === sortKey) {
      setDir(dir === "asc" ? "desc" : "asc");
    } else {
      setSortKey(key);
      setDir(key === "project_name" ? "asc" : "desc");
    }
  }

  const arrow = (key: ProjectSortKey) =>
    key === sortKey ? (dir === "asc" ? " ▲" : " ▼") : "";

  return (
    <div className="space-y-5">
      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">Project comparison</h3>
        {projects.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No project activity in this range.
          </p>
        ) : (
          <div className="scroll-x">
            <table className="w-full min-w-[760px] text-[12px]">
              <thead>
                <tr className="border-b border-line text-left text-[11px] uppercase tracking-wide text-ink-subtle">
                  <SortTh label="Project" onClick={() => toggle("project_name")} suffix={arrow("project_name")} />
                  <SortTh label="Completed" align="right" onClick={() => toggle("tasks_completed")} suffix={arrow("tasks_completed")} />
                  <SortTh label="Failed" align="right" onClick={() => toggle("tasks_failed")} suffix={arrow("tasks_failed")} />
                  <SortTh label="Success" align="right" onClick={() => toggle("success_rate")} suffix={arrow("success_rate")} />
                  <SortTh label="Avg time" align="right" onClick={() => toggle("avg_active_ms")} suffix={arrow("avg_active_ms")} />
                  <SortTh label="Median" align="right" onClick={() => toggle("median_active_ms")} suffix={arrow("median_active_ms")} />
                  <SortTh label="Tokens" align="right" onClick={() => toggle("total_tokens")} suffix={arrow("total_tokens")} />
                  <SortTh label="Reported cost" align="right" onClick={() => toggle("reported_cost_micros")} suffix={arrow("reported_cost_micros")} />
                  <th className="py-2" />
                </tr>
              </thead>
              <tbody>
                {sorted.map((r) => (
                  <tr key={r.project_path} className="border-b border-line/60">
                    <td className="py-2 pr-3">
                      <button
                        className="truncate text-left text-ink hover:text-accent-text"
                        title={r.project_path}
                        onClick={() => onFilterProject(r.project_path)}
                      >
                        {r.project_name}
                      </button>
                      <div className="truncate text-[10.5px] text-ink-subtle">
                        {r.tasks_total} ran · {r.inference_count} model calls
                      </div>
                    </td>
                    <td className="mono py-2 pr-3 text-right text-success">
                      {formatCount(r.tasks_completed)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-danger">
                      {formatCount(r.tasks_failed)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatPercent(r.success_rate)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatMillis(r.avg_active_ms)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatMillis(r.median_active_ms)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatTokens(r.total_tokens)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatUsd(r.reported_cost_micros)}
                    </td>
                    <td className="py-2 text-right">
                      <button
                        className="text-[11px] text-accent-text hover:underline"
                        onClick={() => onOpenProject(r.project_path)}
                      >
                        Open
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">Models</h3>
        {models.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No model usage reported in this range.
          </p>
        ) : (
          <div className="scroll-x">
            <table className="w-full min-w-[720px] text-[12px]">
              <thead>
                <tr className="border-b border-line text-left text-[11px] uppercase tracking-wide text-ink-subtle">
                  <th className="py-2 pr-3 font-medium">Provider / model</th>
                  <th className="py-2 pr-3 text-right font-medium">Model calls</th>
                  <th className="py-2 pr-3 text-right font-medium">Task runs</th>
                  <th className="py-2 pr-3 text-right font-medium">Input</th>
                  <th className="py-2 pr-3 text-right font-medium">Output</th>
                  <th className="py-2 pr-3 text-right font-medium">Cache read</th>
                  <th className="py-2 pr-3 text-right font-medium">Tokens</th>
                  <th className="py-2 pr-3 text-right font-medium">Success (excl.)</th>
                  <th className="py-2 pr-3 text-right font-medium">Mixed</th>
                  <th className="py-2 text-right font-medium">Reported cost</th>
                </tr>
              </thead>
              <tbody>
                {models.map((m, i) => (
                  <tr
                    key={`${m.provider_id}/${m.model_id}/${m.variant ?? ""}/${i}`}
                    className="border-b border-line/60"
                  >
                    <td className="py-2 pr-3 text-ink">{modelLabel(m)}</td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatCount(m.inference_count)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatCount(m.task_runs)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatTokens(m.input_tokens)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatTokens(m.output_tokens)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatTokens(m.cache_read_tokens)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatTokens(m.total_tokens)}
                    </td>
                    <td
                      className="mono py-2 pr-3 text-right text-ink-muted"
                      title={`${m.exclusive_success} of ${m.exclusive_task_runs} exclusively-${m.model_id} tasks succeeded`}
                    >
                      {m.exclusive_task_runs > 0
                        ? `${formatPercent(m.exclusive_success_rate)} (${m.exclusive_task_runs})`
                        : "—"}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {m.mixed_task_runs > 0 ? formatCount(m.mixed_task_runs) : "—"}
                    </td>
                    <td className="mono py-2 text-right text-ink-muted">
                      {formatUsd(m.reported_cost_micros)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        <p className="mt-3 text-[11px] leading-relaxed text-ink-subtle">
          “Reported cost” is the provider’s own price calculation for the tokens
          used — it is not a charge on your card, and subscription plans are not
          billed this way.
        </p>
      </section>
    </div>
  );
}

function SortTh({
  label,
  align = "left",
  onClick,
  suffix,
}: {
  label: string;
  align?: "left" | "right";
  onClick: () => void;
  suffix: string;
}) {
  return (
    <th
      className={`py-2 pr-3 font-medium ${align === "right" ? "text-right" : "text-left"}`}
    >
      <button
        className="uppercase tracking-wide hover:text-ink"
        onClick={onClick}
        aria-label={`Sort by ${label}`}
      >
        {label}
        {suffix}
      </button>
    </th>
  );
}

function ReliabilityTab({
  reliability,
  problems,
  projectNames,
  taskProject,
  onSelectTask,
  onOpenTask,
}: {
  reliability: ReliabilityResult;
  problems: Snapshot["tasks"];
  projectNames: Map<string, string>;
  taskProject: Map<string, string>;
  onSelectTask: (id: string) => void;
  onOpenTask: (projectPath: string, taskId: string) => void;
}) {
  const { summary } = reliability;
  const [category, setCategory] = useState("");
  const [stage, setStage] = useState("");
  const [tool, setTool] = useState("");
  const [repeatedOnly, setRepeatedOnly] = useState(false);
  const [showAll, setShowAll] = useState(false);
  const [selected, setSelected] = useState<ErrorGroup | null>(null);

  const groups = useMemo(
    () =>
      reliability.groups.filter(
        (g) =>
          (!category || g.category === category) &&
          (!stage || g.stage === stage) &&
          (!tool || g.tool_name === tool) &&
          (!repeatedOnly || g.occurrences > 1),
      ),
    [reliability.groups, category, stage, tool, repeatedOnly],
  );
  const visible = showAll ? groups : groups.slice(0, 25);
  const barMax = Math.max(1, ...reliability.categories.map((c) => c.occurrences));

  const seriesPoints = reliability.series.map((p) => ({
    label: formatDay(p.bucket_ms),
    values: [p.errors, p.tasks_failed],
  }));

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-5">
        <Kpi
          label="Task failure rate"
          value={formatPercent(summary.task_failure_rate)}
          hint={`${formatCount(summary.failed_sample)} terminal (excl. canceled)`}
          tone="text-danger"
        />
        <Kpi
          label="Runs with errors"
          value={formatPercent(summary.runs_with_errors_rate)}
          hint={`${formatCount(summary.runs_with_errors)} of ${formatCount(summary.observed_runs)} runs`}
        />
        <Kpi
          label="Recovery rate"
          value={formatPercent(summary.recovery_rate)}
          hint={`${formatCount(summary.recovered_errors)} of ${formatCount(summary.recoverable_errors)} errors`}
        />
        <Kpi
          label="Retry rate"
          value={formatPercent(summary.retry_rate)}
          hint={`${formatCount(summary.retry_tasks)} of ${formatCount(summary.attempt_tasks)} tasks`}
        />
        <Kpi
          label="First-pass success"
          value={formatPercent(summary.first_pass_success_rate)}
          hint={`${formatCount(summary.first_pass_success)} of ${formatCount(summary.first_pass_sample)} terminal`}
        />
      </div>

      <TrendChart
        title="Error occurrences vs task failures (daily)"
        series={[
          { label: "Error events", color: "var(--color-warning)" },
          { label: "Failed tasks", color: "var(--color-danger)" },
        ]}
        points={seriesPoints}
        formatValue={(n) => String(Math.round(n))}
        emptyHint="No errors or failures in this range."
      />

      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">
          Error categories by occurrence
        </h3>
        {reliability.categories.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No categorized errors in this range.
          </p>
        ) : (
          <ul className="space-y-2">
            {reliability.categories.map((c) => (
              <li key={c.category} className="flex items-center gap-3 text-[12px]">
                <span className="w-32 shrink-0 truncate text-ink-muted" title={c.category}>
                  {errorCategoryLabel(c.category)}
                </span>
                <div className="h-2 flex-1 overflow-hidden rounded-full bg-well">
                  <div
                    className="h-full rounded-full bg-warning/70"
                    style={{ width: `${Math.max(2, (c.occurrences / barMax) * 100)}%` }}
                  />
                </div>
                <span className="mono w-10 shrink-0 text-right text-ink">
                  {formatCount(c.occurrences)}
                </span>
                <span className="hidden w-24 shrink-0 text-[11px] text-ink-subtle sm:inline">
                  {formatCount(c.tasks)} tasks ·{" "}
                  {c.recovery_rate === null ? "—" : formatPercent(c.recovery_rate)}
                </span>
              </li>
            ))}
          </ul>
        )}
        <p className="mt-3 text-[11px] leading-relaxed text-ink-subtle">
          An error occurrence is not a failed task: a run can record errors and
          still succeed. “Recovered” means the task's final outcome succeeded.
        </p>
      </section>

      <section className="card rounded-xl p-4">
        <header className="mb-3 flex flex-wrap items-center justify-between gap-2">
          <h3 className="text-[13px] font-semibold text-ink">Recurring errors</h3>
          <div className="flex flex-wrap items-center gap-2">
            <select
              className="select !w-auto"
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              aria-label="Filter by error category"
            >
              <option value="">All categories</option>
              {reliability.categories.map((c) => (
                <option key={c.category} value={c.category}>
                  {errorCategoryLabel(c.category)}
                </option>
              ))}
            </select>
            <select
              className="select !w-auto"
              value={stage}
              onChange={(e) => setStage(e.target.value)}
              aria-label="Filter by stage"
            >
              <option value="">All stages</option>
              {reliability.stages.map((s) => (
                <option key={s.name} value={s.name}>
                  {s.name}
                </option>
              ))}
            </select>
            <select
              className="select !w-auto"
              value={tool}
              onChange={(e) => setTool(e.target.value)}
              aria-label="Filter by tool"
            >
              <option value="">All tools</option>
              {reliability.tools.map((t) => (
                <option key={t.name} value={t.name}>
                  {t.name}
                </option>
              ))}
            </select>
            <label className="flex items-center gap-1.5 text-[12px] text-ink-muted">
              <input
                type="checkbox"
                checked={repeatedOnly}
                onChange={(e) => setRepeatedOnly(e.target.checked)}
              />
              Repeated only
            </label>
          </div>
        </header>
        {groups.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No errors match these filters.
          </p>
        ) : (
          <>
            <div className="scroll-x">
              <table className="w-full min-w-[720px] text-[12px]">
                <thead>
                  <tr className="border-b border-line text-left text-[11px] uppercase tracking-wide text-ink-subtle">
                    <th className="py-2 pr-3 font-medium">Fingerprint</th>
                    <th className="py-2 pr-3 font-medium">Category</th>
                    <th className="py-2 pr-3 text-right font-medium">Occurrences</th>
                    <th className="py-2 pr-3 text-right font-medium">Tasks</th>
                    <th className="py-2 pr-3 text-right font-medium">Recovered</th>
                    <th className="py-2 font-medium">Last seen</th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((g) => (
                    <tr key={g.fingerprint} className="border-b border-line/60">
                      <td className="py-2 pr-3">
                        <button
                          className="truncate text-left text-ink hover:text-accent-text"
                          title={g.fingerprint}
                          onClick={() => setSelected(g)}
                          aria-label={`Inspect ${errorGroupLabel(g)}`}
                        >
                          {errorGroupLabel(g)}
                        </button>
                        <div className="truncate text-[10.5px] text-ink-subtle">
                          {g.stage}
                          {g.tool_name ? ` · ${g.tool_name}` : ""}
                        </div>
                      </td>
                      <td className="py-2 pr-3 text-ink-muted">
                        {errorCategoryLabel(g.category)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink">
                        {formatCount(g.occurrences)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink-muted">
                        {formatCount(g.tasks)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink-muted">
                        {g.recoverable > 0
                          ? `${formatCount(g.recovered)}/${formatCount(g.recoverable)}`
                          : "—"}
                      </td>
                      <td className="py-2 text-ink-subtle">
                        {relativeDay(g.last_seen_ms)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {groups.length > visible.length && (
              <button
                className="btn btn-ghost mt-3"
                onClick={() => setShowAll(true)}
              >
                Show all {formatCount(groups.length)} groups
              </button>
            )}
          </>
        )}
      </section>

      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">
          Tasks needing attention
        </h3>
        {problems.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No failed, interrupted, or blocked tasks in this range.
          </p>
        ) : (
          <ul className="divide-y divide-line">
            {problems.map((t) => (
              <li key={t.id} className="flex items-center gap-3 py-2">
                <span className="min-w-0 flex-1 truncate text-[12.5px] text-ink">
                  {t.title}
                </span>
                <span className="hidden shrink-0 text-[11px] text-ink-subtle sm:inline">
                  {projectNames.get(t.project_path) ?? t.project_path}
                </span>
                <button
                  className="shrink-0 text-[11px] text-accent-text hover:underline"
                  onClick={() => onSelectTask(t.id)}
                >
                  Details
                </button>
                <button
                  className="shrink-0 text-[11px] text-accent-text hover:underline"
                  onClick={() => onOpenTask(t.project_path, t.id)}
                >
                  Open
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {selected && (
        <SideDrawer
          title={errorGroupLabel(selected)}
          subtitle={`${errorCategoryLabel(selected.category)} · ${selected.stage}`}
          onClose={() => setSelected(null)}
        >
          <dl className="space-y-1 text-[12px]">
            <Row label="Fingerprint" value={selected.fingerprint} />
            <Row
              label="Occurrences"
              value={`${formatCount(selected.occurrences)} across ${formatCount(selected.tasks)} tasks`}
            />
            <Row
              label="Recovered"
              value={
                selected.recoverable > 0
                  ? `${formatCount(selected.recovered)} of ${formatCount(selected.recoverable)}`
                  : "unavailable"
              }
            />
            <Row
              label="First seen"
              value={new Date(selected.first_seen_ms).toLocaleString()}
            />
            <Row
              label="Last seen"
              value={new Date(selected.last_seen_ms).toLocaleString()}
            />
          </dl>
          {selected.sample && (
            <p className="mt-3 rounded-lg border border-line bg-well px-3 py-2 text-[11.5px] text-ink-muted">
              {selected.sample}
            </p>
          )}
          <h3 className="mb-2 mt-4 text-[12px] font-semibold text-ink">
            Affected tasks
          </h3>
          {selected.task_ids.length === 0 ? (
            <p className="text-[11.5px] text-ink-subtle">
              No task attribution for this group.
            </p>
          ) : (
            <ul className="divide-y divide-line">
              {selected.task_ids.map((id) => (
                <li key={id} className="flex items-center gap-2 py-1.5">
                  <span className="mono min-w-0 flex-1 truncate text-[11.5px] text-ink-muted">
                    {shortId(id)}
                  </span>
                  <button
                    className="text-[11px] text-accent-text hover:underline"
                    onClick={() => onSelectTask(id)}
                  >
                    Details
                  </button>
                  {taskProject.get(id) && (
                    <button
                      className="text-[11px] text-accent-text hover:underline"
                      onClick={() => onOpenTask(taskProject.get(id)!, id)}
                    >
                      Open
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </SideDrawer>
      )}
    </div>
  );
}

function InteractionsTab({
  interactions,
  taskProject,
  onSelectTask,
  onOpenTask,
}: {
  interactions: InteractionResult;
  taskProject: Map<string, string>;
  onSelectTask: (id: string) => void;
  onOpenTask: (projectPath: string, taskId: string) => void;
}) {
  const { summary } = interactions;
  const [showAll, setShowAll] = useState(false);
  const [selected, setSelected] = useState<InteractionGroup | null>(null);
  const visible = showAll ? interactions.groups : interactions.groups.slice(0, 25);

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
        <Kpi label="Questions" value={formatCount(summary.questions)} />
        <Kpi label="Permissions" value={formatCount(summary.permissions)} />
        <Kpi label="Blocking prompts" value={formatCount(summary.blocking_total)} />
        <Kpi
          label="Unresolved"
          value={formatCount(summary.unresolved)}
          tone={summary.unresolved > 0 ? "text-warning" : "text-ink"}
        />
        <Kpi
          label="Human wait"
          value={formatMillis(summary.total_wait_ms)}
          hint={
            summary.median_wait_ms === null
              ? "no resolved waits"
              : `median ${formatMillis(summary.median_wait_ms)} · p95 ${formatMillis(summary.p95_wait_ms)}`
          }
        />
        <Kpi
          label="Autonomy"
          value={formatPercent(summary.autonomy_rate)}
          hint={`${formatCount(summary.autonomy_sample)} agent runs`}
        />
      </div>

      {interactions.recommendations.length > 0 && (
        <section className="card rounded-xl p-4">
          <h3 className="mb-2 text-[13px] font-semibold text-ink">
            Suggested improvements
          </h3>
          <ul className="space-y-2">
            {interactions.recommendations.map((r) => (
              <li key={r.title} className="text-[12px] leading-relaxed">
                <span className="font-medium text-ink">{r.title}</span>
                <span className="text-ink-muted"> — {r.detail}</span>
              </li>
            ))}
          </ul>
          <p className="mt-2 text-[11px] text-ink-subtle">
            Advisory only. Solayge never edits your repository or relaxes
            permissions on its own.
          </p>
        </section>
      )}

      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">
          Recurring questions and prompts
        </h3>
        {interactions.groups.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No questions or permission prompts in this range.
          </p>
        ) : (
          <>
            <div className="scroll-x">
              <table className="w-full min-w-[720px] text-[12px]">
                <thead>
                  <tr className="border-b border-line text-left text-[11px] uppercase tracking-wide text-ink-subtle">
                    <th className="py-2 pr-3 font-medium">Type</th>
                    <th className="py-2 pr-3 font-medium">Category</th>
                    <th className="py-2 pr-3 text-right font-medium">Occurrences</th>
                    <th className="py-2 pr-3 text-right font-medium">Tasks</th>
                    <th className="py-2 pr-3 text-right font-medium">Avg wait</th>
                    <th className="py-2 pr-3 text-right font-medium">Unresolved</th>
                    <th className="py-2 font-medium">Last seen</th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((g) => (
                    <tr key={g.fingerprint} className="border-b border-line/60">
                      <td className="py-2 pr-3">
                        <button
                          className="text-left text-ink hover:text-accent-text"
                          onClick={() => setSelected(g)}
                          aria-label={`Inspect ${interactionKindLabel(g.kind)}`}
                        >
                          {interactionKindLabel(g.kind)}
                        </button>
                        <div className="truncate text-[10.5px] text-ink-subtle">
                          {g.fingerprint}
                        </div>
                      </td>
                      <td className="py-2 pr-3 text-ink-muted">
                        {questionCategoryLabel(g.category)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink">
                        {formatCount(g.occurrences)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink-muted">
                        {formatCount(g.tasks)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink-muted">
                        {formatMillis(g.avg_wait_ms)}
                      </td>
                      <td className="mono py-2 pr-3 text-right text-ink-muted">
                        {formatCount(g.unresolved)}
                      </td>
                      <td className="py-2 text-ink-subtle">
                        {relativeDay(g.last_seen_ms)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {interactions.groups.length > visible.length && (
              <button className="btn btn-ghost mt-3" onClick={() => setShowAll(true)}>
                Show all {formatCount(interactions.groups.length)} groups
              </button>
            )}
          </>
        )}
        <p className="mt-3 text-[11px] leading-relaxed text-ink-subtle">
          Only structured question and permission events are counted, and only
          their category and timing — never the question or command text. Free-text
          extraction is off by default.
        </p>
      </section>

      <section className="card rounded-xl p-4">
        <h3 className="mb-3 text-[13px] font-semibold text-ink">
          Interaction categories
        </h3>
        {interactions.categories.length === 0 ? (
          <p className="py-6 text-center text-[12px] text-ink-subtle">
            No interaction categories in this range.
          </p>
        ) : (
          <div className="scroll-x">
            <table className="w-full min-w-[560px] text-[12px]">
              <thead>
                <tr className="border-b border-line text-left text-[11px] uppercase tracking-wide text-ink-subtle">
                  <th className="py-2 pr-3 font-medium">Category</th>
                  <th className="py-2 pr-3 font-medium">Kind</th>
                  <th className="py-2 pr-3 text-right font-medium">Occurrences</th>
                  <th className="py-2 pr-3 text-right font-medium">Tasks</th>
                  <th className="py-2 text-right font-medium">Unresolved</th>
                </tr>
              </thead>
              <tbody>
                {interactions.categories.map((c) => (
                  <tr key={`${c.kind}/${c.category}`} className="border-b border-line/60">
                    <td className="py-2 pr-3 text-ink">
                      {questionCategoryLabel(c.category)}
                    </td>
                    <td className="py-2 pr-3 text-ink-muted">
                      {interactionKindLabel(c.kind)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink">
                      {formatCount(c.occurrences)}
                    </td>
                    <td className="mono py-2 pr-3 text-right text-ink-muted">
                      {formatCount(c.tasks)}
                    </td>
                    <td className="mono py-2 text-right text-ink-muted">
                      {formatCount(c.unresolved)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {selected && (
        <SideDrawer
          title={interactionKindLabel(selected.kind)}
          subtitle={questionCategoryLabel(selected.category)}
          onClose={() => setSelected(null)}
        >
          <dl className="space-y-1 text-[12px]">
            <Row label="Fingerprint" value={selected.fingerprint} />
            <Row
              label="Occurrences"
              value={`${formatCount(selected.occurrences)} across ${formatCount(selected.tasks)} tasks`}
            />
            <Row label="Blocking" value={formatCount(selected.blocking)} />
            <Row
              label="Avg wait"
              value={formatMillis(selected.avg_wait_ms)}
            />
            <Row
              label="Last seen"
              value={new Date(selected.last_seen_ms).toLocaleString()}
            />
          </dl>
          <h3 className="mb-2 mt-4 text-[12px] font-semibold text-ink">
            Affected tasks
          </h3>
          <ul className="divide-y divide-line">
            {selected.task_ids.map((id) => (
              <li key={id} className="flex items-center gap-2 py-1.5">
                <span className="mono min-w-0 flex-1 truncate text-[11.5px] text-ink-muted">
                  {shortId(id)}
                </span>
                <button
                  className="text-[11px] text-accent-text hover:underline"
                  onClick={() => onSelectTask(id)}
                >
                  Details
                </button>
                {taskProject.get(id) && (
                  <button
                    className="text-[11px] text-accent-text hover:underline"
                    onClick={() => onOpenTask(taskProject.get(id)!, id)}
                  >
                    Open
                  </button>
                )}
              </li>
            ))}
          </ul>
        </SideDrawer>
      )}
    </div>
  );
}

function SideDrawer({
  title,
  subtitle,
  onClose,
  children,
}: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/30"
        onClick={onClose}
        aria-hidden="true"
      />
      <aside
        role="dialog"
        aria-label={title}
        className="panel fixed inset-y-0 right-0 z-50 flex w-full max-w-[460px] flex-col rounded-none border-y-0 border-r-0 shadow-2xl"
      >
        <header className="titlebar-pad flex items-start justify-between gap-3 border-b border-line px-4 pb-3">
          <div className="min-w-0">
            {subtitle && <div className="text-[11px] text-ink-subtle">{subtitle}</div>}
            <h2 className="truncate text-[14px] font-semibold text-ink">{title}</h2>
          </div>
          <button
            className="rounded-lg border border-line p-1.5 text-ink-muted hover:text-ink"
            onClick={onClose}
            aria-label="Close"
          >
            <Icon name="x" className="h-3.5 w-3.5" />
          </button>
        </header>
        <div className="scroll flex-1 px-4 py-4">{children}</div>
      </aside>
    </>
  );
}

function TaskDrawer({
  taskId,
  projectName,
  onClose,
  onOpenTask,
}: {
  taskId: string;
  projectName: string;
  onClose: () => void;
  onOpenTask: (projectPath: string, taskId: string) => void;
}) {
  const [detail, setDetail] = useState<AnalyticsTaskDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setDetail(null);
    setError(null);
    api
      .analyticsTaskDetail(taskId)
      .then((d) => alive && setDetail(d))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [taskId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/30"
        onClick={onClose}
        aria-hidden="true"
      />
      <aside
        role="dialog"
        aria-label="Task analytics"
        className="panel fixed inset-y-0 right-0 z-50 flex w-full max-w-[460px] flex-col rounded-none border-y-0 border-r-0 shadow-2xl"
      >
        <header className="titlebar-pad flex items-start justify-between gap-3 border-b border-line px-4 pb-3">
          <div className="min-w-0">
            <div className="text-[11px] text-ink-subtle">
              {projectName} · {shortId(taskId)}
            </div>
            <h2 className="truncate text-[14px] font-semibold text-ink">
              {detail?.title || "Task"}
            </h2>
          </div>
          <button
            className="rounded-lg border border-line p-1.5 text-ink-muted hover:text-ink"
            onClick={onClose}
            aria-label="Close"
          >
            <Icon name="x" className="h-3.5 w-3.5" />
          </button>
        </header>

        <div className="scroll flex-1 px-4 py-4">
          {error && <p className="text-[12px] text-danger">{error}</p>}
          {!detail && !error && (
            <p className="text-[12px] text-ink-subtle">Loading…</p>
          )}
          {detail && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-3">
                <Kpi
                  label="Active"
                  value={formatMillis(
                    detail.runs.length
                      ? detail.runs[detail.runs.length - 1].active_ms
                      : null,
                  )}
                />
                <Kpi label="Tokens" value={formatTokens(detail.total_tokens)} />
                <Kpi label="Reported cost" value={formatUsd(detail.reported_cost_micros)} />
                <Kpi label="Attempts" value={String(detail.attempts)} />
              </div>
              <dl className="space-y-1 text-[12px]">
                <Row label="Status" value={detail.status} />
                <Row label="Kind" value={detail.kind} />
                <Row
                  label="Provider / model"
                  value={
                    [detail.provider, detail.model].filter(Boolean).join(" / ") || "—"
                  }
                />
                {detail.review_status && (
                  <Row label="Review" value={detail.review_status} />
                )}
                <Row
                  label="Created"
                  value={new Date(detail.created_at_ms).toLocaleString()}
                />
                {detail.started_at_ms && (
                  <Row
                    label="Started"
                    value={new Date(detail.started_at_ms).toLocaleString()}
                  />
                )}
                {detail.ended_at_ms && (
                  <Row
                    label="Finished"
                    value={new Date(detail.ended_at_ms).toLocaleString()}
                  />
                )}
              </dl>

              {detail.error && (
                <p className="rounded-lg border border-danger-line bg-danger-soft px-3 py-2 text-[11.5px] text-danger">
                  {detail.error}
                </p>
              )}

              <section>
                <h3 className="mb-2 text-[12px] font-semibold text-ink">Runs</h3>
                {detail.runs.length === 0 ? (
                  <p className="text-[11.5px] text-ink-subtle">No run records.</p>
                ) : (
                  <ul className="space-y-2">
                    {detail.runs.map((r) => (
                      <li
                        key={r.run_id}
                        className="rounded-lg border border-line bg-well px-3 py-2 text-[11.5px]"
                      >
                        <div className="flex items-center justify-between">
                          <span className="text-ink">Attempt {r.attempt}</span>
                          <span className="text-ink-muted">
                            {compactDuration(Math.round((r.active_ms ?? 0) / 1000))}
                          </span>
                        </div>
                        <div className="mt-0.5 flex flex-wrap gap-x-3 text-ink-subtle">
                          <span>{r.status}</span>
                          <span>{formatTokens(r.total_tokens)} tokens</span>
                          <span>{formatUsd(r.reported_cost_micros)}</span>
                          <span>{r.inference_count} calls</span>
                        </div>
                      </li>
                    ))}
                  </ul>
                )}
              </section>

              {detail.errors.length > 0 && (
                <section>
                  <h3 className="mb-2 text-[12px] font-semibold text-ink">
                    Errors ({detail.errors.length})
                  </h3>
                  <ul className="space-y-2">
                    {detail.errors.map((e) => (
                      <li
                        key={e.error_id}
                        className="rounded-lg border border-line bg-well px-3 py-2 text-[11.5px]"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span className="min-w-0 truncate text-ink">
                            {errorCategoryLabel(e.category)} · {e.code}
                          </span>
                          <span className="shrink-0 text-ink-subtle">
                            {e.recovered === true
                              ? "recovered"
                              : e.recovered === false
                                ? "not recovered"
                                : "unknown"}
                          </span>
                        </div>
                        <div className="mt-0.5 flex flex-wrap gap-x-3 text-ink-subtle">
                          <span>{e.stage}</span>
                          {e.tool_name && <span>{e.tool_name}</span>}
                          <span>{new Date(e.occurred_at_ms).toLocaleString()}</span>
                        </div>
                      </li>
                    ))}
                  </ul>
                </section>
              )}

              {detail.interactions.length > 0 && (
                <section>
                  <h3 className="mb-2 text-[12px] font-semibold text-ink">
                    Human interactions ({detail.interactions.length})
                  </h3>
                  <ul className="space-y-2">
                    {detail.interactions.map((i) => (
                      <li
                        key={i.interaction_id}
                        className="rounded-lg border border-line bg-well px-3 py-2 text-[11.5px]"
                      >
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-ink">
                            {interactionKindLabel(i.kind)} ·{" "}
                            {questionCategoryLabel(i.category)}
                          </span>
                          <span className="shrink-0 text-ink-subtle">
                            {i.outcome ?? "unresolved"}
                          </span>
                        </div>
                        <div className="mt-0.5 text-ink-subtle">
                          {i.resolved_at_ms
                            ? `waited ${formatMillis(i.resolved_at_ms - i.asked_at_ms)}`
                            : "still open"}
                        </div>
                      </li>
                    ))}
                  </ul>
                </section>
              )}

              <button
                className="btn btn-primary w-full justify-center"
                onClick={() => onOpenTask(detail.project_path, detail.task_id)}
              >
                Open task in workspace
              </button>
            </div>
          )}
        </div>
      </aside>
    </>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="text-ink-subtle">{label}</dt>
      <dd className="min-w-0 truncate text-right text-ink">{value}</dd>
    </div>
  );
}
