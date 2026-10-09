import { describe, expect, it } from "vitest";

/**
 * The UI must not reach an OS capability directly. Screens, presentational
 * components, and the client seam talk only to `RemoteClient`; the sole
 * exception is the Tauri-backed barcode scanner inside the pairing onboarding
 * (`src/AddDesktopFlow.tsx` / `src/pairing.ts`), which is not scanned here.
 *
 * Source text is loaded with Vite's `import.meta.glob` so the test needs no
 * filesystem access.
 */
const sources = import.meta.glob("./{screens,components,client}/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const entries = Object.entries(sources).filter(
  ([path]) => !/\.test\.(ts|tsx)$/.test(path) && !/\.d\.ts$/.test(path),
);

describe("UI capability boundary", () => {
  it("has sources to scan", () => {
    expect(entries.length).toBeGreaterThan(0);
  });

  it("never imports Tauri APIs or calls invoke from screens, components, or the client", () => {
    const offenders = entries
      .filter(([, content]) => /@tauri-apps/.test(content) || /\binvoke\s*\(/.test(content))
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });

  it("does not re-declare shared protocol DTOs privately", () => {
    const offenders = entries
      .filter(([, content]) =>
        /interface\s+(SessionSummary|ApprovalSummary|ProjectSummary)\b/.test(content),
      )
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });
});
