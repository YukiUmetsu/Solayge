import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icons";

/** A titled group heading with an icon. */
export function SectionLabel({
  icon,
  children,
}: {
  icon: IconName;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
      <Icon name={icon} className="h-3.5 w-3.5" />
      {children}
    </div>
  );
}

/** A labelled form control: the label wraps the control it describes. */
export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1 block text-[10.5px] font-semibold uppercase tracking-wide text-ink-subtle">
        {label}
      </span>
      {children}
      {hint && <span className="mt-1 block text-[10.5px] text-ink-subtle">{hint}</span>}
    </label>
  );
}

/** The small uppercase caption used above loose (unwrapped) controls. */
export function Label({ children }: { children: ReactNode }) {
  return (
    <span className="mb-1.5 block text-[11px] font-semibold uppercase tracking-wide text-ink-subtle">
      {children}
    </span>
  );
}

/** The standard inline error banner. Renders nothing when there is no error. */
export function ErrorNote({ error }: { error: string | null | undefined }) {
  if (!error) return null;
  return (
    <div className="rounded-lg border border-danger-line bg-danger-soft p-2.5 text-[12px] text-danger">
      {error}
    </div>
  );
}
