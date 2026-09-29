import { useEffect, useRef, useState } from "react";
import type {
  AgentConfig,
  CacheStats,
  CommandTemplates,
  PermissionProfile,
  SecretStore,
  Settings,
  Snapshot,
} from "../types";
import { api } from "../api";
import { clock } from "../lib/format";
import { DEFAULT_COMMANDS, SECRET_STORES } from "../lib/providers";
import { useTheme, type ThemePref } from "../theme";
import { AgentConfigForm, Field, SectionLabel } from "./AgentConfigForm";
import { SaveButton, SavedPill, type SaveState } from "./SaveButton";
import { Modal } from "./Modal";
import { Icon } from "./Icons";

const RETENTION_OPTIONS: { value: number; label: string }[] = [
  { value: 7, label: "7 days" },
  { value: 30, label: "30 days" },
  { value: 90, label: "90 days" },
  { value: 365, label: "1 year" },
  { value: 0, label: "Keep forever" },
];

const THEME_OPTIONS: ThemePref[] = ["system", "light", "dark"];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function SettingsModal({
  snapshot,
  onClose,
  onSaved,
}: {
  snapshot: Snapshot;
  onClose: () => void;
  onSaved: (s: Snapshot) => void;
}) {
  const { pref, setPref } = useTheme();
  const settings = snapshot.settings;
  const [stats, setStats] = useState<CacheStats | null>(null);
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [flash, setFlash] = useState<string | null>(null);
  const flashTimer = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  function flashSaved(key: string) {
    setFlash(key);
    if (flashTimer.current) window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(null), 2000);
  }

  const [agent, setAgent] = useState<AgentConfig>(() => ({
    provider: settings.provider ?? null,
    model: settings.model ?? null,
    fallback_provider: settings.fallback_provider ?? null,
    fallback_model: settings.fallback_model ?? null,
    review_provider: settings.review_provider ?? null,
    review_model: settings.review_model ?? null,
    review_mode: settings.review_mode ?? "off",
    editor: settings.editor ?? null,
  }));
  const [templates, setTemplates] = useState<CommandTemplates>(
    () => settings.command_templates ?? DEFAULT_COMMANDS,
  );

  const refreshStats = () =>
    api
      .cacheStats()
      .then(setStats)
      .catch(() => {});

  useEffect(() => {
    void refreshStats();
  }, []);

  async function persist(patch: Partial<Settings>, key: string) {
    setBusy(true);
    setError(null);
    try {
      onSaved(await api.updateSettings({ ...settings, ...patch }));
      await refreshStats();
      flashSaved(key);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function saveAgent() {
    setSaveState("saving");
    setError(null);
    try {
      onSaved(
        await api.updateSettings({
          ...settings,
          ...agent,
          review_mode: agent.review_mode ?? "off",
          command_templates: templates,
        }),
      );
      setDirty(false);
      setSaveState("saved");
      window.setTimeout(() => setSaveState("idle"), 2000);
    } catch (e) {
      setError(String(e));
      setSaveState("idle");
    }
  }

  function patchAgent(p: Partial<AgentConfig>) {
    setAgent((a) => ({ ...a, ...p }));
    setDirty(true);
  }

  function patchTemplate(key: keyof CommandTemplates, value: string) {
    setTemplates((t) => ({ ...t, [key]: value }));
    setDirty(true);
  }

  async function clear(prompts: boolean, logs: boolean) {
    setBusy(true);
    setError(null);
    try {
      setStats(await api.clearCache(prompts, logs));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Settings"
      subtitle="Appearance, agent defaults, backup, code review, and the local cache."
      onClose={onClose}
      width="max-w-2xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose}>
            Done
          </button>
          <SaveButton
            dirty={dirty}
            state={saveState}
            onClick={saveAgent}
            label="Save agent defaults"
            disabled={busy}
          />
        </>
      }
    >
      <div className="space-y-6">
        <section className="space-y-2">
          <SectionLabel icon="sun">Appearance</SectionLabel>
          <div className="flex rounded-lg border border-line p-0.5">
            {THEME_OPTIONS.map((p) => (
              <button
                key={p}
                onClick={() => setPref(p)}
                className={`flex-1 rounded-md px-3 py-1.5 text-[12px] font-medium capitalize transition ${
                  pref === p
                    ? "bg-accent-soft text-accent-text"
                    : "text-ink-muted hover:bg-hover"
                }`}
              >
                {p}
              </button>
            ))}
          </div>
        </section>

        <section className="space-y-2">
          <div className="flex items-center justify-between">
            <SectionLabel icon="layers">Default permissions</SectionLabel>
            <SavedPill show={flash === "permissions"} />
          </div>
          <select
            className="select"
            disabled={busy}
            value={settings.default_profile ?? "none"}
            onChange={(e) =>
              void persist(
                {
                  default_profile:
                    e.target.value === "none"
                      ? null
                      : (e.target.value as PermissionProfile),
                },
                "permissions",
              )
            }
          >
            <option value="none">No default (each task chooses)</option>
            <option value="autonomous">Autonomous (pre-granted)</option>
            <option value="supervised">Supervised (ask, notify)</option>
            <option value="readonly">Read-only</option>
          </select>
          <p className="text-[11px] text-ink-subtle">
            Used for new tasks in projects without their own default.
          </p>
        </section>

        <section className="space-y-4">
          <SectionLabel icon="sparkles">Agent defaults</SectionLabel>
          <p className="text-[11px] text-ink-subtle">
            Projects inherit these unless they set their own.
          </p>
          <AgentConfigForm
            value={agent}
            onChange={patchAgent}
            disabled={busy}
          />

          <div className="space-y-3 rounded-lg border border-line bg-well p-3">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
              Command templates
            </div>
            <p className="text-[11px] leading-relaxed text-ink-subtle">
              <span className="mono">{"{prompt}"}</span> is one argument,{" "}
              <span className="mono">{"{model}"}</span> and{" "}
              <span className="mono">{"{auto}"}</span> are substituted where
              present, and <span className="mono">[ … ]</span> groups are dropped
              when their placeholders are empty.
            </p>
            {(
              [
                ["opencode", "opencode"],
                ["codex", "Codex"],
                ["claude", "Claude Code"],
                ["cursor", "Cursor Agent"],
              ] as [keyof CommandTemplates, string][]
            ).map(([key, label]) => (
              <Field key={key} label={label}>
                <input
                  className="input mono !text-[11.5px]"
                  disabled={busy}
                  value={templates[key]}
                  onChange={(e) => patchTemplate(key, e.target.value)}
                />
              </Field>
            ))}
          </div>
        </section>

        <section className="space-y-2">
          <div className="flex items-center justify-between">
            <SectionLabel icon="eye">Secret storage</SectionLabel>
            <SavedPill show={flash === "secrets"} />
          </div>
          <select
            className="select"
            disabled={busy}
            value={settings.secret_store ?? "auto"}
            onChange={(e) =>
              void persist(
                {
                  secret_store:
                    e.target.value === "auto"
                      ? null
                      : (e.target.value as SecretStore),
                },
                "secrets",
              )
            }
          >
            {SECRET_STORES.map((s) => (
              <option key={s.id} value={s.id}>
                {s.label}
              </option>
            ))}
          </select>
          <p className="text-[11px] leading-relaxed text-ink-subtle">
            {SECRET_STORES.find(
              (s) => s.id === (settings.secret_store ?? "auto"),
            )?.desc}{" "}
            Changing this re-encrypts the project environment variables already
            saved.
          </p>
        </section>

        <section className="space-y-2">
          <div className="flex items-center justify-between">
            <SectionLabel icon="trash">Cache</SectionLabel>
            <SavedPill show={flash === "cache"} />
          </div>
          <div className="rounded-lg border border-line bg-well p-3">
            <div className="flex items-center justify-between gap-3 text-[12px]">
              <span className="text-ink-muted">Keep cached data for</span>
              <select
                className="select !w-auto"
                disabled={busy}
                value={settings.cache_retention_days}
                onChange={(e) =>
                  void persist(
                    { cache_retention_days: Number(e.target.value) },
                    "cache",
                  )
                }
              >
                {RETENTION_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="mt-3 grid grid-cols-2 gap-2">
              <StatBox
                label="Prompts"
                value={stats ? `${stats.prompt_count}` : "—"}
                hint={
                  stats?.oldest_prompt
                    ? `oldest ${clock(stats.oldest_prompt)}`
                    : "none cached"
                }
              />
              <StatBox
                label="Task logs"
                value={stats ? `${stats.log_count}` : "—"}
                hint={stats ? formatBytes(stats.log_bytes) : ""}
              />
            </div>

            <div className="mt-3 flex flex-wrap gap-2">
              <button
                className="btn btn-ghost"
                disabled={busy || !stats?.prompt_count}
                onClick={() => void clear(true, false)}
              >
                <Icon name="trash" className="h-3.5 w-3.5" />
                Clear prompt history
              </button>
              <button
                className="btn btn-ghost"
                disabled={busy || !stats?.log_count}
                onClick={() => void clear(false, true)}
              >
                <Icon name="trash" className="h-3.5 w-3.5" />
                Clear task logs
              </button>
              <button
                className="btn btn-danger"
                disabled={busy || (!stats?.prompt_count && !stats?.log_count)}
                onClick={() => void clear(true, true)}
              >
                Clear all cache
              </button>
            </div>
            <p className="mt-3 text-[11px] leading-relaxed text-ink-subtle">
              Prompts you've written are remembered so you can re-use them, and
              task logs are kept for review. Both are pruned after the retention
              window. Projects and task history are always kept. Logs of running
              tasks are never cleared.
            </p>
          </div>
        </section>

        {error && (
          <div className="rounded-lg border border-danger-line bg-danger-soft p-2.5 text-[12px] text-danger">
            {error}
          </div>
        )}
      </div>
    </Modal>
  );
}

function StatBox({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="rounded-lg border border-line bg-panel px-3 py-2">
      <div className="text-[10px] uppercase tracking-wide text-ink-subtle">
        {label}
      </div>
      <div className="mono text-base font-semibold text-ink">{value}</div>
      {hint && <div className="text-[10px] text-ink-subtle">{hint}</div>}
    </div>
  );
}
