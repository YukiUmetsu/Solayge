import type { StatusView } from "../client/status";

/**
 * A status that always pairs an icon with text, so state is never conveyed by
 * color alone (mobile spec section 7.3).
 */
export function StatusPill({ view, size = "md" }: { view: StatusView; size?: "sm" | "md" }) {
  return (
    <span className={`pill pill--${view.tone} pill--${size}`}>
      <span className="pill__icon" aria-hidden="true">
        {view.icon}
      </span>
      <span className="pill__label">{view.label}</span>
    </span>
  );
}
