import { describe, expect, it } from "vitest";

import capabilities from "../src-tauri/capabilities/default.json";

/**
 * The QR flow calls `checkPermissions`, `requestPermissions`, `scan`, and
 * `cancel` from `@tauri-apps/plugin-barcode-scanner`. Each maps to a plugin
 * command that Tauri's ACL only exposes when the matching permission is granted,
 * so a missing entry silently degrades the scanner to the manual fallback.
 */
describe("mobile capabilities", () => {
  it("grants every barcode-scanner command the QR flow calls", () => {
    const permissions: string[] = capabilities.permissions;
    for (const required of [
      "barcode-scanner:allow-scan",
      "barcode-scanner:allow-cancel",
      "barcode-scanner:allow-check-permissions",
      "barcode-scanner:allow-request-permissions",
    ]) {
      expect(permissions).toContain(required);
    }
  });
});
