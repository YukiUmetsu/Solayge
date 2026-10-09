import { useState } from "react";
import type { ApprovalField, JsonValue } from "@solayge/shared-types";

/** Answers keyed by field, in the shape the host forwards to the provider. */
export type FieldAnswers = Record<string, JsonValue>;

function textOf(value: JsonValue | undefined): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  return "";
}

function listOf(value: JsonValue | undefined): string[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is string => typeof entry === "string")
    : [];
}

/**
 * Only `http`/`https` URLs are rendered as live links. A question's URL is
 * supplied by the agent, which is untrusted, so a `javascript:`/`data:` scheme
 * (or any non-web scheme) is shown as plain text instead of becoming a
 * clickable navigation out of the app.
 */
function safeExternalHref(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:" ? parsed.href : null;
  } catch {
    return null;
  }
}

/**
 * Renders the structured content of an agent question: the exact prompt text,
 * any choices, and the free-text input the provider allows. It only collects
 * answers; sending is the caller's job, so a question never authorizes an action
 * (mobile spec section 7.2 D).
 */
export function QuestionFields({
  fields,
  answers,
  onChange,
  disabled,
}: {
  fields: ApprovalField[];
  answers: FieldAnswers;
  onChange: (key: string, value: JsonValue | null) => void;
  disabled: boolean;
}) {
  // Free text for a multiselect field's "other" entries, kept separate until it
  // is merged into the answer array.
  const [extra, setExtra] = useState<Record<string, string>>({});

  function mergedMulti(selected: string[], text: string): JsonValue {
    const tokens = text
      .split(",")
      .map((token) => token.trim())
      .filter(Boolean);
    return [...new Set([...selected, ...tokens])];
  }

  return (
    <div className="question-fields">
      {fields.map((field) => {
        const labelId = `qfield-${field.key}`;
        const hasOptions = field.options.length > 0;
        const showText = field.custom || !hasOptions;
        const raw = answers[field.key];
        const selectedList = listOf(raw);
        const isOptionValue =
          typeof raw === "string" && field.options.some((option) => option.value === raw);
        const inputValue =
          field.kind === "multiselect" ? (extra[field.key] ?? "") : isOptionValue ? "" : textOf(raw);
        const externalHref = field.kind === "external" ? safeExternalHref(field.url) : null;

        return (
          <div className="question-field" key={field.key}>
            <span className="question-field__label" id={labelId}>
              {field.label}
              {field.required ? <span className="question-field__required"> *</span> : null}
            </span>
            {field.description ? (
              <span className="question-field__help">{field.description}</span>
            ) : null}

            {field.kind === "external" ? (
              <p className="question-field__help">
                {externalHref ? (
                  <a href={externalHref} target="_blank" rel="noreferrer">
                    {field.url}
                  </a>
                ) : field.url ? (
                  <span className="question-field__url">{field.url}</span>
                ) : (
                  "This is completed outside the app."
                )}
              </p>
            ) : null}

            {field.kind === "boolean" ? (
              <label className="radio-row">
                <input
                  type="checkbox"
                  checked={raw === true}
                  disabled={disabled}
                  aria-labelledby={labelId}
                  onChange={(event) => onChange(field.key, event.target.checked)}
                />
                <span className="radio-row__label">Yes</span>
              </label>
            ) : null}

            {field.kind === "multiselect"
              ? field.options.map((option) => (
                  <label className="radio-row" key={option.value}>
                    <input
                      type="checkbox"
                      checked={selectedList.includes(option.value)}
                      disabled={disabled}
                      onChange={(event) => {
                        const next = event.target.checked
                          ? [...selectedList, option.value]
                          : selectedList.filter((entry) => entry !== option.value);
                        onChange(
                          field.key,
                          mergedMulti(next, extra[field.key] ?? ""),
                        );
                      }}
                    />
                    <span>
                      <span className="radio-row__label">{option.label}</span>
                      {option.description ? (
                        <span className="radio-row__help">{option.description}</span>
                      ) : null}
                    </span>
                  </label>
                ))
              : field.kind !== "boolean" && field.kind !== "external"
                ? field.options.map((option) => (
                    <label className="radio-row" key={option.value}>
                      <input
                        type="radio"
                        name={`qfield-${field.key}`}
                        checked={raw === option.value}
                        disabled={disabled}
                        onChange={() => onChange(field.key, option.value)}
                      />
                      <span>
                        <span className="radio-row__label">{option.label}</span>
                        {option.description ? (
                          <span className="radio-row__help">{option.description}</span>
                        ) : null}
                      </span>
                    </label>
                  ))
                : null}

            {showText && field.kind !== "boolean" && field.kind !== "external" ? (
              <input
                className="question-field__input"
                type={field.kind === "number" || field.kind === "integer" ? "number" : "text"}
                value={inputValue}
                placeholder={field.placeholder ?? "Type your answer…"}
                disabled={disabled}
                aria-labelledby={labelId}
                onChange={(event) => {
                  const text = event.target.value;
                  if (field.kind === "multiselect") {
                    setExtra((prev) => ({ ...prev, [field.key]: text }));
                    onChange(field.key, mergedMulti(selectedList, text));
                  } else if (field.kind === "number" || field.kind === "integer") {
                    const parsed = Number(text);
                    onChange(
                      field.key,
                      text.trim() === "" || Number.isNaN(parsed) ? null : parsed,
                    );
                  } else {
                    onChange(field.key, text);
                  }
                }}
              />
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
