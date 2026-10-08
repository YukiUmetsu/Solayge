import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Frontend unit/component tests. The Tauri IPC bridge is mocked per test (see
// `src/test/setup.ts` and the individual test files), so the components run in
// plain jsdom.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    exclude: ["node_modules", "dist", "e2e", "src-tauri"],
    coverage: {
      provider: "v8",
      reportsDirectory: "coverage",
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.{test,spec}.{ts,tsx}",
        "src/test/**",
        "src/**/*.d.ts",
        "src/main.tsx",
      ],
    },
  },
});
