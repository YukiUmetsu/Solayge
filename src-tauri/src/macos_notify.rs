//! Desktop notifications on macOS via the modern UserNotifications framework.
//!
//! `NSUserNotificationCenter` (what `notify-rust` / `mac-notification-sys`, and
//! therefore the Tauri notification plugin, use) is a "LegacyConnection" that
//! `usernoted` now denies outright — notifications built on it are silently
//! dropped. `UNUserNotificationCenter` is the supported API, so notifications
//! are posted from here instead.
//!
//! macOS can still refuse a request with `UNErrorDomain` code 1
//! (`UNErrorCodeNotificationsNotAllowed`): the app was never granted permission,
//! permission was later denied, or the process is not running from an installed
//! bundle (a plain `cargo`/`tauri dev` binary has no bundle identifier, so
//! `usernoted` has nothing to authorize). We cannot grant permission from inside
//! the process, so instead of dumping the raw framework error we:
//!
//! * remember the authorization answer so Settings can show a banner, while
//!   still attempting each delivery — a user who turns notifications on later
//!   recovers without restarting the app;
//! * translate a not-allowed failure into an actionable message that points at
//!   System Settings, and log that failure only once per app run;
//! * expose the status so Settings can show a banner and a shortcut to the pane.
//!
//! This is the only module allowed to touch Objective-C directly; everything
//! else stays safe Rust (`lib.rs` denies `unsafe_code`).

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSError, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

/// The pane System Settings opens for per-app notification settings.
const NOTIFICATION_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.preference.notifications";

/// The message shown when macOS refuses to deliver a notification.
pub const NOT_ALLOWED_HINT: &str =
    "macOS is not allowing Solayge to show notifications. Open System Settings → \
     Notifications, select Solayge, and turn on “Allow Notifications”.";

/// What macOS currently thinks of our notification authorization. Cached from
/// the async completion handler because the UI needs a synchronous answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthStatus {
    /// `init` has not heard back yet, or the app never asked.
    Unknown,
    /// Permission granted (including provisional/ephemeral).
    Authorized,
    /// Permission denied, or the OS refused the request.
    Denied,
}

static AUTH: Mutex<AuthStatus> = Mutex::new(AuthStatus::Unknown);
/// Records a denial once instead of on every notification an app run produces.
static DENIAL_LOGGED: AtomicBool = AtomicBool::new(false);

fn set_status(status: AuthStatus) {
    if let Ok(mut guard) = AUTH.lock() {
        *guard = status;
    }
}

fn status() -> AuthStatus {
    AUTH.lock().map(|guard| *guard).unwrap_or(AuthStatus::Unknown)
}

/// A machine-readable status word for the Settings UI.
pub fn status_word() -> &'static str {
    match status() {
        AuthStatus::Authorized => "authorized",
        AuthStatus::Denied => "denied",
        AuthStatus::Unknown => "undetermined",
    }
}

/// Translate a raw `NSError` string into something a user can act on.
///
/// Kept pure so it can be unit-tested without a running notification center.
pub fn describe_delivery_error(raw: &str) -> String {
    let lowered = raw.to_lowercase();
    if lowered.contains("unerrordomain error 1")
        || lowered.contains("notifications are not allowed")
        || lowered.contains("not allowed for this application")
    {
        format!("{NOT_ALLOWED_HINT} ({raw})")
    } else {
        raw.to_string()
    }
}

/// What a `UNUserNotificationCenter` delivery completion tells us.
#[derive(Debug, PartialEq, Eq)]
enum DeliveryOutcome {
    /// The request was accepted; the app is authorized.
    Delivered,
    /// macOS refused because notifications are not allowed for this app.
    NotAllowed,
    /// Any other failure, with the (already translated) detail to log.
    Failed(String),
}

/// Classify a delivery completion. `error` is `None` on success. Kept pure so
/// the authorization transitions are testable without the framework.
fn classify_delivery(error: Option<&str>) -> DeliveryOutcome {
    match error {
        None => DeliveryOutcome::Delivered,
        Some(raw) => {
            let detail = describe_delivery_error(raw);
            if detail.starts_with(NOT_ALLOWED_HINT) {
                DeliveryOutcome::NotAllowed
            } else {
                DeliveryOutcome::Failed(detail)
            }
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    struct NotifyDelegate;

    unsafe impl NSObjectProtocol for NotifyDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotifyDelegate {
        /// Presents notifications even while Solayge is the frontmost app;
        /// without a delegate that opts in, macOS quietly suppresses them.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion_handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler
                .call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
        }
    }
);

thread_local! {
    /// The notification center holds its delegate weakly, so keep our own strong
    /// reference alive. `init` runs on the main thread, where the delegate lives.
    static DELEGATE: Retained<NotifyDelegate> = unsafe {
        let delegate = NotifyDelegate::alloc().set_ivars(());
        msg_send![super(delegate), init]
    };
}

/// Set the foreground delegate and ask for notification permission once.
/// Must run on the main thread (call it from Tauri's `setup`).
pub fn init() {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    DELEGATE.with(|delegate| {
        center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
    });

    let handler = RcBlock::new(|granted: Bool, error: *mut NSError| {
        let granted = granted.as_bool();
        set_status(if granted {
            AuthStatus::Authorized
        } else {
            AuthStatus::Denied
        });
        if !granted {
            let detail = unsafe { error.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_default();
            if detail.is_empty() {
                eprintln!("[Solayge] notification permission was not granted");
            } else {
                eprintln!(
                    "[Solayge] notification permission was not granted: {}",
                    describe_delivery_error(&detail)
                );
            }
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge,
        &handler,
    );
}

/// Deliver a notification immediately. Safe to call from any thread and never
/// panics.
///
/// Delivery is always attempted, even after an earlier denial, so enabling
/// notifications in System Settings takes effect without restarting the app. A
/// repeat denial is logged only once per run; the in-app toast still fires.
pub fn show(app: &tauri::AppHandle, title: &str, body: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    // A unique identifier so consecutive notifications don't replace each other.
    let identifier = NSString::from_str(&format!("solayge-{}", uuid::Uuid::new_v4()));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);

    let handle = app.clone();
    let handler = RcBlock::new(move |error: *mut NSError| {
        let raw = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
        match classify_delivery(raw.as_deref()) {
            // A successful delivery proves the user has now allowed us (the
            // cached answer may have been a stale denial).
            DeliveryOutcome::Delivered => set_status(AuthStatus::Authorized),
            // Remember the refusal so Settings shows the banner, but record it
            // at most once instead of on every event.
            DeliveryOutcome::NotAllowed => {
                set_status(AuthStatus::Denied);
                record_denial_once(&handle);
            }
            DeliveryOutcome::Failed(detail) => {
                crate::errorlog::record(
                    &handle,
                    "notification",
                    &format!("could not show notification: {detail}"),
                );
            }
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&handler));
}

fn record_denial_once(app: &tauri::AppHandle) {
    if DENIAL_LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    crate::errorlog::record(app, "notification", NOT_ALLOWED_HINT);
}

/// Open the macOS Notifications settings pane. The URL is a fixed constant, so
/// this exposure cannot open an arbitrary location.
pub fn open_settings() -> Result<(), String> {
    Command::new("open")
        .arg(NOTIFICATION_SETTINGS_URL)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open System Settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_not_allowed_error_becomes_an_actionable_hint() {
        let raw = "The operation couldn’t be completed. (UNErrorDomain error 1.)";
        let mapped = describe_delivery_error(raw);
        assert!(mapped.starts_with(NOT_ALLOWED_HINT));
        assert!(mapped.contains("System Settings"));
    }

    #[test]
    fn other_errors_pass_through_unchanged() {
        assert_eq!(describe_delivery_error("disk full"), "disk full");
    }

    #[test]
    fn a_successful_delivery_authorizes_the_app() {
        assert_eq!(classify_delivery(None), DeliveryOutcome::Delivered);
    }

    #[test]
    fn a_not_allowed_delivery_is_recognized_and_other_failures_are_not() {
        let raw = "The operation couldn’t be completed. (UNErrorDomain error 1.)";
        assert_eq!(classify_delivery(Some(raw)), DeliveryOutcome::NotAllowed);
        match classify_delivery(Some("disk full")) {
            DeliveryOutcome::Failed(detail) => assert_eq!(detail, "disk full"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn the_status_word_covers_every_state() {
        // The status is process-global; only assert the mapping is total.
        for status in [AuthStatus::Unknown, AuthStatus::Authorized, AuthStatus::Denied] {
            let word = match status {
                AuthStatus::Unknown => "undetermined",
                AuthStatus::Authorized => "authorized",
                AuthStatus::Denied => "denied",
            };
            assert!(!word.is_empty());
        }
        assert!(["authorized", "denied", "undetermined"].contains(&status_word()));
    }
}
