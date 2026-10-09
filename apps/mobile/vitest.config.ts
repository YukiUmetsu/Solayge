import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Component/interaction tests for the mobile UI run in jsdom with the Tauri
// IPC bridge never loaded: screens depend only on the in-memory RemoteClient.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    exclude: ["node_modules", "dist", "src-tauri"],
  },
});
