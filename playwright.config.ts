import { defineConfig, devices } from "@playwright/test";

const PORT = 4317;

/**
 * End-to-end tests run the real Vite build of the frontend against a mocked
 * Tauri IPC bridge (`e2e/tauri-mock.ts`), so they run anywhere Chromium does —
 * no Rust toolchain, git, provider CLIs, or desktop WebView required.
 *
 * A dedicated port keeps the suite off the `tauri dev` port (1420), where a
 * developer may already be running a different app.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `pnpm exec vite --port ${PORT}`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
