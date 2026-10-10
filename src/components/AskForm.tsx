import { useEffect, useState } from "react";
import type { AskField, Snapshot, Task, TaskAsk } from "../types";
import { api } from "../api";
import { Icon } from "./Icons";
import {
  DECISION_HINTS,
  DECISION_LABELS,
  PermissionDetails,
} from "./PermissionDetails";

/** Seed an answer object from an ask's field defaults. */
export function defaultAnswer(ask: TaskAsk): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const f of ask.fields) {
    // External fields are completed elsewhere; there is no value to submit.
    if (f.kind === "external") continue;
    if (f.kind === "multiselect") out[f.key] = Array.isArray(f.default) ? f.default : [];
    else if (f.kind === "boolean") out[f.key] = typeof f.default === "boolean" ? f.default : false;
    else out[f.key] = f.default ?? "";
  }
  return out;
}

/** Whether a field already carries an answer (external fields count as done). */
function fieldAnswered(field: AskField, value: unknown): boolean {
  if (field.kind === "external") return true;
  if (value === "" || value === undefined || value === null) return false;
  if (Array.isArray(value) && value.length === 0) return false;
  return true;
}

/**
 * The form for one pending ask, shared by the app-level attention modal and the
 * task side panel so the two never drift.
 *
 * A question with several fields is split into tabs so each one can be answered
 * on its own without losing sight of the others (and without the form pushing
 * the task log off screen). A single Submit posts the whole answer.
 */
export function AskForm({
  task,
  onAnswered,
  onError,
}: {
  task: Task;
  onAnswered?: (s: Snapshot) => void;
  onError?: (message: string) => void;
}) {
  const ask = task.ask as TaskAsk;
  const [answer, setAnswer] = useState<Record<string, unknown>>(() => defaultAnswer(ask));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [tab, setTab] = useState(0);

  useEffect(() => {
    setAnswer(defaultAnswer(ask));
    setErr(null);
    setTab(0);
    // Reset only when the ask itself changes; the snapshot poll replaces the
    // `ask` object every couple of seconds with the same id.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ask.id]);

  const set = (key: string, value: unknown) =>
    setAnswer((a) => ({ ...a, [key]: value }));

  const missing = ask.fields.some((f) => f.required && !fieldAnswered(f, answer[f.key]));

  async function send(value: Record<string, unknown>) {
    setBusy(true);
    setErr(null);
    try {
      const s = await api.answerTask(task.id, value);
      onAnswered?.(s);
    } catch (e) {
      const message = String(e);
      setErr(message);
      onError?.(message);
    } finally {
      setBusy(false);
    }
  }

  if (ask.kind === "permission") {
    return (
      <div className="space-y-3">
        <PermissionDetails ask={ask} />
        <div className="flex flex-wrap gap-2">
          {ask.options.map((o) => (
            <button
              key={o}
              className={
                o === "once"
                  ? "btn btn-primary"
                  : o === "reject"
                    ? "btn btn-danger"
                    : "btn btn-ghost"
              }
              disabled={busy}
              title={DECISION_HINTS[o]}
              onClick={() => void send({ decision: o })}
            >
              {DECISION_LABELS[o] ?? o}
            </button>
          ))}
        </div>
        {err && <p className="text-[11px] text-danger">{err}</p>}
      </div>
    );
  }

  const multiple = ask.fields.length > 1;
  const current = ask.fields[Math.min(tab, ask.fields.length - 1)];

  return (
    <div className="space-y-3">
      {ask.message && (
        <pre className="mono whitespace-pre-wrap rounded-lg border border-line bg-well p-2 text-[11.5px] leading-relaxed text-ink-muted">
          {ask.message}
        </pre>
      )}

      {multiple && (
        <div className="scroll-x flex items-end gap-1 border-b border-line">
          {ask.fields.map((f, i) => {
            const answered = fieldAnswered(f, answer[f.key]);
            const needs = f.required && !answered;
            return (
              <button
                key={f.key}
                onClick={() => setTab(i)}
                title={f.label}
                className={`flex shrink-0 items-center gap-1.5 rounded-t-lg px-2.5 py-1.5 text-[11.5px] transition ${
                  i === tab
                    ? "border-b-2 border-accent text-ink"
                    : "text-ink-muted hover:text-ink"
                }`}
              >
                <span
                  className={`flex h-4 w-4 items-center justify-center rounded-full text-[9.5px] font-semibold ${
                    answered
                      ? "bg-success-soft text-success"
                      : needs
                        ? "bg-warning-soft text-warning"
                        : "bg-well-strong text-ink-subtle"
                  }`}
                >
                  {answered ? <Icon name="check" className="h-2.5 w-2.5" /> : i + 1}
                </span>
                <span className="max-w-[13rem] truncate">{f.label}</span>
              </button>
            );
          })}
        </div>
      )}

      <div className="space-y-3">
        {multiple && current ? (
          <AskFieldInput
            field={current}
            value={answer[current.key]}
            onChange={(v) => set(current.key, v)}
          />
        ) : (
          ask.fields.map((f) => (
            <AskFieldInput
              key={f.key}
              field={f}
              value={answer[f.key]}
              onChange={(v) => set(f.key, v)}
            />
          ))
        )}
      </div>

      {multiple && (
        <div className="flex items-center justify-between text-[11px] text-ink-subtle">
          <button
            className="btn btn-ghost !px-2 !py-1"
            disabled={tab === 0}
            onClick={() => setTab((t) => Math.max(0, t - 1))}
          >
            Back
          </button>
          <span>
            Question {tab + 1} of {ask.fields.length}
          </span>
          <button
            className="btn btn-ghost !px-2 !py-1"
            disabled={tab >= ask.fields.length - 1}
            onClick={() => setTab((t) => Math.min(ask.fields.length - 1, t + 1))}
          >
            Next
          </button>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2 border-t border-line pt-3">
        <button
          className="btn btn-primary"
          disabled={busy || missing}
          onClick={() => void send(answer)}
        >
          <Icon name="check" className="h-3.5 w-3.5" />
          {multiple ? "Submit all answers" : "Send answer"}
        </button>
        {missing && (
          <span className="text-[11px] text-ink-subtle">
            Fill in the required fields
          </span>
        )}
        {err && <span className="text-[11px] text-danger">{err}</span>}
      </div>
    </div>
  );
}

export function AskFieldInput({
  field,
  value,
  onChange,
}: {
  field: AskField;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const [draft, setDraft] = useState("");
  const label = (
    <div className="mb-1 text-[12px] font-medium text-ink">
      {field.label}
      {field.required && <span className="text-danger"> *</span>}
    </div>
  );
  const description = field.description ? (
    <p className="mt-1 text-[10.5px] text-ink-subtle">{field.description}</p>
  ) : null;

  // An external field is completed outside the form (e.g. an OAuth page).
  if (field.kind === "external") {
    return (
      <div>
        {label}
        <div className="flex flex-wrap items-center gap-2">
          <button
            className="btn btn-ghost !py-1"
            disabled={!field.url}
            onClick={() => {
              if (field.url) void api.openExternal(field.url).catch(() => {});
            }}
          >
            <Icon name="external" className="h-3.5 w-3.5" />
            Open
          </button>
          <span className="text-[10.5px] text-ink-subtle">
            Finish this outside Solayge, then send your answer below.
          </span>
        </div>
        {description}
      </div>
    );
  }

  if (field.kind === "boolean") {
    return (
      <div>
        {label}
        <div className="flex gap-1.5">
          {[
            { l: "Yes", v: true },
            { l: "No", v: false },
          ].map((b) => (
            <button
              key={b.l}
              className={`rounded-lg border px-2 py-1 text-[11.5px] ${
                value === b.v
                  ? "border-accent-line bg-accent-soft text-accent-text"
                  : "border-line bg-well text-ink-muted"
              }`}
              onClick={() => onChange(b.v)}
            >
              {b.l}
            </button>
          ))}
        </div>
        {description}
      </div>
    );
  }

  const numeric = field.kind === "number" || field.kind === "integer";

  if (field.options.length > 0) {
    const isOption = (v: string) => field.options.some((o) => o.value === v);
    const arr = Array.isArray(value) ? (value as string[]) : [];
    // Hand-typed multiselect entries, shown as removable chips.
    const customValues = arr.filter((v) => !isOption(v));
    // For a single-choice field, a typed value that is not one of the options.
    const customText =
      !Array.isArray(value) && typeof value === "string" && value !== "" && !isOption(value)
        ? value
        : "";

    const optionClass = (on: boolean) =>
      `rounded-lg border px-2 py-1 text-[11.5px] transition ${
        on
          ? "border-accent-line bg-accent-soft text-accent-text"
          : "border-line bg-well text-ink-muted hover:border-accent-line"
      }`;

    return (
      <div>
        {label}
        <div className="flex flex-wrap gap-1.5">
          {field.options.map((o) => {
            const on = field.kind === "multiselect" ? arr.includes(o.value) : value === o.value;
            return (
              <button
                key={o.value}
                title={o.description ?? undefined}
                className={optionClass(on)}
                onClick={() => {
                  if (field.kind === "multiselect") {
                    const next = [...arr];
                    const i = next.indexOf(o.value);
                    if (i >= 0) next.splice(i, 1);
                    else next.push(o.value);
                    onChange(next);
                  } else {
                    onChange(o.value);
                  }
                }}
              >
                {o.label}
              </button>
            );
          })}
        </div>

        {customValues.length > 0 && (
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {customValues.map((v) => (
              <span
                key={v}
                className="inline-flex items-center gap-1 rounded-lg border border-accent-line bg-accent-soft px-2 py-0.5 text-[11px] text-accent-text"
              >
                {v}
                <button
                  className="text-accent-text hover:text-ink"
                  title="Remove"
                  onClick={() => onChange(arr.filter((x) => x !== v))}
                >
                  <Icon name="x" className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
        )}

        {field.custom &&
          (field.kind === "multiselect" ? (
            <div className="mt-1.5 flex gap-1.5">
              <input
                className="min-w-0 flex-1 rounded-lg border border-line bg-well px-2 py-1 text-[12px] text-ink outline-none focus:border-accent-line"
                placeholder={field.placeholder ?? "Type your own answer"}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && draft.trim()) {
                    e.preventDefault();
                    onChange([...arr, draft.trim()]);
                    setDraft("");
                  }
                }}
              />
              <button
                className="btn btn-ghost !py-1"
                disabled={!draft.trim()}
                onClick={() => {
                  onChange([...arr, draft.trim()]);
                  setDraft("");
                }}
              >
                Add
              </button>
            </div>
          ) : (
            <input
              className="mt-1.5 w-full rounded-lg border border-line bg-well px-2 py-1 text-[12px] text-ink outline-none focus:border-accent-line"
              placeholder={field.placeholder ?? "Or type your own answer"}
              value={customText}
              onChange={(e) => onChange(e.target.value)}
            />
          ))}

        {!field.custom && field.kind !== "multiselect" && (
          <p className="mt-1 text-[10.5px] text-ink-subtle">
            Choose one of the options above.
          </p>
        )}
        {description}
      </div>
    );
  }

  const inputType = numeric
    ? "number"
    : field.format === "email"
      ? "email"
      : field.format === "uri"
        ? "url"
        : field.format === "date"
          ? "date"
          : field.format === "date-time"
            ? "datetime-local"
            : "text";
  return (
    <div>
      {label}
      <input
        className="w-full rounded-lg border border-line bg-well px-2 py-1 text-[12px] text-ink outline-none focus:border-accent-line"
        type={inputType}
        placeholder={field.placeholder ?? ""}
        min={field.min ?? undefined}
        max={field.max ?? undefined}
        minLength={field.min_length ?? undefined}
        maxLength={field.max_length ?? undefined}
        pattern={field.pattern ?? undefined}
        step={field.kind === "integer" ? 1 : numeric ? "any" : undefined}
        value={typeof value === "string" || typeof value === "number" ? value : ""}
        onChange={(e) =>
          onChange(
            numeric
              ? e.target.value === ""
                ? ""
                : Number(e.target.value)
              : e.target.value,
          )
        }
      />
      {description}
    </div>
  );
}
