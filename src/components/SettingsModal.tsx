import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AgentConfig,
  CacheStats,
  CommandTemplates,
  EnvironmentStatus,
  NotificationSettings,
  NotifyKind,
  PermissionProfile,
  SecretStore,
  Settings,
  Snapshot,
} from "../types";
import { api } from "../api";
import { clock } from "../lib/format";
import { SECRET_STORES } from "../lib/providers";
import {
  DEFAULT_NOTIFICATION_SETTINGS,
  DEFAULT_SOUND_FOR,
  SOUND_PRESETS,
  playSound,
} from "../lib/notifications";
import { useTheme, type ThemePref } from "../theme";
import { AgentConfigForm, Field, SectionLabel } from "./AgentConfigForm";
import { ErrorNote } from "./Field";
import { SaveButton, SavedPill, type SaveState } from "./SaveButton";
import { Modal } from "./Modal";
import { Icon, type IconName } from "./Icons";

const RETENTION_OPTIONS: { value: number; label: string }[] = [
  { value: 7, label: "7 days" },
  { value: 30, label: "30 days" },
  { value: 90, label: "90 days" },
  { value: 365, label: "1 year" },
  { value: 0, label: "Keep forever" },
];

const THEME_OPTIONS: ThemePref[] = ["system", "light", "dark"];

type Tab = "general" | "agent" | "notifications" | "advanced";

const TABS: { id: Tab; label: string; icon: IconName }[] = [
  { id: "general", label: "General", icon: "settings" },
  { id: "agent", label: "Agent", icon: "sparkles" },
  { id: "notifications", label: "Notifications", icon: "zap" },
  { id: "advanced", label: "Advanced", icon: "layers" },
];

/** The four event kinds the user can tune, with the field that holds each. */
const NOTIFY_EVENTS: {
  kind: NotifyKind;
  toggle: keyof NotificationSettings;
  sound: keyof NotificationSettings;
  label: string;
  hint: string;
}[] = [
  {
    kind: "task_complete",
    toggle: "on_task_complete",
    sound: "complete_sound",
    label: "Task completes",
    hint: "A task finished successfully (and any review passed).",
  },
  {
    kind: "task_failed",
    toggle: "on_task_failed",
    sound: "failed_sound",
    label: "Task fails",
    hint: "A task failed, or its review could not run.",
  },
  {
    kind: "task_review",
    toggle: "on_task_review",
    sound: "review_sound",
    label: "Entering review",
    hint: "A task finished and its automatic review started or settled.",
  },
  {
    kind: "needs_attention",
    toggle: "on_needs_attention",
    sound: "attention_sound",
    label: "Needs attention",
    hint: "A task is waiting on a question, permission, conflict, or was interrupted.",
  },
];

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function SettingsModal({
  snapshot,
  tools,
  onClose,
  onSaved,
}: {
  snapshot: Snapshot;
  tools: EnvironmentStatus | null;
  onClose: () => void;
  onSaved: (s: Snapshot) => void;
}) {
  const { pref, setPref } = useTheme();
  const settings = snapshot.settings;
  const [tab, setTab] = useState<Tab>("general");
  const [stats, setStats] = useState<CacheStats | null>(null);
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [flash, setFlash] = useState<string | null>(null);
  const flashTimer = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [errorLog, setErrorLog] = useState<string>("");

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
    review_prompt: settings.review_prompt ?? null,
    editor: settings.editor ?? null,
  }));
  const [templates, setTemplates] = useState<CommandTemplates>(
    () => settings.command_templates,
  );
  const [notifications, setNotifications] = useState<NotificationSettings>(
    () => ({
      ...DEFAULT_NOTIFICATION_SETTINGS,
      ...(settings.notifications ?? {}),
    }),
  );

  const refreshStats = () =>
    api
      .cacheStats()
      .then(setStats)
      .catch(() => {});

  const refreshErrorLog = () =>
    api
      .errorLog()
      .then(setErrorLog)
      .catch(() => {});

  useEffect(() => {
    void refreshStats();
    void refreshErrorLog();
  }, []);

  async function clearErrors() {
    setBusy(true);
    setError(null);
    try {
      await api.clearErrorLog();
      setErrorLog("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

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

  async function saveAll() {
    setSaveState("saving");
    setError(null);
    try {
      onSaved(
        await api.updateSettings({
          ...settings,
          ...agent,
          review_mode: agent.review_mode ?? "off",
          command_templates: templates,
          notifications,
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

  function patchNotifications(p: Partial<NotificationSettings>) {
    setNotifications((n) => ({ ...n, ...p }));
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
      subtitle="Appearance, agents, notifications, backup, and the local cache."
      onClose={onClose}
      width="max-w-5xl"
      footer={
        <>
          <button className="btn btn-ghost" onClick={onClose}>
            Done
          </button>
          <SaveButton
            dirty={dirty}
            state={saveState}
            onClick={saveAll}
            label="Save changes"
            disabled={busy}
          />
        </>
      }
    >
      <div className="flex min-h-[60vh] gap-5">
        <nav className="flex w-40 shrink-0 flex-col gap-1">
          {TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={`flex items-center gap-2 rounded-lg px-3 py-2 text-left text-[12.5px] font-medium transition ${
                tab === t.id
                  ? "bg-accent-soft text-accent-text"
                  : "text-ink-muted hover:bg-hover hover:text-ink"
              }`}
            >
              <Icon name={t.icon} className="h-3.5 w-3.5" />
              {t.label}
            </button>
          ))}
        </nav>

        <div className="min-w-0 flex-1 space-y-6">
          {tab === "general" && (
            <GeneralTab
              settings={settings}
              busy={busy}
              pref={pref}
              setPref={setPref}
              flash={flash}
              persist={persist}
            />
          )}

          {tab === "agent" && (
            <>
              <section className="space-y-4">
                <SectionLabel icon="sparkles">Agent defaults</SectionLabel>
                <p className="text-[11px] text-ink-subtle">
                  Projects inherit these unless they set their own.
                </p>
                <AgentConfigForm
                  value={agent}
                  onChange={patchAgent}
                  disabled={busy}
                  tools={tools}
                  effectiveProvider={agent.provider ?? "opencode"}
                />

                <div className="space-y-3 rounded-lg border border-line bg-well p-3">
                  <div className="text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
                    Command templates
                  </div>
                  <p className="text-[11px] leading-relaxed text-ink-subtle">
                    <span className="mono">{"{prompt}"}</span> is one argument,{" "}
                    <span className="mono">{"{model}"}</span> and{" "}
                    <span className="mono">{"{auto}"}</span> are substituted where
                    present, and <span className="mono">[ … ]</span> groups are
                    dropped when their placeholders are empty.
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
            </>
          )}

          {tab === "notifications" && (
            <NotificationsTab
              value={notifications}
              busy={busy}
              onChange={patchNotifications}
              onError={setError}
            />
          )}

          {tab === "advanced" && (
            <AdvancedTab
              settings={settings}
              busy={busy}
              stats={stats}
              flash={flash}
              persist={persist}
              clear={clear}
              errorLog={errorLog}
              refreshErrorLog={refreshErrorLog}
              clearErrors={clearErrors}
            />
          )}

          <ErrorNote error={error} />
        </div>
      </div>
    </Modal>
  );
}

function GeneralTab({
  settings,
  busy,
  pref,
  setPref,
  flash,
  persist,
}: {
  settings: Settings;
  busy: boolean;
  pref: ThemePref;
  setPref: (p: ThemePref) => void;
  flash: string | null;
  persist: (patch: Partial<Settings>, key: string) => Promise<void>;
}) {
  return (
    <>
      <section className="space-y-2">
        <SectionLabel icon="sun">Appearance</SectionLabel>
        <div className="flex max-w-md rounded-lg border border-line p-0.5">
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
          className="select max-w-md"
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

      <section className="space-y-2">
        <div className="flex items-center justify-between">
          <SectionLabel icon="eye">Secret storage</SectionLabel>
          <SavedPill show={flash === "secrets"} />
        </div>
        <select
          className="select max-w-md"
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
    </>
  );
}

function NotificationsTab({
  value,
  busy,
  onChange,
  onError,
}: {
  value: NotificationSettings;
  busy: boolean;
  onChange: (p: Partial<NotificationSettings>) => void;
  onError: (e: string | null) => void;
}) {
  return (
    <>
      <section className="space-y-3">
        <SectionLabel icon="zap">Desktop notifications</SectionLabel>
        <div className="rounded-lg border border-line bg-well p-3">
          <ToggleRow
            label="Enable notifications"
            hint="Master switch. Turns off every desktop notification below."
            checked={value.enabled}
            disabled={busy}
            onChange={(v) => onChange({ enabled: v })}
          />
        </div>
      </section>

      <section className="space-y-3">
        <SectionLabel icon="alert">Notify me when</SectionLabel>
        <div className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-well">
          {NOTIFY_EVENTS.map((ev) => (
            <ToggleRow
              key={ev.kind}
              label={ev.label}
              hint={ev.hint}
              checked={value[ev.toggle] as boolean}
              disabled={busy || !value.enabled}
              onChange={(v) =>
                onChange({ [ev.toggle]: v } as Partial<NotificationSettings>)
              }
            />
          ))}
        </div>
        <div className="flex justify-end gap-2">
          <button
            className="btn btn-ghost !px-2.5 !py-1 text-[11px]"
            disabled={busy || !value.enabled}
            onClick={() => {
              onError(null);
              void playSound(value.attention_sound, value.volume);
            }}
          >
            <Icon name="play" className="h-3 w-3" />
            Test sound
          </button>
          <button
            className="btn btn-ghost !px-2.5 !py-1 text-[11px]"
            disabled={busy || !value.enabled}
            onClick={() => {
              onError(null);
              api
                .testNotification()
                .catch((e) => onError(String(e)));
            }}
          >
            <Icon name="zap" className="h-3 w-3" />
            Test notification
          </button>
        </div>
      </section>

      <section className="space-y-3">
        <SectionLabel icon="sparkles">Sound</SectionLabel>
        <div className="rounded-lg border border-line bg-well p-3">
          <ToggleRow
            label="Play a sound"
            hint="Play a tone alongside each notification."
            checked={value.sound_enabled}
            disabled={busy || !value.enabled}
            onChange={(v) => onChange({ sound_enabled: v })}
          />
          <div className="mt-3 flex items-center gap-3">
            <span className="w-20 shrink-0 text-[12px] text-ink-muted">
              Volume
            </span>
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(value.volume * 100)}
              disabled={busy || !value.enabled || !value.sound_enabled}
              onChange={(e) => onChange({ volume: Number(e.target.value) / 100 })}
              className="h-1 flex-1 cursor-pointer accent-accent"
            />
            <span className="w-9 shrink-0 text-right text-[11px] text-ink-subtle">
              {Math.round(value.volume * 100)}%
            </span>
          </div>
        </div>

        <div className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-well">
          {NOTIFY_EVENTS.map((ev) => (
            <SoundRow
              key={ev.kind}
              label={ev.label}
              value={value[ev.sound] as string | null}
              fallback={DEFAULT_SOUND_FOR[ev.kind]}
              disabled={busy || !value.enabled || !value.sound_enabled}
              volume={value.volume}
              onChange={(v) =>
                onChange({ [ev.sound]: v } as Partial<NotificationSettings>)
              }
              onError={onError}
            />
          ))}
        </div>
      </section>
    </>
  );
}

function AdvancedTab({
  settings,
  busy,
  stats,
  flash,
  persist,
  clear,
  errorLog,
  refreshErrorLog,
  clearErrors,
}: {
  settings: Settings;
  busy: boolean;
  stats: CacheStats | null;
  flash: string | null;
  persist: (patch: Partial<Settings>, key: string) => Promise<void>;
  clear: (prompts: boolean, logs: boolean) => Promise<void>;
  errorLog: string;
  refreshErrorLog: () => Promise<void>;
  clearErrors: () => Promise<void>;
}) {
  return (
    <>
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

      <section className="space-y-2">
        <div className="flex items-center justify-between">
          <SectionLabel icon="alert">Error log</SectionLabel>
          <div className="flex items-center gap-2">
            <button
              className="btn btn-ghost !px-2 !py-1"
              onClick={() => void refreshErrorLog()}
              disabled={busy}
            >
              <Icon name="refresh" className="h-3 w-3" />
              Refresh
            </button>
            <button
              className="btn btn-ghost !px-2 !py-1"
              onClick={() => void clearErrors()}
              disabled={busy || !errorLog.trim()}
            >
              <Icon name="trash" className="h-3 w-3" />
              Clear
            </button>
          </div>
        </div>
        <p className="text-[11px] leading-relaxed text-ink-subtle">
          Errors only: scheduler, task-launch, and state-save failures. Full task
          output stays under each task's Logs.
        </p>
        <pre className="mono max-h-64 min-h-16 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-well p-2.5 text-[11px] leading-relaxed text-ink-muted">
          {errorLog.trim() || "No errors logged."}
        </pre>
      </section>
    </>
  );
}

/** A labelled on/off switch row. */
function ToggleRow({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 px-3 py-2.5">
      <div className="min-w-0">
        <div className="text-[12.5px] font-medium text-ink">{label}</div>
        {hint && (
          <div className="mt-0.5 text-[11px] leading-relaxed text-ink-subtle">
            {hint}
          </div>
        )}
      </div>
      <button
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative h-5 w-9 shrink-0 rounded-full transition disabled:opacity-50 ${
          checked ? "bg-accent" : "bg-line-strong"
        }`}
      >
        <span
          className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-all ${
            checked ? "left-[18px]" : "left-0.5"
          }`}
        />
      </button>
    </div>
  );
}

/** A sound picker: bundled presets, a file picker, or silence. */
function SoundRow({
  label,
  value,
  fallback,
  disabled,
  volume,
  onChange,
  onError,
}: {
  label: string;
  value: string | null;
  fallback: string;
  disabled?: boolean;
  volume: number;
  onChange: (v: string | null) => void;
  onError: (e: string | null) => void;
}) {
  const isCustom = !!value && value.startsWith("file:");
  const selectValue = value === null ? "none" : isCustom ? "custom" : value;

  async function chooseFile() {
    try {
      const picked = await open({
        multiple: false,
        filters: [
          {
            name: "Audio",
            extensions: ["wav", "mp3", "m4a", "aac", "ogg", "flac"],
          },
        ],
      });
      if (typeof picked === "string") onChange(`file:${picked}`);
    } catch (e) {
      onError(String(e));
    }
  }

  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <span className="w-28 shrink-0 text-[12.5px] text-ink">{label}</span>
      <select
        className="select flex-1"
        disabled={disabled}
        value={selectValue}
        onChange={(e) => {
          const v = e.target.value;
          if (v === "custom") void chooseFile();
          else onChange(v === "none" ? null : v);
        }}
      >
        <option value="none">No sound</option>
        {SOUND_PRESETS.map((p) => (
          <option key={p.id} value={p.id}>
            {p.label}
          </option>
        ))}
        <option value="custom">
          {isCustom ? customName(value) : "Choose file…"}
        </option>
      </select>
      <button
        className="btn btn-ghost shrink-0 !px-2 !py-1"
        disabled={disabled || !value}
        title="Preview"
        onClick={() => {
          onError(null);
          void playSound(value ?? fallback, volume);
        }}
      >
        <Icon name="play" className="h-3 w-3" />
      </button>
    </div>
  );
}

function customName(value: string): string {
  const path = value.slice("file:".length);
  const name = path.split(/[\\/]/).pop() ?? path;
  return `Custom: ${name}`;
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
