import { expect, test } from "@playwright/test";
import { installTauriMock, lastCall, mockCalls } from "./tauri-mock";

test.describe("workspace", () => {
  test("loads the project and its tasks", async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");

    await expect(
      page.getByRole("heading", { name: "solayge-project" }),
    ).toBeVisible();
    await expect(page.getByText("Set up CI")).toBeVisible();
    await expect(page.getByText("Add login form")).toBeVisible();
  });
});

test.describe("project settings — prompt tab", () => {
  test("a new prompt is editable and saved", async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");

    await page.getByRole("button", { name: "Project settings" }).click();
    const dialog = page.getByRole("dialog", { name: /settings/i });
    await expect(dialog).toBeVisible();

    await dialog.getByRole("button", { name: "Prompt" }).click();

    const textarea = dialog.getByPlaceholder(/this project uses pnpm/i);
    await expect(textarea).toBeEnabled();
    await textarea.fill("This project uses pnpm. Run tests on {{current_branch}}.");

    // Typing turns the prompt on without touching the Enabled checkbox.
    await expect(dialog.getByRole("checkbox")).toBeChecked();

    await dialog.getByRole("combobox").selectOption("suffix");
    await dialog.getByRole("button", { name: "Save" }).click();

    await expect
      .poll(async () => Boolean(await lastCall(page, "update_project_config")))
      .toBe(true);
    const call = await lastCall(page, "update_project_config");
    const config = call!.args.config as {
      system_prompt: { text: string; position: string; enabled: boolean };
    };
    expect(config.system_prompt).toEqual({
      text: "This project uses pnpm. Run tests on {{current_branch}}.",
      position: "suffix",
      enabled: true,
    });
  });
});

test.describe("new task — attachments", () => {
  test("attaches multiple files and sends them with the task", async ({
    page,
  }) => {
    await installTauriMock(page, {
      pickedFiles: ["/tmp/diagram.png", "/tmp/spec.md"],
    });
    await page.goto("/");

    await page.locator('button[title="New task"]').click();
    const dialog = page.getByRole("dialog", { name: "New task" });
    await expect(dialog).toBeVisible();

    await dialog
      .getByPlaceholder(/full instruction/i)
      .fill("Use the attached files to update the docs.");

    await dialog.getByRole("button", { name: /attach files/i }).click();
    await expect(dialog.getByText("diagram.png", { exact: true })).toBeVisible();
    await expect(dialog.getByText("spec.md", { exact: true })).toBeVisible();

    await dialog.getByRole("button", { name: /create task/i }).click();

    await expect
      .poll(async () => Boolean(await lastCall(page, "create_tasks")))
      .toBe(true);
    const call = await lastCall(page, "create_tasks");
    const tasks = call!.args.tasks as { attachments: string[]; prompt: string }[];
    expect(tasks).toHaveLength(1);
    expect(tasks[0].attachments).toEqual(["/tmp/diagram.png", "/tmp/spec.md"]);
    expect(tasks[0].prompt).toBe("Use the attached files to update the docs.");
  });

  test("removing an attachment updates the payload", async ({ page }) => {
    await installTauriMock(page, {
      pickedFiles: ["/tmp/a.txt", "/tmp/b.txt"],
    });
    await page.goto("/");

    await page.locator('button[title="New task"]').click();
    const dialog = page.getByRole("dialog", { name: "New task" });
    await dialog.getByPlaceholder(/full instruction/i).fill("Hi");
    await dialog.getByRole("button", { name: /attach files/i }).click();
    await expect(dialog.getByText("a.txt", { exact: true })).toBeVisible();

    await dialog.getByRole("button", { name: /remove a\.txt/i }).click();
    await expect(dialog.getByText("a.txt", { exact: true })).toHaveCount(0);

    await dialog.getByRole("button", { name: /create task/i }).click();
    await expect
      .poll(async () => Boolean(await lastCall(page, "create_tasks")))
      .toBe(true);
    const tasks = (await lastCall(page, "create_tasks"))!.args.tasks as {
      attachments: string[];
    }[];
    expect(tasks[0].attachments).toEqual(["/tmp/b.txt"]);
  });
});

test.describe("task execution", () => {
  test("Execute runs the project", async ({ page }) => {
    await installTauriMock(page);
    await page.goto("/");

    await page.getByRole("button", { name: /^execute/i }).click();

    await expect
      .poll(async () =>
        (await mockCalls(page)).some((c) => c.cmd === "execute_project"),
      )
      .toBe(true);
  });
});

test.describe("analytics", () => {
  test("the sidebar opens cross-project analytics with metrics", async ({
    page,
  }) => {
    await installTauriMock(page, {
      responses: {
        get_analytics_overview: {
          has_data: true,
          tasks_total: 5,
          tasks_completed: 4,
          tasks_failed: 1,
          tasks_canceled: 0,
          tasks_blocked: 0,
          tasks_interrupted: 0,
          success_rate: 0.8,
          success_sample: 5,
          avg_active_ms: 60000,
          median_active_ms: 60000,
          duration_sample: 5,
          avg_wait_ms: null,
          total_tokens: 1500000,
          input_tokens: 100000,
          output_tokens: 200000,
          reasoning_tokens: 0,
          cache_read_tokens: 1200000,
          cache_write_tokens: 0,
          reported_cost_micros: 730000,
          inference_count: 10,
          usage_coverage_runs: 5,
          usage_runs_with_data: 4,
          currency: "USD",
          cost_basis: "provider_reported_not_a_bill",
          runs_with_errors: 2,
          error_events: 2,
          retry_rate: 0.2,
          first_pass_success_rate: 0.75,
          autonomy_rate: 0.6,
          autonomy_sample: 5,
          generated_at_ms: 1700000000000,
        },
        get_analytics_projects: [
          {
            project_path: "/tmp/solayge-project",
            project_name: "solayge-project",
            tasks_total: 5,
            tasks_completed: 4,
            tasks_failed: 1,
            success_rate: 0.8,
            success_sample: 5,
            avg_active_ms: 60000,
            median_active_ms: 60000,
            total_tokens: 1500000,
            reported_cost_micros: 730000,
            inference_count: 10,
          },
        ],
        get_analytics_reliability: {
          summary: {
            has_data: true,
            tasks_failed: 1,
            failed_sample: 5,
            task_failure_rate: 0.2,
            runs_with_errors: 2,
            observed_runs: 5,
            runs_with_errors_rate: 0.4,
            error_events: 2,
            recoverable_errors: 2,
            recovered_errors: 1,
            recovery_rate: 0.5,
            retry_tasks: 1,
            attempt_tasks: 5,
            retry_rate: 0.2,
            first_pass_success: 3,
            first_pass_sample: 5,
            first_pass_success_rate: 0.6,
          },
          categories: [
            {
              category: "build",
              occurrences: 2,
              tasks: 1,
              recovered: 1,
              recoverable: 2,
              recovery_rate: 0.5,
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
              recovered: 1,
              recoverable: 2,
              unresolved: 0,
              first_seen_ms: 1700000000000,
              last_seen_ms: 1700000100000,
              sample: null,
              task_ids: [],
            },
          ],
          stages: [{ name: "run", occurrences: 2, tasks: 1, recovered: 1 }],
          tools: [],
          series: [{ bucket_ms: 1700000000000, errors: 2, tasks_failed: 1 }],
        },
        get_analytics_interactions: {
          summary: {
            has_data: true,
            questions: 2,
            permissions: 1,
            manual_confirmations: 0,
            blocking_total: 3,
            resolved: 2,
            unresolved: 1,
            total_wait_ms: 120000,
            avg_wait_ms: 60000,
            median_wait_ms: 60000,
            p95_wait_ms: 120000,
            autonomy_rate: 0.6,
            autonomy_autonomous: 3,
            autonomy_sample: 5,
            first_pass_success_rate: 0.6,
          },
          groups: [
            {
              fingerprint: "question:environment",
              kind: "question",
              category: "environment_setup",
              occurrences: 2,
              tasks: 2,
              blocking: 2,
              unresolved: 1,
              avg_wait_ms: 60000,
              last_seen_ms: 1700000100000,
              task_ids: [],
            },
          ],
          categories: [
            {
              category: "environment_setup",
              kind: "question",
              occurrences: 2,
              tasks: 2,
              unresolved: 1,
              avg_wait_ms: 60000,
            },
          ],
          recommendations: [],
        },
      },
    });
    await page.goto("/");

    await page.getByRole("button", { name: "Analytics", exact: true }).click();

    await expect(page.getByRole("heading", { name: "Analytics" })).toBeVisible();
    await expect(page.getByText("Tasks completed")).toBeVisible();
    await expect(page.getByText("~$0.73").first()).toBeVisible();

    // Reliability shows categorized errors, not raw text.
    await page.getByRole("tab", { name: /reliability/i }).click();
    await expect(page.getByText("Task failure rate")).toBeVisible();
    await expect(page.getByText("Build · TS2322")).toBeVisible();

    // Interactions shows recurring-question grouping and autonomy.
    await page.getByRole("tab", { name: /interactions/i }).click();
    await expect(page.getByText("Human wait")).toBeVisible();
    await expect(page.getByText("question:environment")).toBeVisible();

    // Selecting a project from the sidebar returns to the workspace rather than
    // leaving the Analytics page up.
    await page
      .locator("div.cursor-pointer", { hasText: "solayge-project" })
      .first()
      .click();
    await expect(page.getByRole("heading", { name: "Analytics" })).toBeHidden();
  });
});
