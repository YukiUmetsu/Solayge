import { afterEach, describe, expect, it, vi } from "vitest";
import { newId } from "./id";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("newId", () => {
  it("returns unique ids", () => {
    const ids = new Set(Array.from({ length: 50 }, () => newId("sk")));
    expect(ids.size).toBe(50);
  });

  it("falls back to a prefixed id when randomUUID is unavailable", () => {
    vi.stubGlobal("crypto", {
      randomUUID: () => {
        throw new Error("unavailable");
      },
    });
    const id = newId("sk");
    expect(id.startsWith("sk-")).toBe(true);
  });
});
