import type { Page } from "@playwright/test";
import type { Snapshot } from "../src/types";
import {
  makeProject,
  makeSettings,
  makeSnapshot,
  makeTask,
  makeTools,
} from "../src/test/fixtures";

/** A recorded Tauri command call, readable from `window.__MOCK_CALLS__`. */
export interface MockCall {
  cmd: string;
  args: Record<string, unknown>;
}

export interface TauriMockOptions {
  snapshot?: Snapshot;
  /** File paths the OS picker returns for `plugin:dialog|open`. */
  pickedFiles?: string[];
  /** Extra canned responses, keyed by Tauri command name. */
  responses?: Record<string, unknown>;
}

/**
 * Install a fake `window.__TAURI_INTERNALS__` before the app boots, so the
 * frontend runs without the Rust backend. Every `invoke` is recorded in
 * `window.__MOCK_CALLS__` for assertions.
 */
export async function installTauriMock(
  page: Page,
  options: TauriMockOptions = {},
): Promise<void> {
  await page.addInitScript(
    (payload: {
      snapshot: Snapshot;
      tools: unknown;
      pickedFiles: string[] | undefined;
      responses: Record<string, unknown>;
    }) => {
      const calls: { cmd: string; args: Record<string, unknown> }[] = [];
      (window as any).__MOCK_CALLS__ = calls;

      const responses: Record<string, unknown> = {
        get_snapshot: payload.snapshot,
        environment_check: payload.tools,
        project_status: {
          is_repo: true,
          branch: "main",
          dirty: false,
          changed_files: [],
          ahead: 0,
          behind: 0,
        },
        project_remote: null,
        project_worktrees: [],
        project_branches: [],
        get_project_secrets: [],
        get_prompt_history: [],
        get_cache_stats: {
          prompt_count: 0,
          oldest_prompt: null,
          log_count: 0,
          log_bytes: 0,
          retention_days: 0,
        },
        get_error_log: "",
        get_task_log: "",
        get_review_log: "",
        list_models: [],
        // The file picker returns whatever the test supplied.
        "plugin:dialog|open": payload.pickedFiles ?? null,
        "plugin:event|listen": 1,
        "plugin:event|unlisten": null,
        ...payload.responses,
      };

      // Commands the real backend answers with a fresh `Snapshot`; returning one
      // keeps the UI consistent after a mutation.
      const snapshotCommands = new Set([
        "add_project",
        "remove_project",
        "create_tasks",
        "update_task",
        "delete_task",
        "restore_task",
        "start_task_now",
        "cancel_task",
        "retry_task",
        "retry_review",
        "answer_task",
        "remove_task_worktree",
        "clear_finished",
        "execute_project",
        "set_concurrency",
        "set_project_default_profile",
        "reorder_projects",
        "update_project_config",
        "update_settings",
      ]);

      (window as any).__TAURI_INTERNALS__ = {
        invoke: async (cmd: string, args: Record<string, unknown>) => {
          calls.push({ cmd, args });
          if (cmd in responses) return responses[cmd];
          return snapshotCommands.has(cmd) ? payload.snapshot : null;
        },
        transformCallback: (cb: unknown) => {
          const id = Math.floor(Math.random() * 1_000_000);
          (window as any).__callbacks = (window as any).__callbacks ?? {};
          (window as any).__callbacks[id] = cb;
          return id;
        },
        unregisterCallback: () => {},
        convertFileSrc: (path: string) => path,
      };
      // `unlisten` reaches into this namespace directly.
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => {},
      };
    },
    {
      snapshot: options.snapshot ?? defaultSnapshot(),
      tools: makeTools(),
      pickedFiles: options.pickedFiles,
      responses: options.responses ?? {},
    },
  );
}

/** Read the recorded calls from the page. */
export async function mockCalls(page: Page): Promise<MockCall[]> {
  return page.evaluate(() => (window as any).__MOCK_CALLS__ ?? []);
}

/** Find the most recent call to a command. */
export async function lastCall(
  page: Page,
  cmd: string,
): Promise<MockCall | undefined> {
  const calls = (await mockCalls(page)).filter((c) => c.cmd === cmd);
  return calls[calls.length - 1];
}

function defaultSnapshot(): Snapshot {
  const project = makeProject();
  return makeSnapshot({
    projects: [project],
    tasks: [
      makeTask({ id: "task-1", title: "Set up CI", status: "draft" }),
      makeTask({ id: "task-2", title: "Add login form", status: "succeeded" }),
    ],
    settings: makeSettings(),
  });
}
