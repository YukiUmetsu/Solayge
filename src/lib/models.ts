import { api } from "../api";
import type { Provider } from "../types";
import { MODEL_SUGGESTIONS } from "./providers";

/**
 * Provider model lists are fetched from the provider CLI (e.g. `opencode
 * models`, `cursor-agent --list-models`) and cached for the session. When a
 * provider has no listing command, we fall back to the curated suggestions.
 */
const cache = new Map<Provider, string[]>();
const inflight = new Map<Provider, Promise<string[]>>();

export function cachedModels(provider: Provider): string[] | undefined {
  return cache.get(provider);
}

export async function loadModels(
  provider: Provider,
  force = false,
): Promise<string[]> {
  if (!force) {
    const hit = cache.get(provider);
    if (hit) return hit;
    const pending = inflight.get(provider);
    if (pending) return pending;
  }
  const promise = api
    .listModels(provider, force)
    .then((models) =>
      models.length ? models : (MODEL_SUGGESTIONS[provider] ?? []),
    )
    .catch(() => MODEL_SUGGESTIONS[provider] ?? [])
    .then((models) => {
      cache.set(provider, models);
      inflight.delete(provider);
      return models;
    });
  inflight.set(provider, promise);
  return promise;
}
