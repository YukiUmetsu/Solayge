import { describe, expect, it, vi } from "vitest";

// A mutable holder so each test can install a fresh mock. Reusing one hoisted
// `vi.fn` across tests trips Vitest's unhandled-rejection detector when a mock
// rejects, even though `loadModels` catches it.
const { api } = vi.hoisted(() => ({
  api: {
    listModels: vi.fn() as (
      provider: string,
      force?: boolean,
    ) => Promise<string[]>,
  },
}));
vi.mock("../api", () => ({ api }));

import { cachedModels, loadModels } from "./models";
import { MODEL_SUGGESTIONS } from "./providers";

describe("loadModels", () => {
  it("caches the fetched list", async () => {
    const listModels = vi.fn().mockResolvedValue(["m1", "m2"]);
    api.listModels = listModels;

    await expect(loadModels("opencode")).resolves.toEqual(["m1", "m2"]);
    expect(cachedModels("opencode")).toEqual(["m1", "m2"]);

    // A second call is served from the cache.
    await expect(loadModels("opencode")).resolves.toEqual(["m1", "m2"]);
    expect(listModels).toHaveBeenCalledTimes(1);
  });

  it("falls back to curated suggestions when the provider returns none", async () => {
    api.listModels = vi.fn().mockResolvedValue([]);
    await expect(loadModels("codex")).resolves.toEqual(MODEL_SUGGESTIONS.codex);
  });

  it("falls back to suggestions when the provider errors", async () => {
    api.listModels = vi.fn().mockImplementation(() => Promise.reject(new Error("boom")));
    await expect(loadModels("claude")).resolves.toEqual(MODEL_SUGGESTIONS.claude);
  });

  it("refetches when forced", async () => {
    const listModels = vi.fn().mockResolvedValue(["first"]);
    api.listModels = listModels;
    await loadModels("cursor");

    listModels.mockResolvedValue(["second"]);
    await expect(loadModels("cursor", true)).resolves.toEqual(["second"]);
    expect(listModels).toHaveBeenCalledTimes(2);
  });
});
