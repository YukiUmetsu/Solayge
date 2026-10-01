//! Desktop notifications on macOS via the modern UserNotifications framework.
//!
//! `NSUserNotificationCenter` (what `notify-rust` / `mac-notification-sys`, and
//! therefore the Tauri notification plugin, use) is a "LegacyConnection" that
//! `usernoted` now denies outright — notifications built on it are silently
//! dropped. `UNUserNotificationCenter` is the supported API, so notifications
//! are posted from here instead.
//!
//! This is the only module allowed to touch Objective-C directly; everything
//! else stays safe Rust (`lib.rs` denies `unsafe_code`).

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_foundation::{NSObjectProtocol, NSString, NSError};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

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
            completion_handler.call((
                UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,
            ));
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
        if !granted.as_bool() {
            let detail = unsafe { error.as_ref() }
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_default();
            eprintln!("[Solayge] notification permission was not granted: {detail}");
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
        &handler,
    );
}

/// Deliver a notification immediately. Safe to call from any thread and never
/// panics; a denied notification is simply not shown (the in-app toast still is).
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
        if let Some(error) = unsafe { error.as_ref() } {
            let detail = error.localizedDescription().to_string();
            eprintln!("[Solayge] could not deliver notification: {detail}");
            crate::errorlog::record(
                &handle,
                "notification",
                &format!("could not show notification: {detail}"),
            );
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&handler));
}
