import { Icon } from "./Icons";

export type SaveState = "idle" | "saving" | "saved";

/**
 * A save button that makes its state obvious: greyed out when there is nothing
 * to save, accent-coloured when there are unsaved changes, "Saving…" while the
 * request is in flight, and green "Saved" for a moment afterwards.
 */
export function SaveButton({
  dirty,
  state,
  onClick,
  label = "Save",
  disabled,
}: {
  dirty: boolean;
  state: SaveState;
  onClick: () => void;
  label?: string;
  disabled?: boolean;
}) {
  const cls =
    state === "saved"
      ? "btn border border-success-line bg-success-soft text-success"
      : state === "saving"
        ? "btn btn-primary opacity-70"
        : dirty
          ? "btn btn-primary"
          : "btn btn-ghost opacity-60";
  return (
    <button
      className={cls}
      onClick={onClick}
      disabled={disabled || state === "saving" || (!dirty && state !== "saved")}
    >
      {state === "saved" && <Icon name="check" className="h-3.5 w-3.5" />}
      {state === "saving" ? "Saving…" : state === "saved" ? "Saved" : label}
    </button>
  );
}

/** Small transient "Saved" indicator for controls that persist immediately. */
export function SavedPill({ show, label = "Saved" }: { show: boolean; label?: string }) {
  if (!show) return null;
  return (
    <span className="flex items-center gap-1 text-[10.5px] font-medium text-success">
      <Icon name="check" className="h-3 w-3" />
      {label}
    </span>
  );
}
