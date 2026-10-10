import { useClientContext } from "../client/ClientContext";
import { connectionStatusView } from "../client/status";

/**
 * The app header. Root screens show the selected desktop (tap to switch);
 * detail screens show a back control and the screen title. Settings is always
 * one tap away (mobile spec section 7.1).
 */
export function AppHeader({
  title,
  onBack,
  onOpenSettings,
  onOpenSwitcher,
}: {
  title?: string;
  onBack?: () => void;
  onOpenSettings: () => void;
  onOpenSwitcher: () => void;
}) {
  const { desktop, desktopId, desktops, status } = useClientContext();
  const view = connectionStatusView(status);
  const known = desktops.find((entry) => entry.id === desktopId);
  const name = desktop?.name ?? known?.name ?? "No desktop connected";

  return (
    <header className="app-header">
      {title ? (
        <>
          <button className="app-header__back" type="button" onClick={onBack} aria-label="Back">
            <span aria-hidden="true">‹</span>
          </button>
          <h1 className="app-header__title">{title}</h1>
        </>
      ) : (
        <button
          className="host-chip"
          type="button"
          onClick={onOpenSwitcher}
          aria-label={`Current desktop: ${name}, ${view.label}. Change desktop.`}
        >
          <span className={`host-chip__dot host-chip__dot--${view.tone}`} aria-hidden="true">
            {view.icon}
          </span>
          <span className="host-chip__label">{name}</span>
          <span className="host-chip__chevron" aria-hidden="true">
            ▾
          </span>
        </button>
      )}
      <button
        className="app-header__settings"
        type="button"
        onClick={onOpenSettings}
        aria-label="Settings"
      >
        <span aria-hidden="true">⚙</span>
      </button>
    </header>
  );
}
