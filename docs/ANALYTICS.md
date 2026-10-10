# Solayge Analytics — Product, UX, and Technical Implementation Plan

**Status:** Proposed implementation specification  
**Prepared:** October 8, 2026  
**Audience:** Solayge developer / AI coding agent  
**Scope:** Cross-project, local-first analytics for AI coding tasks and agents  
**Evidence basis:** Screenshot of Solayge desktop UI provided October 8, 2026; research links in §18. **The live Solayge repository was not inspected for this document.** File paths, framework details, and database choices below are recommendations, not verified current implementation details.

## 0. Executive decisions

| Decision | Recommendation |
|---|---|
| Navigation | A persistent **Analytics** item in the global left sidebar, anchored below the scrollable project list and above the existing **Max parallel** footer; not another top-right toolbar icon. |
| Icon | Lucide **`ChartNoAxesCombined`**, 18–20 px, with visible “Analytics” label and tooltip when sidebar is collapsed. |
| Default landing page | Global **Overview**, all projects, last 30 days, completed/failed terminal task outcomes and token/cost estimates. |
| Information architecture | **Overview**, **Projects & Models**, **Reliability**, **Interactions**. A project context-menu action opens Analytics prefiltered to that project. |
| First-release must-haves | Task counts/status, success/failure rate, queue/run/wait durations, model usage tokens, reported/estimated cost, project comparisons, error summary. |
| Next priorities | Retry/failure stages, model comparisons, permissions and questions, autonomy and first-pass success. |
| Architecture | Store **Solayge task lifecycle as authoritative**, ingest AI runtime usage and interaction data via a backend adapter, and write normalized, idempotent local records. |
| Persistence | Reuse the app’s existing local database if appropriate; **SQLite** is a reasonable default for an offline desktop app. Do not create a parallel analytics database without justification. |
| Privacy | Local-only by default; no full prompts, credentials, terminal output, or response transcripts in analytics. Retention controls and export/delete. |
| Cost precision | Display distinct **provider-reported**, **API-equivalent estimate**, and **actual billed cost** (only when verified). Never imply subscription usage is a real charge. |

### Outcomes the feature must answer

1. Which projects completed the most work, how successfully, and at what token/cost budget?
2. How long does a task take, including queueing, execution, review, and waiting for humans?
3. Where, how often, and why are tasks or agent/tool executions failing?
4. Which models and workflows are most efficient on comparable tasks?
5. What questions and permission prompts keep interrupting autonomous agents?
6. Can the user click any surprising value to inspect the underlying tasks and evidence?

## 1. Current interface and navigation placement

### Observations from the supplied screenshot

- Dark, low-contrast purple/charcoal visual language; compact desktop layout.
- Left column ~270 px at the screenshot’s 2048×1260 capture; header with logo, Solayge title, utility icons; **PROJECTS** list with an **Add** action.
- The project list is followed by an anchored footer containing **Max parallel** slider and three small aggregate counters (**Tasks**, **Running**, **Done**).
- Main workspace has selected project title, branch, change count, task/review/execute controls, agent/provider chips, task progress, and large task graph area.
- Several existing toolbar actions are already dense. Adding Analytics there would suggest “analytics for this open task” rather than “analytics across projects.”
- The screenshot shows multiple related projects with similar/truncated names: an Analytics project selector must use full names and optionally path/repository subtitle.

### Exact placement recommendation

Add a visible **Analytics** row as a persistent global navigation item **between the independently scrolling PROJECTS region and Max parallel footer**. Provide a separator and preserve the current bottom counters. If the sidebar can collapse, display its icon with an accessible tooltip.

```text
┌──────────────────────────────┐
│ Solayge            [settings]│
│                              │
│ PROJECTS              [+ Add]│
│  ▣ solayge                   │
│  ▣ adaptive-learn            │
│  ▣ filequell-desktop         │
│  ▣ filequell-license-server  │
│  ▣ ...                       │
│        (projects scroll)     │
│ ──────────────────────────── │
│  ↗ Analytics                 │  ← Lucide ChartNoAxesCombined
│ ──────────────────────────── │
│ Max parallel              4  │
│ ━━━━━━━━░░░░░░░░░░░░         │
│ [Tasks] [Running] [Done]     │
└──────────────────────────────┘
```

**Details:**

- **Icon:** `ChartNoAxesCombined` ([official icon](https://lucide.dev/icons/chart-no-axes-combined)); prefer a labeled row to an icon-only button. Never add an unread badge to this ordinary nav item.
- Row height ~36–40 px; align with existing project list indentation, hover style and selected purple fill; keyboard-focus visible.
- Selected Analytics route should not falsely select/highlight a project; restore prior selected project when navigating back.
- Sidebar pinned region remains visible even with dozens of projects; never put the Analytics item *inside* the project scroll container.
- Project row context menu: **View analytics**; also allow “View project analytics” from project header menu, but do **not** add another permanent header icon.
- Support command palette action **Open Analytics**; optionally a keyboard shortcut after checking collisions with current shortcuts.
- Avoid confusing Analytics with Solayge’s existing “Max parallel” live operational controls; Analytics is historical/diagnostic, not the execution control surface.

## 2. UX research and how to apply it

Recommendations below are product-design synthesis, not proof that the screenshot’s current implementation has these defects.

| Research finding | Solayge decision |
|---|---|
| Nielsen Norman Group: dashboards are for **at-a-glance** monitoring; position and length communicate quantities well. [[1]](#18-research-sources) | Show **six** key metrics first, two uncomplicated trend charts, followed by an actionable table. Do not make Overview a giant visual report. |
| NN/g: data-table users need to **find, compare, view, and take action**; filters must be visible and sorting understandable. [[2]](#18-research-sources) | Sortable comparison tables, sticky project-name column, searchable projects, right-aligned numerics, click-through task details. |
| IBM Carbon: bars work for category comparisons, lines for changes over time; chart axes and labels must communicate units and avoid distortion. [[3]](#18-research-sources) [[4]](#18-research-sources) | Line for daily activity/cost; horizontal bars for failure categories; numeric axes on bar charts start at zero; label gaps as unavailable. |
| IBM Carbon: chart color must not be the sole differentiator; legends and patterns can aid accessibility. [[5]](#18-research-sources) | Status text/icons and patterns alongside purple, green, amber, and red accents. Charts offer accessible underlying tables. |
| WCAG 2.2 AA includes non-text contrast, visible focus and minimum targets in applicable circumstances. [[6]](#18-research-sources) | Design toward >=4.5:1 normal text, >=3:1 essential graphics/controls; at least 24×24 CSS px targets (prefer 32–40 px for desktop); fully keyboard operable. |
| Atlassian design system provides practical patterns for tabs, dynamic sortable tables, empty states, skeletons, tooltips. [[7]](#18-research-sources) | Reuse Solayge’s existing components; apply established interaction patterns instead of custom novel controls. |

### Visual design principles

- **Preserve current aesthetic:** same dark surfaces, thin borders, subtle purple selected/active accents, restrained shadows and small labels. No bright/filled KPI summary boxes.
- **Hierarchy:** page title (24–28 px), metric value (22–28 px), sectional header (15–17 px), body/table (12–14 px), muted metadata (11–12 px). Text size here is a design target; match the existing token scale where possible.
- **Density:** maximum six KPI tiles across at desktop size; 3×2 around 1150 px content width; 2-column as space narrows; stack at small sizes. Content should be fluid, not horizontally clipped.
- **Data labels:** “Tasks completed”, “Task failure rate”, “Avg. execution time”, “Total tokens”, “API-equivalent cost”, “Autonomy rate”. Show units and precise values on hover/focus.
- **Trust cues:** distinguish `Estimated`, `Reported`, `Incomplete`, `Historical`, and `Unavailable` using ordinary neutral text/icon treatment, not false certainty. Explain metric definition in tooltip and a “How this is calculated” disclosure.
- **Warnings:** never use success green to decorate a generic recommendation. Reserve color for error/success state semantics and chart series.

## 3. Information architecture and flows

**Navigation:** Global sidebar `Analytics` → page containing four tabs. URL/router state (or equivalent local navigation state) includes active tab, project(s), date range, model(s), state filters, and selection for drill-down; support back/forward and deep linking if routing permits.

| Tab | Answer | Main components |
|---|---|---|
| **Overview** (default) | Are agents doing useful work, reliably and economically? | Six KPI metrics, daily activity trend, token/cost trend, top project table, short top errors/questions previews. |
| **Projects & Models** | Which projects/models use time/tokens and succeed? | Project comparison table, model breakdown table, project detail drawer, grouped token & pricing breakdown. |
| **Reliability** | What fails, how often, and how recoverably? | Failure trends, error categories, stage/tool breakdown, recurring error fingerprints, affected task list. |
| **Interactions** | What stops agents from operating autonomously? | Questions asked, human blockers, permission frequency, latency to human response, repeated-question clusters. |

**Global controls:** compact filter bar under title: `Date [Last 30 days ▼]  Project [All ▼]  Model [All ▼]  [Reset] [Export]`. Date choices: 7/30/90 days, all time, custom range. Default last 30 days. Preserve filters between tabs. Exports apply to the current filters and clearly label time zone/currency/estimate type.

**Progressive disclosure:** KPI or chart click → filtered task list → task-specific analytics drawer (with task name, status, project, model/provider, attempts, timeline, tokens, costs, error/interaction events, links to existing task details). Drawer closes back to the same scroll/filter context. Avoid modal stacks.

**Task-context access:** Task detail can show small metrics `31m active • 4.2M tokens • $0.73 estimate • 2 retries`. Prefer integrated display over new top-bar controls. Hide unavailable metrics rather than displaying misleading `$0.00`.

### Overview desktop wireframe (~1400 px usable content)

```text
Analytics                                   [Last 30 days ▼] [Export]
[Overview] [Projects & Models] [Reliability] [Interactions]
[All projects ▼]  [All models ▼]  [Reset filters]

 Tasks completed   Success rate   Avg. active time   Tokens used   Est. API cost   Autonomy
      128              91.4%           18m 12s          24.8M       ~$42.50         79%

┌─ Task activity (daily) ───────────────┐ ┌─ Token / cost trend ────────────────┐
│ line: completed vs failed             │ │ line: token usage or USD [toggle]    │
│ axis: count; date; explicit gaps      │ │ axis: millions or USD; date          │
└───────────────────────────────────────┘ └──────────────────────────────────────┘

Project comparison                                [Search projects]
Project                    Tasks  Success   Avg time     Tokens     Est. cost
filequell-desktop             42     92%        18m        8.2M       ~$12.40
solayge                       28     96%        12m        4.1M        ~$6.20
adaptive-learn                19     84%        25m        5.7M        ~$8.50
[View all projects]

Top failures                            Questions / permissions
Build failure  18                     Missing requirements  12
Test failure   14                     Permission requests    9
[View Reliability]                     [View Interactions]
```

*All example figures are mock values, not measured Solayge data.*

### Reliability desktop wireframe

```text
Reliability                                  [Last 30 days ▼] [All projects ▼]
Task failure rate: 8.6%    Runs with errors: 21%    Recovery rate: 76%

Failure volume (daily)            Error categories by count
[time series]                     [horizontal bars]

[Error category ▼] [Stage ▼] [Tool ▼] [Show repeated only □]
Fingerprint / Summary        Occurrences  Tasks  Auto-recovered  Last seen
TS2322 type incompatibility     14         5      11              Yesterday
cargo test assertion failed     10         4       7              Oct 6
API rate limit                  8          6       5              Oct 5

Click row -> contextual drawer with sanitized sample, affected tasks,
first/last seen, model and stage, reproduction evidence where available.
```

### Responsive desktop behavior

- **Wide (>=1200 px content):** six KPIs on one row, two charts side-by-side, full comparison table.
- **Medium (800–1199 px):** 3×2 KPIs, two charts stacked if cramped, horizontal table scroll inside its own viewport; sticky project name.
- **Narrow (<800 px):** 2-column KPI cards, tabs scroll horizontally with keyboard access; comparison table becomes column-selectable table/list; preserve values and labels. Sidebar may collapse if that is already a pattern in Solayge.
- Never force the entire analytics page to horizontally scroll. For many projects/models, add pagination/virtualization if necessary; do not truncate both name and repository path beyond recognition.

### Loading/empty/error states

| State | UI |
|---|---|
| No task history | “No task data yet. Run a task to start collecting analytics.” Link to projects; no fake zeros for missing metrics. |
| Task history, no usage data | Show task counts/durations; usage card: “Token data unavailable for these runs.” Offer a diagnostic explanation. |
| Some models missing usage | Annotate `Coverage: 17/20 task runs`; calculate known totals and report missing count, not a silently complete number. |
| No matching filters | “No activity matches your filters.” One-click **Reset filters**. |
| Backfill in progress | Nonblocking “Importing existing history… 63/100 runs”; page remains usable. |
| Backend/data-store problem | Retry action and precise non-sensitive error; analytics failure must not disrupt task execution. |
| Loading | Stable skeleton in charts/table only; do not blank out the navigation or already-available cached aggregate totals. |

## 4. Canonical data model and definitions

**Critical distinction:** a **task** is a user-intended unit of work; a **run/attempt** is one execution of a task; a **session** is an AI runtime conversation/process; a **message/inference** is a billed or measured model invocation; a **tool event** is an operation such as shell/build/test. These are not interchangeable, and one task may have multiple runs, sessions, subagents, models, and review cycles.

| Metric | Canonical definition / handling |
|---|---|
| **Total tasks** | Distinct `task_id` created during the interval; separate task creation cohorts from executions in date-based charts. |
| **Completed tasks** | Distinct task IDs whose latest terminal outcome is **success**, with terminal timestamp inside range (completion cohort). |
| **Failed tasks** | Distinct task IDs whose latest terminal outcome is **failed**, not including canceled/aborted unless explicitly selected. |
| **Task success rate** | `successful terminal tasks / (successful + failed terminal tasks)`; exclude queued/running/canceled from denominator; display `N` sample. |
| **Task failure rate** | `failed terminal tasks / (successful + failed terminal tasks)`; not “fraction of runs that experienced an error.” |
| **Runs with errors** | `run IDs with >=1 classified error / all observed started run IDs` in range. May include successful tasks. |
| **Error event rate** | `error occurrences / total tool operations` *only* for tool error analysis; use a different name from task failure rate. |
| **Retry rate** | `tasks with >1 execution attempt / tasks with >=1 started attempt`; keep task review cycles separate from provider transient retries. |
| **First-pass success** | `success with no failed run/review/test retry / terminal successful+failed tasks` where review/test validation is available; mark coverage. |
| **Queue duration** | `first_started_at - queued_at`; report median/p95, not just mean. |
| **Active execution duration** | Sum of non-overlapping intervals when run was executing (exclude paused/waiting/queued); **do not sum parallel subagent wall times** as elapsed duration. |
| **Elapsed task duration** | `terminal_at - created_at` (or explicit started-to-terminal value if titled “run elapsed”); includes queue/human waiting. |
| **Average running time** | Arithmetic mean of *finalized* run active durations in filter; also display median/p95; exclude in-progress runs from averages or show separately. |
| **Human wait** | Sum of non-overlapping intervals from actionable prompt/request to user reply or cancellation; do not treat agent rhetorical questions as blocking. |
| **Autonomy rate** | `terminal successful tasks with zero blocking human interventions / terminal successful tasks with observable intervention data`; display numerator/denominator and coverage. |
| **Questions asked** | Count detected actionable agent questions; keep separate from permission prompts and ambiguous natural-language questions. |
| **Human interventions** | Count distinct blocker IDs (permission, clarification, credential/setup request, manual action) requiring human input; retries/repeated UI render are not new interventions. |
| **Time-to-response** | `resolved_at - requested_at` for resolved blocker events; unresolved excluded and shown separately. |
| **Cost per successful task** | Sum attributable cost for all included attempts of selected task cohort, including failures, divided by successful task count; clarify whether spend from unsuccessful terminal tasks is included based on chosen metric. Prefer both `cost per completed task` and `cost per successful task (including failed attempts on completed tasks)`. |
| **Token usage** | Sum normalized per-inference usage records, not session summaries **plus** message usage. Preserve original provider counts and category semantics. |
| **Estimated cost** | Rate-card calculation at the time of usage, with model/provider, unit, currency, rate version and uncertainty. |
| **Actual billed cost** | Only if authoritative payment or provider billing data is available and attributable; otherwise `Unavailable`. |

**Date semantics:** Overview completion/outcome KPIs use task terminal time. The token/cost line chart uses actual invocation timestamps; run duration views use run start/stop timestamps. Explain this in hover help; do not pretend every chart uses the same event-cohort time. Apply project/model filters to records with a reliable association. Model-filtered task success requires an attribution rule for multi-model tasks (default: exclude/label as “mixed” for per-model outcomes, rather than crediting the whole task to every model).

**Durations and concurrency:** Record lifecycle timestamps in UTC with millisecond precision, render in local timezone. Prefer a monotonic clock for in-process durations; on restart use persisted wall times and flag uncertainty. Track `queued`, `active`, `waiting_on_user`, `waiting_on_external`, `reviewing`, `completed/failed/canceled` as mutually exclusive state intervals where feasible. Interval overlaps must be deduplicated when summing task elapsed time.

**Missing data:** `null` means **unknown**; `0` means observed zero. Do not infer absent token counts or costs to be zero. Display coverage with any aggregate missing telemetry.

## 5. Detailed analytics domains

### 5.1 Project + task analytics (Phase 1)

Project table columns: **Project**, **Tasks completed**, **Success %**, **Failed**, **Avg active time**, **Median active time**, **Tokens**, **Estimated API cost**, optionally **Human interventions**. Full project name and repo path available via subtitle/tooltip. Clicking opens a filtered task list; clicking task shows attempts and timeline.

Task drill-down includes ID, title, status, task stage, branch/worktree (if available), created/started/finished timestamps, total time broken down by phase, agent/backend/model list, model usage records, errors/retries, review outcome, and source trace links. Avoid showing private prompt text by default.

### 5.2 Model + cost analytics (Phase 1/2)

Model table: provider/model ID, tasks with attributable usage, inference calls, input/output/reasoning/cache tokens where available, **reported cost**, **API-equivalent estimate**, latency distribution, failed tool/run percentages, sample size. Sort by metric. Optional comparable-cohort filter by task type and difficulty before showing rankings.

**Cost UX:** Labels must not conflate subscription usage with actual charges. An OpenCode Go/Go Plus user could pay a subscription while experiencing an API-equivalent usage value. Show:

- `Reported usage cost` — the runtime/provider's reported figure; explain if it is a model-price calculation, not a bill.
- `Est. API-equivalent cost` — calculated with a dated, versioned per-model input/output/cache rate card; prefix `~` if approximate.
- `Actual billed` — only from a billing/receipt integration, otherwise an em dash + explanation.

Where token categories are ambiguous, trust a provider-supplied `totalTokens` for totals when valid, or use a **documented normalization rule** for the specific provider. Do **not** blindly add input + output + reasoning + cache: some reports include reasoning within output and cached tokens within input. Store each category and its original raw fields. Provider/model identity **and variant** should be captured when supported.

`total token` and `cost` panels should show coverage: e.g. `98% of inferences reported usage`. If cost is unknown for one provider, total estimated cost is a **known subtotal** and labeled as such, never a complete total. No fabricated historical rates. Preserve cost snapshots rather than recalculating past actuals with today's prices.

### 5.3 Error and reliability analytics (Phase 2)

Recommended error taxonomy:

| Top-level category | Examples |
|---|---|
| `provider` | rate limit, unavailable model, authentication, context overflow, timeouts |
| `tool` | shell command failure, filesystem permission, tool timeout, malformed tool arguments |
| `build` | compiler/type errors, bundler/package failures |
| `test` | unit/integration assertions, CI command failures |
| `security_policy` | blocked permission, unsafe operation disallowed |
| `agent_runtime` | session crash, stuck state, invalid state transition |
| `integration` | Git/worktree/branch merge, DB/service integration failure |
| `unknown` | insufficient signal; never guess a precise cause |

Captured error record: **fingerprint**, sanitized error category and summary, severity, stage, originating tool, task/run/session/attempt IDs, timestamps, count, auto-recovered (`true/false/unknown`), test/build source evidence references. Repeated occurrence must not automatically imply task failure.

**Fingerprinting:** deterministic normalized code (`TS2322`, `ENOENT`, HTTP 429, Cargo test name, etc.) plus stable source/context. Strip absolute home paths, tokens, random IDs, timestamps. Only group errors when signals match, otherwise label uncertain. Drilldown: first/last occurrence, affected tasks, successful recovery examples, and links to existing logs if user explicitly requests details.

**Recovery rate:** `observed recoveries / recoverable error instances with known resolution`; define resolution as a subsequent successful action of the same operation or successful final run without the error remaining blocking. Keep `unknown` excluded and disclose coverage.

### 5.4 Questions, permissions, and autonomy (Phase 3)

Track *actionable* human attention separately from free-form sentences in model responses:

| Interaction type | Reliable event source | Default content storage |
|---|---|---|
| Permission request | Runtime permission asked/replied event | Category, tool, timestamps; no raw command by default |
| Explicit question UI | Structured runtime question prompt + response if available | Question category, status, timestamps; no raw text by default |
| Manual confirmation | Solayge UI action blocking/resuming a run | Event type and duration |
| Credential/setup request | Solayge structured interruption or opt-in classifier | Type only; no secret or credential values |
| Free-text question | Best-effort opt-in extraction from messages | Off by default; redacted and reviewed |

Initial question categories: missing requirements, implementation choice, missing repository instructions, environment setup, permissions, credentials, approval/deployment, ambiguous success criteria, other/unknown.

**Grouping:** Start with exact structured categories and normalized question templates. Only if truly beneficial, add an optional on-device semantic clustering path; never send local task transcripts to cloud LLMs by default. Show confidence, group membership and `Ungroup` / `Wrong category` actions. Do not equate a repeated tool-permission prompt with “AI repeatedly asking the same question” unless it was an actual blocking request.

**Recommendations:** If the same instruction is requested frequently, suggest “Document chosen database in AGENTS.md” or “Add build/test commands to repository instructions.” Recommendations are **advisory**; never auto-edit AGENTS.md, loosen permissions, or create secrets on their basis. Avoid LLM usage/cost merely to display routine charts.

### 5.5 Later/optional metrics

Useful if reliable: stage bottlenecks, review retry reasons, `p95` duration, cost of abandoned attempts, tool failure rate, context cache efficiency, parallelism efficiency, interrupted-session recovery rate, task-type distribution, provider outage windows, exportable per-task event chronology. **Defer** speculative developer-hours-saved and raw code-change counts as productivity scores; they are easy to misinterpret.

## 6. Analytics ingestion architecture

**Principle:** Solayge owns task state and scheduling; OpenCode or any other agent runtime provides usage, tool, message, and permission events. Analytics combines them through an adapter, but does **not** control execution.

```text
Solayge Task Scheduler / Task State Machine
    │ task/run/status timestamps; reviewed outcome
    ├──────────────┐
    │              │
OpenCode Adapter  Other Provider Adapters (future)
    │ sessions, messages, tool/permission events
    └──────────────┬───────────────┘
                   ▼
         Normalize + Deduplicate
       (schema validation, redaction)
                   ▼
          Local Event Repository
           + Usage Snapshots
                   ▼
          Aggregation / Queries
                   ▼
        Analytics UI + CSV/JSON
```

**OpenCode source feasibility (verified from current docs):**

- Server API exposes `GET /session`, `GET /session/:id`, `GET /session/:id/message`, `/event`, `/global/event`, and OpenAPI documentation at `/doc`; JS/TS SDK exposes client methods. [[8]](#18-research-sources) [[9]](#18-research-sources)
- Plugin events include session status/error/idle, `message.updated`, `message.part.updated`, permission asked/replied, and tool execution hooks. **Events provide signals, not necessarily finalized, directly billable rows.** [[10]](#18-research-sources)
- OpenCode source contains normalized token and cost handling for sessions and models; **actual SDK shape and semantics must be checked for Solayge’s installed OpenCode version** before adapter implementation. [[11]](#18-research-sources)

**Recommended integration steps:**

1. Inspect existing Solayge task runner, storage, and OpenCode bridge; identify a stable `task_id -> run_id -> provider session_id` relation. Record mappings *at launch* rather than guessing from titles or branch names after completion.
2. Subscribe to lifecycle/interaction events from the **same** OpenCode runtime instance already used by Solayge, or extend current bridge. Do not spawn a second server only to collect metrics.
3. Ingest **Solayge** task/run lifecycle directly with timestamps and terminal outcome; never derive success from `session.idle` alone (idle means session not active, not necessarily successful/review-passed).
4. When a message becomes finalized or a session reaches a consistent checkpoint, query message `info` and reconcile per-message token/cost snapshots using a stable composite identity. Streaming message-updated events can repeat; do **not** sum them as deltas.
5. Subscribe to permission/tool error events where supported. Attach events via known run/session mapping; if correlation is missing, put in `unattributed` diagnostics, not in the wrong project.
6. On app start/reconnect, reconcile unfinished runs and recent sessions with persisted watermarks. Reconciliation is idempotent; allow a finite backfill horizon and a user-initiated full rebuild.
7. Keep collector failures isolated: nonblocking queue with bounded memory, transaction commits, exponential backoff, and health status; never slow down a task just to write analytics.
8. Version the adapter and feature-detect capabilities, since provider event schemas may change.

**Do not:** parse terminal stdout and regex “success” as canonical outcome; duplicate entire OpenCode conversation data; permanently log raw API payloads; assume `session.idle == task.completed`; count a child/subagent session as a new Solayge task.

## 7. Proposed persisted schema (conceptual)

Use existing DB abstraction/migrations. SQLite column syntax shown as a portable conceptual reference, **not a drop-in migration until repository schema is reviewed**.

```sql
CREATE TABLE analytics_events (
  event_id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  source_event_id TEXT,
  event_type TEXT NOT NULL,
  project_id TEXT,
  task_id TEXT,
  run_id TEXT,
  session_id TEXT,
  occurred_at_ms INTEGER NOT NULL,
  recorded_at_ms INTEGER NOT NULL,
  payload_version INTEGER NOT NULL,
  sanitized_payload_json TEXT
);
CREATE INDEX idx_analytics_event_date ON analytics_events(occurred_at_ms);
CREATE INDEX idx_analytics_event_task ON analytics_events(task_id, occurred_at_ms);

CREATE TABLE analytics_task_runs (
  run_id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  attempt_number INTEGER NOT NULL,
  status TEXT NOT NULL,
  queued_at_ms INTEGER,
  started_at_ms INTEGER,
  ended_at_ms INTEGER,
  active_ms INTEGER,
  waiting_user_ms INTEGER,
  review_attempt_count INTEGER,
  validation_outcome TEXT,
  telemetry_quality TEXT NOT NULL DEFAULT 'partial',
  UNIQUE(task_id, attempt_number)
);
CREATE INDEX idx_analytics_runs_project_time
  ON analytics_task_runs(project_id, started_at_ms);

CREATE TABLE analytics_model_usage (
  usage_id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  session_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  project_id TEXT,
  task_id TEXT,
  run_id TEXT,
  occurred_at_ms INTEGER NOT NULL,
  provider_id TEXT,
  model_id TEXT,
  variant TEXT,
  input_tokens INTEGER,
  output_tokens INTEGER,
  reasoning_tokens INTEGER,
  cache_read_tokens INTEGER,
  cache_write_tokens INTEGER,
  provider_total_tokens INTEGER,
  reported_cost_usd TEXT,
  estimated_api_cost_usd TEXT,
  rate_card_version TEXT,
  telemetry_quality TEXT NOT NULL,
  UNIQUE(source, session_id, message_id)
);
CREATE INDEX idx_analytics_usage_project_date
  ON analytics_model_usage(project_id, occurred_at_ms);

CREATE TABLE analytics_errors (
  error_id TEXT PRIMARY KEY,
  event_id TEXT UNIQUE,
  project_id TEXT,
  task_id TEXT,
  run_id TEXT,
  session_id TEXT,
  category TEXT NOT NULL,
  fingerprint TEXT,
  stage TEXT,
  tool_name TEXT,
  summary_redacted TEXT,
  occurred_at_ms INTEGER NOT NULL,
  recovered INTEGER -- NULL=unknown; 0=false; 1=true
);
CREATE INDEX idx_analytics_error_fingerprint
  ON analytics_errors(fingerprint, occurred_at_ms);

CREATE TABLE analytics_interactions (
  interaction_id TEXT PRIMARY KEY,
  source_event_id TEXT,
  project_id TEXT,
  task_id TEXT,
  run_id TEXT,
  session_id TEXT,
  kind TEXT NOT NULL,
  category TEXT,
  fingerprint TEXT,
  asked_at_ms INTEGER NOT NULL,
  resolved_at_ms INTEGER,
  blocking INTEGER NOT NULL DEFAULT 0,
  source_confidence TEXT NOT NULL DEFAULT 'structured'
);
CREATE INDEX idx_analytics_interactions_task
  ON analytics_interactions(task_id, asked_at_ms);
```

**Schema notes:**

- IDs should be opaque, stable, and unique. For provider-generated events without stable IDs, derive deterministic IDs from canonical source identity/type/version; make dedup rules explicit.
- `source_event_id` is not guaranteed unique across event types/backends. Use a scoped unique key only when the source contract warrants it.
- Cost stored as decimal **string** or integer micro-USD in storage; do not accumulate financial values with imprecise JS floating-point sums.
- Separate immutable events from mutable **current snapshots**: upsert `analytics_model_usage` with final message values; avoid incrementing tokens for every partial update.
- Task metadata/project IDs should reference existing canonical tables where possible. Ensure SQLite foreign-key behavior and deletion policy match the existing app.
- Task status event history must support resumed, re-run, and canceled states; if full state history exists elsewhere, avoid duplicating it unnecessarily.
- Pricing rate cards are separate, versioned metadata; never write API keys there. Define database migrations before first release.
- App-side query layer returns scoped aggregates and task IDs; UI should **not** load all raw events to compute totals. Use indices and SQL grouping; materialized daily aggregates only after profiling demonstrates need.

## 8. Analytics service/API contract

If Solayge already has a Tauri command or local IPC query layer, implement analytics there rather than adding an HTTP server.

Suggested *logical* service methods (adapt naming to existing conventions):

```ts
getAnalyticsOverview(filters): Promise<OverviewSummary>
getAnalyticsTimeSeries(filters, resolution): Promise<TimeSeriesPoint[]>
getAnalyticsProjects(filters, sort, pagination): Promise<ProjectMetricPage>
getAnalyticsModels(filters, sort, pagination): Promise<ModelMetricPage>
getAnalyticsErrors(filters, groupBy, pagination): Promise<ErrorGroupPage>
getAnalyticsInteractions(filters, pagination): Promise<InteractionGroupPage>
getAnalyticsTaskDetail(taskId): Promise<TaskAnalyticsDetail>
exportAnalytics(filters, format): Promise<LocalFileResult>
getAnalyticsCollectionStatus(): Promise<CollectionStatus>
```

Types must make `unavailable` distinguishable from numeric zero. Return explicit `sampleSize`, `coverage`, `timeZone`, `currency`, `priceBasis`, and (where applicable) `isEstimate`. Validate date ranges and bounds in the backend, not only the UI. Use pagination for potentially large tables and sanitize exported content to avoid CSV formula injection.

## 9. Backfill, retention, and settings

- Backfill task IDs, project IDs, statuses and timestamps from existing **Solayge-owned** records first. Only backfill token/cost/error/interaction data where reliable, permitted source records exist.
- Run a resumable, idempotent local backfill with progress indicator and cancel support; do not block task execution. Mark imported values `historical` or `partial` where semantics differ.
- User controls in existing Settings → **Analytics**: `Collect local analytics` (on by default for non-content operational metadata), `Capture sanitized error examples` (off by default), `Analyze free-text questions` (off by default), history retention (e.g. 30/90/365 days/forever), export JSON/CSV, clear history.
- Disabling collection stops new collection but does not silently delete existing analytics. Clearing should show an explicit confirmation with affected scope and explain its effect on historical charts.
- Export locally only, avoid cloud sync by default. If Solayge later supports phone access, serve analytics through the **existing authenticated device channel** with explicit authorization; do not open an unauthenticated analytics endpoint.
- Never store secrets, full file paths with sensitive usernames if avoidable, full transcript text, env vars, raw shell commands or terminal output without explicit opt-in. Redact at ingestion before persistence, not just at rendering.

## 10. Security and reliability requirements

- Local analytics storage should use app-scoped filesystem permissions and existing data protection conventions; no external telemetry/third-party analytics dependency required.
- Validate and bound incoming event payload size; allowlist fields; reject oversized or malformed payloads without breaking task execution.
- Avoid path traversal in export filenames; use OS file-picker and atomic write; protect from CSV formula injection in free-text fields (`=`, `+`, `-`, `@`, tabs/newlines depending on export method).
- Log only sanitized, minimal analytics collector errors; prevent sensitive payloads in diagnostics and crash reports.
- Queries must be read-only to task state and must not allow a dashboard action to cancel/run/delete tasks accidentally.
- Respect project/task deletion: remove, anonymize, or retain only aggregate records according to user's configured policy; document choice and behavior.
- Bound collector queue, SQLite write contention, backfill concurrency, and query response sizes. On crashes, resume safely without duplicate counts or negative durations.
- Treat imported provider metadata as untrusted text in React/WebView and in exports; escape content and avoid dangerously set HTML.
- No subscription or pricing data retrieved remotely without user awareness; rate-table updates can be packaged or opt-in fetched, with version metadata.

## 11. QA, functional tests, and UX acceptance tests

### Metric correctness tests

- Task with three runs (failed, failed, success) is **one** completed task, **three** attempts; costs all attributable invocations; retry rate counts that task once.
- Successful run with compiler errors later fixed contributes to **run with errors**, not **failed task**.
- Canceled task excluded from success/failure denominator, shown distinctly in task state breakdown.
- Running and queued tasks excluded from finalized average duration; sample size and unknown durations visible.
- Two parallel subagents do not double wall-clock task active duration; their token use **is** aggregated if attributable and not already included elsewhere.
- Repeated `message.updated` snapshots do not duplicate tokens/cost; a later finalized correction replaces earlier snapshot.
- A task using multiple models is attributed as `mixed` for unambiguous per-model outcome comparisons; per-inference usage stays attributable to each model.
- Missing cost is **unknown**, not `$0`; missing token count is **unknown**, not zero.
- Subscription vs API estimate displays correct basis and does not imply a card charge.
- Clear history/deletion preserves task execution and resets only intended analytics scope.

### Reliability / data-quality tests

- Loss of SSE connection, out-of-order/replayed event, app restart during run, duplicate session ID, delayed model usage record, clock skew, partial backfill.
- Backfill can be resumed/restarted without double counting; project rename or worktree changes do not split project history when ID is stable.
- Provider API schema drift yields partial coverage and diagnostics, not a crash; collector cannot stall task scheduler.
- Error fingerprints redact username/home paths, tokens, and random identifiers; repeated error groups are stable across runs.
- User reply resolves exactly one linked blocker; expired or canceled questions remain unresolved unless explicitly canceled.

### UI/interaction tests

- Analytics nav visible with 30+ projects and short window; project list independently scrollable.
- Accessibility: tab order, focus indicators, keyboard-selectable tabs/filters/charts/table rows; screen-reader-friendly KPI values and chart table alternative.
- Dark-theme contrast at normal and scaled fonts; charts can be understood without color alone.
- 2048×1260 screenshot-like window and smaller sizes: no clipped KPIs, footer or unwanted page-width overflow.
- All filters apply consistently; persist when switching tabs; reset predictable; sorting numeric not lexical (`9` before `100` must not happen in ascending).
- Clicking KPI/chart/table navigates to filtered detail without losing return position.
- Empty/no results/loading/partial telemetry/error screens have actionable, nonmisleading states.
- Charts use correct timestamp basis, time zone and axis labels; no interpolated lines across missing data.
- Exports include scope, timezone, units, estimate provenance and escape malicious CSV cells.

### Release quality gates

- Pass repository formatter/linter/typechecker, backend/frontend unit tests, integration tests, migrations, production build and relevant packaging/smoke checks.
- Measure analytics page load/query cost on at least **10,000 tasks and 100,000 usage/events** synthetic dataset; choose reasonable baseline targets *after* profiling on representative hardware. UI must remain responsive while collection/backfill runs.
- No observed loss/duplication in restart and replay tests. No sensitive content persisted by default.
- Conduct a small manual usability walkthrough: (a) find most expensive project, (b) find recurring build failure and affected tasks, (c) find top repeated agent question, (d) explain why cost is estimated.

## 12. Phased delivery with acceptance criteria

### Phase 0 — Repository audit and telemetry design (small, mandatory)

**Inspect:** current storage/DB, task and run lifecycle, OpenCode bridge and installed version, review pipeline, existing components/navigation/router, testing infrastructure and privacy patterns. Produce a short capability matrix: metrics available, missing, or inferred. Confirm what “completed” and “review passed” mean in Solayge today.

**Deliverables:** metric contract, event-source mapping, source-to-task ID attribution, DB migration plan, UI component reuse map.

**Done when:** each Phase 1 metric has a reliable data source or a clear `Unavailable` fallback; no architecture assumptions remain unverified.

### Phase 1 — Useful MVP (highest priority)

**Build:** sidebar Analytics row; Overview and Projects & Models baseline; task/run lifecycle analytics, duration metrics, local persistence; OpenCode message usage ingestion; token + reported/estimated cost with coverage; activity trend; project comparison table; filters/drill-down; partial/empty/loading states; safe export.

**Done when:** user can answer “which projects ran how many tasks, how long, how many tokens, and roughly what API-equivalent value?”; values stay correct after app restart and across multiple attempts; production build/test suite passes.

### Phase 2 — Reliability + model efficiency

**Build:** normalized error events and categories, fingerprints, error-rate definitions and trends, retries/recovery, failure stage/tool breakdown, model comparison with mixed-model handling, affected-task drill-down; second-pass privacy audit.

**Done when:** user can locate a recurring failure, distinguish error occurrence from task failure, see whether agents self-recovered, and compare models without overstating success attribution.

### Phase 3 — Questions, intervention and autonomy

**Build:** permission/question/approval structured-event tracking, interruption durations, question-category grouping, autonomy/first-pass metrics, recommendation copy linking to project documentation, optional manual misclassification correction. Keep semantic extraction optional/off by default.

**Done when:** user can see which agent questions recur and how much human interaction delayed execution; no raw prompt/question transcripts retained by default.

### Phase 4 — Hardening and polish

**Build:** pagination/virtualization when needed, lifecycle drift reconciliation, backfill enhancement, responsive refinements, accessibility audit, project shortcut, telemetry quality diagnostics, long-term retention/export robustness.

**Done when:** metrics reconcile after shutdown/restart, analytics remain responsive at realistic volume, all documented security/UX checks pass.

## 13. Prioritization: what NOT to build initially

- No external SaaS observability infrastructure, cloud accounts, or mandatory server just for desktop analytics.
- No real-time animated charts polling each second; historical overview can update on terminal events and when opened, with bounded refresh.
- No LLM-generated insights by default; do rule-based grouping and metric definitions first.
- No speculative “hours saved” or per-model leaderboard without comparable sample sizes.
- No automatic AGENTS.md edits or permission-policy relaxations.
- No new expensive UI chart framework before evaluating existing dependencies/components.
- No per-project Analytics sidebar entries; one global destination with filters is simpler.

## 14. Implementation task breakdown for coding agent

- [ ] **A01** Inspect live architecture, task states, OpenCode integration and DB; document source availability.
- [ ] **A02** Define canonical metric types, statuses, telemetry coverage and source attribution rules.
- [ ] **A03** Add migrations, indexes, bounded event/usage collector and deterministic deduplication.
- [ ] **A04** Record task queue/start/end/wait/review/attempt lifecycle without altering execution behavior.
- [ ] **A05** Add OpenCode session/message usage adapter, reconciliation and version guards.
- [ ] **A06** Implement analytics query service with filters, aggregates and missing-data semantics.
- [ ] **A07** Add pinned global sidebar Analytics nav, routing and project-scoped shortcut.
- [ ] **A08** Implement reusable filter bar, KPI components, time series and sortable project table.
- [ ] **A09** Implement task details/drilldown, loading/empty/partial/error states, exports.
- [ ] **A10** Implement categorized error records, retries, recovery, reliability tab.
- [ ] **A11** Implement structured interaction capture, question grouping and autonomy metrics.
- [ ] **A12** Add settings/retention/delete, privacy protections, tests, performance benchmarks.
- [ ] **A13** Run production build and complete regression/security/usability review; report unverified items.

**Task sizing:** A01–A02 should happen before visual work. A03–A06 are critical path for trustworthy numbers. A07–A09 can be implemented in parallel with backend analytics if data contracts are agreed first. A10–A11 depend on structured event availability. Avoid presenting all fields as supported when source data is missing.

## 15. Suggested task breakdown into independently releasable PRs

1. **Contracts + DB** — metric definitions, migrations, test fixtures, usage mapping.
2. **Collection + reconciliation** — runtime adapter, event dedup, restart recovery, source-quality labels.
3. **Analytics MVP UI** — navigation, filters, KPIs, charts, project table, task drilldowns.
4. **Reliability** — error taxonomy/fingerprints, retries, recovery and failure attribution.
5. **Interactions** — structured question/permission events, autonomy analysis, grouping.
6. **Hardening** — accessibility, privacy controls, export, scale tests and cross-platform production validation.

Each PR must include tests for new calculations, pass existing checks, and avoid changing unrelated task orchestration behavior.

## 16. Open questions to resolve by inspecting the actual repo

These should be **investigated by the implementing agent**, not used as a reason to block initial planning:

1. What is Solayge's existing database, routing, UI component/chart stack, and analytics-friendly storage API?
2. Is the task-to-OpenCode session mapping explicit today? Can one task have multiple sessions/subagents/review loops?
3. Does `Review passed` belong to the task outcome semantics, and when can the status change after initial completion?
4. Which OpenCode version/provider adapters are installed, and which message/token/event shapes are returned in real runs?
5. Do provider messages already expose costs, or is a versioned rate table needed for some backends?
6. Are questions represented as structured UI events or only in conversation text? Are they blocking?
7. How are projects identified across folder renames, Git worktrees and project deletion?
8. Do existing task counters include deleted/archived tasks, and what is the retention policy?
9. Which platforms and window sizes must be validated in the current release?
10. Should Analytics be accessible to remote paired devices later? If so, enforce existing user/device authorization before exposing even aggregated metadata.

## 17. Ready-to-use implementation instruction

Copy this *short* brief into a coding agent together with this Markdown file:

> Implement the Solayge Analytics feature according to `Solayge_Analytics_Implementation_Plan.md`. First inspect the repository, task lifecycle, OpenCode adapter, actual schema, and existing UI components; adjust unverified details in the plan to match the code. Prioritize accurate collection and idempotent task/session usage attribution before charts. Deliver Phase 0 and Phase 1 first, with usable Analytics navigation, project/task metrics, token and cost breakdown, filters, drill-down, and partial-data states. Do not mislabel estimated API usage as actual billing or silently treat missing usage as zero. Preserve the app's dark visual language and existing workflows. Add regression/security tests, run production builds, and report source limitations. Proceed to later phases only after MVP validation or as specifically requested.

## 18. Research sources

Research checked October 8, 2026. Links document **external guidance/capability**, while implementation decisions are recommendations for Solayge. The internal repo was not audited as part of this UX research.

1. Nielsen Norman Group — [Dashboards: Making Charts and Graphs Easier to Understand](https://www.nngroup.com/articles/dashboards-preattentive/). At-a-glance priorities, perceptual chart choices, distinction between operational and analytical dashboards.
2. Nielsen Norman Group — [Data Tables: Four Major User Tasks](https://www.nngroup.com/articles/data-tables/). Sorting, filtering, comparison, finding and action in data tables.
3. IBM Carbon Design System — [Simple charts](https://www.carbondesignsystem.com/building-blocks/data-visualization/simple-charts). Chart types for category comparison and timeseries.
4. IBM Carbon Design System — [Axes and labels](https://www.carbondesignsystem.com/building-blocks/data-visualization/axes-and-labels/). Start bar axes at zero, label gaps, maintain consistent time intervals.
5. IBM Carbon Design System — [Legends](https://www.carbondesignsystem.com/building-blocks/data-visualization/legends). Legends and non-color differentiation; accessible chart semantics.
6. W3C WAI — [WCAG 2.2](https://www.w3.org/TR/WCAG22/) and [What's New in WCAG 2.2](https://www.w3.org/WAI/standards-guidelines/wcag/new-in-22/). Focus and target size guidance; non-text contrast and other AA criteria.
7. [Atlassian Design System — Components](https://atlassian.design/components). Usable design precedents: tabs, tables, loading and empty states.
8. OpenCode — [Server API documentation](https://opencode.ai/docs/server/) and [type-safe SDK](https://docs.opencode.ai/docs/sdk/). Session/message/event endpoints, version-dependent integration points.
9. OpenCode — [Go subscription details](https://docs.opencode.ai/docs/go/). Usage tiers and model rate cards; subscription usage is not automatically actual billing.
10. OpenCode — [Plugin events and hooks](https://opencode.ai/docs/plugins/). Session, message, permission and tool lifecycle hooks.
11. OpenCode — [Session implementation and usage normalization](https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/session/session.ts). Implementation reference, **not** stable integration API. Confirm actual installed version and use published SDK/schema where possible.
12. Lucide — [`chart-no-axes-combined`](https://lucide.dev/icons/chart-no-axes-combined). Recommended Analytics navigation icon.
13. IBM Carbon Design System — [Accessibility for developers](https://www.carbondesignsystem.com/building-blocks/foundations/accessibility/developers). Semantic controls and keyboard-first navigation.

---

**Definition of success:** Solayge should explain *what agents accomplished, how much resources they consumed, what went wrong, and where human intervention was required*—without requiring a cloud service, exposing private code/prompts, or displaying misleading cost and performance statistics.
