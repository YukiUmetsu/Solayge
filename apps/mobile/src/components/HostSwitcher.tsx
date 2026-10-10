import { useClientContext } from "../client/ClientContext";
import { connectionStatusView } from "../client/status";
import { Modal } from "./Modal";
import { StatusPill } from "./StatusPill";

/**
 * The desktop switcher reached from the header chip. One active desktop at a
 * time; tapping a row selects it (mobile spec section 7.1).
 */
export function HostSwitcher({
  open,
  onClose,
  onAddDesktop,
}: {
  open: boolean;
  onClose: () => void;
  onAddDesktop: () => void;
}) {
  const { desktops, desktopId, selectDesktop, status } = useClientContext();

  return (
    <Modal open={open} title="Your desktops" onClose={onClose}>
      <ul className="desktop-list">
        {desktops.map((desktop) => {
          const selected = desktop.id === desktopId;
          const view = connectionStatusView(selected ? status : "offline");
          return (
            <li key={desktop.id}>
              <button
                type="button"
                className={`desktop-row${selected ? " desktop-row--selected" : ""}`}
                aria-pressed={selected}
                onClick={() => {
                  selectDesktop(desktop.id);
                  onClose();
                }}
              >
                <span className="desktop-row__name">{desktop.name}</span>
                {desktop.platform ? (
                  <span className="desktop-row__platform">{desktop.platform}</span>
                ) : null}
                {selected ? (
                  <StatusPill view={view} size="sm" />
                ) : (
                  <span className="desktop-row__hint">
                    <span aria-hidden="true">⚠ </span>
                    Offline
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
      <button className="btn" type="button" onClick={onAddDesktop}>
        + Add a desktop
      </button>
    </Modal>
  );
}
