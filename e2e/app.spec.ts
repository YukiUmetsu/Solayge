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
