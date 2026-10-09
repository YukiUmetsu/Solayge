import { useState } from "react";

import { useNotifications } from "../client/NotificationCenter";

const DISMISS_KEY = "solayge.notifications.prompt-dismissed.v1";

function dismissedBefore(): boolean {
  try {
    return typeof localStorage !== "undefined" && localStorage.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * A one-time nudge to turn notifications on. It only appears while the platform
 * can still ask (permission not yet denied) and the user has not enabled or
 * dismissed it, so it never nags.
 */
export function NotificationPrompt() {
  const { settings, permission, supported, updateSettings, requestPermission } =
    useNotifications();
  const [dismissed, setDismissed] = useState(dismissedBefore);

  function dismiss() {
    setDismissed(true);
    try {
      localStorage.setItem(DISMISS_KEY, "1");
    } catch {
      // Dismissal is best effort.
    }
  }

  if (settings.enabled || dismissed || !supported || permission === "denied") return null;

  return (
    <div className="notify-prompt" role="status">
      <div className="notify-prompt__text">
        <strong>Get notified when an agent needs you</strong>
        <span>Questions, task completions, and failures can alert you on this phone.</span>
      </div>
      <div className="notify-prompt__actions">
        <button
          className="btn btn-primary btn--small"
          type="button"
          onClick={() => {
            void (async () => {
              const result = await requestPermission();
              if (result === "granted") updateSettings({ enabled: true });
            })();
          }}
        >
          Turn on
        </button>
        <button className="btn btn--small" type="button" onClick={dismiss}>
          Not now
        </button>
      </div>
    </div>
  );
}
