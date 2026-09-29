import { useEffect, useId, useState } from "react";
import type { AgentConfig, EnvironmentStatus, Provider, ReviewMode } from "../types";
import {
  EDITORS,
  MODEL_SUGGESTIONS,
  PROVIDERS,
  REVIEW_MODES,
} from "../lib/providers";
import { cachedModels, loadModels } from "../lib/models";
import { Icon } from "./Icons";
import { Field, SectionLabel } from "./Field";

// Re-exported so existing `from "./AgentConfigForm"` imports keep working.
export { Field, SectionLabel } from "./Field";

export function ProviderSelect({
  value,
  onChange,
  disabled,
  inheritLabel,
}: {
  value?: Provider | null;
  onChange: (p: Provider | null) => void;
  disabled?: boolean;
  inheritLabel?: string;
}) {
  return (
    <select
      className="select"
      disabled={disabled}
      value={value ?? ""}
      onChange={(e) => onChange((e.target.value || null) as Provider | null)}
    >
      {inheritLabel !== undefined && <option value="">{inheritLabel}</option>}
      {PROVIDERS.map((p) => (
        <option key={p.id} value={p.id}>
          {p.label}
        </option>
      ))}
    </select>
  );
}

export function ModelField({
  provider,
  value,
  onChange,
  disabled,
  placeholder = "Provider default",
}: {
  provider?: Provider | null;
  value?: string | null;
  onChange: (m: string | null) => void;
  disabled?: boolean;
  placeholder?: string;
}) {
  const p: Provider = provider ?? "opencode";
  const listId = useId();
  const [models, setModels] = useState<string[]>(
    () => cachedModels(p) ?? MODEL_SUGGESTIONS[p] ?? [],
  );
  const [loading, setLoading] = useState(false);

  // Fetch the provider's real model list once per session (and on refresh).
  useEffect(() => {
    const hit = cachedModels(p);
    if (hit) {
      setModels(hit);
      return;
    }
    let alive = true;
    setLoading(true);
    loadModels(p, false)
      .then((m) => alive && setModels(m))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [p]);

  function refresh() {
    setLoading(true);
    loadModels(p, true)
      .then(setModels)
      .finally(() => setLoading(false));
  }

  return (
    <>
      <div className="flex items-stretch gap-1.5">
        <input
          className="input"
          list={listId}
          disabled={disabled}
          placeholder={placeholder}
          value={value ?? ""}
          onChange={(e) => onChange(e.target.value || null)}
        />
        <button
          type="button"
          className="btn btn-ghost shrink-0 !px-2"
          onClick={refresh}
          disabled={disabled || loading}
          title={
            loading
              ? "Fetching models…"
              : `Fetch models from ${p} (${models.length} available)`
          }
          aria-label="Fetch models"
        >
          <Icon
            name="refresh"
            className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`}
          />
        </button>
      </div>
      <datalist id={listId}>
        {models.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>
      {(loading || models.length === 0) && (
        <span className="mt-1 block text-[10.5px] text-ink-subtle">
          {loading
            ? "Fetching models from the provider…"
            : "No list from this provider — type a model id."}
        </span>
      )}
    </>
  );
}

/**
 * The provider / model / backup / code-review / editor block, shared by the
 * account settings and the per-project settings.
 */
export function AgentConfigForm({
  value,
  onChange,
  disabled,
  inherit,
  tools,
  effectiveProvider,
}: {
  value: AgentConfig;
  onChange: (patch: Partial<AgentConfig>) => void;
  disabled?: boolean;
  /** Show "Use account default" options (for per-project config). */
  inherit?: boolean;
  /** Preflight results, to warn when the selected CLI is missing. */
  tools?: EnvironmentStatus | null;
  /** The provider actually used (value may be null = inherit). */
  effectiveProvider?: Provider | null;
}) {
  const inheritLabel = inherit ? "Use account default" : undefined;
  const modeValue = value.review_mode ?? "";
  const providerTool = tools
    ? tools.providers.find(
        (p) => p.provider === (effectiveProvider ?? value.provider ?? "opencode"),
      )
    : undefined;
  return (
    <div className="space-y-5">
      <section className="space-y-2">
        <SectionLabel icon="layers">Agent</SectionLabel>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Provider">
            <ProviderSelect
              value={value.provider}
              onChange={(provider) => onChange({ provider })}
              disabled={disabled}
              inheritLabel={inheritLabel}
            />
          </Field>
          <Field label="Model">
            <ModelField
              provider={value.provider}
              value={value.model}
              onChange={(model) => onChange({ model })}
              disabled={disabled}
            />
          </Field>
        </div>
        {providerTool && (!providerTool.found || providerTool.note) && (
          <p
            className={`text-[11px] leading-relaxed ${
              providerTool.found ? "text-warning" : "text-danger"
            }`}
          >
            {providerTool.found
              ? providerTool.note
              : `"${providerTool.command}" was not found on PATH — tasks will fail to launch. Install it, or pick another provider.`}
          </p>
        )}
      </section>

      <section className="space-y-2">
        <SectionLabel icon="retry">Backup on failure</SectionLabel>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Provider" hint="Used once if the agent exits non-zero.">
            <ProviderSelect
              value={value.fallback_provider}
              onChange={(fallback_provider) => onChange({ fallback_provider })}
              disabled={disabled}
              inheritLabel={inheritLabel ?? "None"}
            />
          </Field>
          <Field label="Model">
            <ModelField
              provider={value.fallback_provider}
              value={value.fallback_model}
              onChange={(fallback_model) => onChange({ fallback_model })}
              disabled={disabled}
              placeholder="Provider default"
            />
          </Field>
        </div>
      </section>

      <section className="space-y-2">
        <SectionLabel icon="check">Auto code review after each step</SectionLabel>
        <Field label="When a task succeeds">
          <select
            className="select"
            disabled={disabled}
            value={modeValue}
            onChange={(e) =>
              onChange({
                review_mode: (e.target.value || null) as ReviewMode | null,
              })
            }
          >
            {inherit && <option value="">Use account default</option>}
            {REVIEW_MODES.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
        </Field>
        <p className="text-[11px] leading-relaxed text-ink-subtle">
          {REVIEW_MODES.find((m) => m.id === (value.review_mode ?? "off"))?.desc}
        </p>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Reviewer provider">
            <ProviderSelect
              value={value.review_provider}
              onChange={(review_provider) => onChange({ review_provider })}
              disabled={disabled}
              inheritLabel={inheritLabel}
            />
          </Field>
          <Field label="Reviewer model">
            <ModelField
              provider={value.review_provider ?? value.provider}
              value={value.review_model}
              onChange={(review_model) => onChange({ review_model })}
              disabled={disabled}
            />
          </Field>
        </div>
      </section>

      <section className="space-y-2">
        <SectionLabel icon="code">Editor</SectionLabel>
        <Field label="Open project in">
          <select
            className="select"
            disabled={disabled}
            value={value.editor ?? (inherit ? "" : "system")}
            onChange={(e) =>
              onChange({ editor: e.target.value ? e.target.value : null })
            }
          >
            {inherit && <option value="">Use account default</option>}
            {EDITORS.map((ed) => (
              <option key={ed.id} value={ed.id}>
                {ed.label}
              </option>
            ))}
          </select>
        </Field>
        <p className="text-[11px] text-ink-subtle">
          Falls back to the system handler if the editor CLI isn't installed.
        </p>
      </section>
    </div>
  );
}
