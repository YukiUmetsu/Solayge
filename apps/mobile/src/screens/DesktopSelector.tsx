import { useState } from "react";

import { useClientContext } from "../client/ClientContext";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";

/**
 * First launch / desktop selector (mobile spec section 7.2 A). With no paired
 * desktop it explains the requirement and offers QR pairing; the desktop
 * always approves the phone.
 */
export function DesktopSelector({ onAddDesktop }: { onAddDesktop: () => void }) {
  const { desktops, desktopId, selectDesktop, status } = useClientContext();
  const [howOpen, setHowOpen] = useState(false);

  if (desktops.length === 0) {
    return (
      <div className="selector">
        <EmptyState
          icon="⌘"
          title="Connect to your computer"
          body="Your computer must be running Solayge. Pair this phone by scanning the QR code it shows."
          action={
            <>
              <button className="btn btn-primary" type="button" onClick={onAddDesktop}>
                Scan QR code
              </button>
              <button className="btn" type="button" onClick={() => setHowOpen(true)}>
                How it works
              </button>
            </>
          }
        />
        <Modal open={howOpen} title="How pairing works" onClose={() => setHowOpen(false)}>
          <p>
            The QR code contains only public information. You still approve this phone on the desktop,
            and both devices show the same short verification code before you trust the connection.
          </p>
        </Modal>
      </div>
    );
  }

  return (
    <div className="selector">
      <h2 className="selector__title">Your desktops</h2>
      <ul className="desktop-list">
        {desktops.map((desktop) => {
          const selected = desktop.id === desktopId;
          return (
            <li key={desktop.id}>
              <button
                type="button"
                className={`desktop-row${selected ? " desktop-row--selected" : ""}`}
                aria-pressed={selected}
                onClick={() => selectDesktop(desktop.id)}
              >
                <span className="desktop-row__name">{desktop.name}</span>
                {desktop.platform ? (
                  <span className="desktop-row__platform">{desktop.platform}</span>
                ) : null}
                <span className="desktop-row__hint">
                  <span aria-hidden="true">{selected ? "● " : "⚠ "}</span>
                  {selected ? "Selected" : "Offline"}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <button className="btn" type="button" onClick={onAddDesktop}>
        + Add a desktop
      </button>
      <p className="selector__status" role="status">
        Connection: {status}
      </p>
    </div>
  );
}
