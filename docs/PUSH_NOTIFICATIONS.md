# Solayge Mobile: background push notifications (Android)

This document describes the FCM-based background notification path, how it
relates to the existing local notifications, its security model, the setup
steps, and what still has to be verified on a physical device.

> **Status:** production path implemented and unit-tested on the desktop,
> protocol, gateway, and TypeScript layers. Background delivery was verified
> end-to-end on a physical Android phone (see
> [Verification status](#verification-status)). The desktop now embeds and
> auto-starts the gateway, and per-device notification settings are honored.
> For a step-by-step setup see [FCM_SETUP_ANDROID.md](./FCM_SETUP_ANDROID.md).

## 1. Why local notifications are not enough

The app already shows notifications two ways:

* the desktop `notify()` path shows an OS notification on the computer, and
* the mobile Rust monitor polls a connected desktop and posts a **local**
  notification on the phone.

Both require a live process. Android suspends or kills a backgrounded app, so a
local notification cannot arrive when the app is not running. Reliable delivery
needs the OS to wake the app: **Firebase Cloud Messaging (FCM)**.

## 2. Architecture

```
 desktop (source of truth)                gateway                     phone
 ┌───────────────────────┐   HTTPS+token  ┌──────────────────┐  FCM   ┌────────────────────┐
 │ agent event           │ ─────────────► │ /v1/push         │ ─────► │ SolaygeMessaging   │
 │ notify() → push::     │  ids only      │ auth + map to    │        │ Service (Kotlin)   │
 │ dispatch()            │                │ FCM HTTP v1      │        │  dedup + notify    │
 └──────────┬────────────┘                └────────┬─────────┘        └─────────┬──────────┘
            │                                       │ service-account            │ tap
            │ device registry (push tokens)         │ (gateway only)             ▼
            │                                       │                  open session/approval
            ▼                                       ▼                  over Iroh/Tailscale
      Iroh / Tailscale ◄─────────────────────────────────────────────────────────┘
      (all real content: prompts, code, results)
```

* **Desktop** is authoritative. On an event it posts a minimal alert to the
  gateway for each authorized device that registered a token **and enabled that
  event kind**.
* **Gateway** (`crates/solayge-push-gateway`) authenticates the desktop with a
  bearer secret, maps the alert to a generic FCM message, and sends it. The
  desktop normally **embeds and auto-starts** this gateway on loopback when the
  user configures a Firebase service account (Settings → Devices → Background
  push notifications), so no separate process is required. It is the only
  component that reads the service-account key. A remote/shared gateway can
  still be selected with environment variables.
* **FCM** wakes the app even when it is not running.
* **Android service** (`SolaygeMessagingService`) shows a system notification
  when the app is backgrounded, deduplicating by event id. A tap deep-links to
  the session/approval; the app reconnects over Iroh/Tailscale to read the real
  content.

### Payload contract (minimal identifiers only)

Desktop → gateway (`POST /v1/push`):

```json
{
  "token": "<fcm registration token>",
  "event": {
    "event_id": "desk_1:se:ses_42:task_completed",
    "kind": "task_completed",
    "desktop_id": "desk_1",
    "session_id": "ses_42",
    "approval_id": null
  }
}
```

`kind` is one of `task_completed`, `task_failed`, `question`, `approval`.
**No prompts, code, credentials, tool output, or message content are ever sent.**
The gateway validates that identifiers are non-empty, bounded (≤256 chars), and
newline-free, so free text cannot be smuggled through.

The gateway turns this into an FCM message whose title/body are generic
(`"Task completed"`, `"Agent has a question"`, …) and whose `data` holds only
the identifiers above. `question` and `approval` use FCM **high priority**
because they are waiting on the user; completions and failures are normal
priority so they do not wake a sleeping device unnecessarily.

## 3. Security model

* **Service-account isolation.** The Firebase service-account JSON lives only on
  the gateway. It is never compiled into, shipped with, or stored by the desktop
  or mobile app.
* **Gateway authentication.** The desktop presents a bearer secret
  (`PUSH_GATEWAY_TOKEN`). Comparison is constant-time. An unconfigured gateway
  refuses every request.
* **No content in the cloud.** Only opaque ids transit the gateway and FCM.
  Real content is read peer-to-peer over Iroh/Tailscale after reconnect.
* **Token handling.** The phone's FCM token and its per-kind notification
  settings are registered with the desktop over the authenticated protocol
  (`push.register`). The desktop stores them in `push-tokens.json` (owner-only).
  The token is forwarded to the gateway as an addressing hint, never displayed
  or logged.
* **Embedded token.** When the embedded gateway is used, the desktop generates a
  bearer token and stores it owner-only in `push-gateway.json`. The desktop
  authenticates to its own loopback gateway with it; it is never logged.
* **Revocation.** Revoking a device on the desktop removes its push token and
  excludes it from `push_targets()`, so it stops receiving background alerts
  immediately. A token FCM reports as unregistered (`410 Gone`) is also cleared
  automatically so a reinstalled app's stale address is not retried. The gateway
  never stores a token beyond a single request.
* **Untrusted input.** The gateway treats the request as untrusted and the phone
  treats the push as an untrusted hint: it never renders push-provided text as
  content, only generic titles and identifiers.

## 4. Deduplication

The same logical event can be seen by more than one path (local poll, local Rust
monitor, FCM). Deduplication uses a canonical event id:

* approval: `"{desktop_id}:ap:{approval_id}"`
* session: `"{desktop_id}:se:{session_id}:{kind}"`

* The **desktop** computes it (`push::canonical_event_id`) and sends it.
* The **gateway** uses it as the FCM collapse tag, so a redelivery replaces the
  previous notification instead of stacking.
* The **Android service** keeps a bounded set of seen ids and drops repeats.
* While the app is in the **foreground**, the Kotlin service stays silent — the
  webview already shows an in-app toast from its own refresh.
* Once a device has an FCM token, the mobile Rust poller **stands down**
  (`poll_notifications` returns early): FCM owns background delivery, and the
  webview owns foreground toasts. This removes the local/pushed duplicate and
  stops the radio from waking on a timer. Without a token (FCM not set up), the
  Rust poller remains the fallback.

  > **Operational caveat.** The stand-down keys off the phone's FCM token, which
  > only proves FCM is configured *in the app* — not that the desktop has a
  > gateway (`SOLAYGE_PUSH_GATEWAY_URL`/`SOLAYGE_PUSH_GATEWAY_TOKEN`). If the app
  > ships with Firebase but the desktop gateway is unset, the desktop drops every
  > push and the poller has already stood down, so **no** notification arrives.
  > Configure the gateway on the desktop whenever a build enables FCM.

### Bridge files and the Android data directory

The Kotlin service and the Rust shell exchange two small JSON files
(`push-token.json`, `push-tap.json`). They must live in the **same** directory,
and on Android that directory is Tauri's `app_data_dir()`, which resolves to
`Context.getDataDir()` — the app's package directory (for example
`/data/user/0/com.solayge.mobile`). Tauri's Android resolver, unlike the desktop
resolver, does **not** append the bundle identifier, so the Kotlin helper must
write to `context.dataDir` directly, not to a `<dataDir>/<bundleIdentifier>`
subdirectory. Writing to the subdirectory silently strands the token and tap
files where Rust never reads them, which disables FCM registration and
deep-linking without any error.

## 5. Setup

The full, from-zero walkthrough is in
[FCM_SETUP_ANDROID.md](./FCM_SETUP_ANDROID.md). In short:

### 5.1 Firebase project (once)

1. Create a Firebase project and add an **Android** app with package name
   `com.solayge.mobile`.
2. Download `google-services.json` and place it at
   `apps/mobile/src-tauri/gen/android/app/google-services.json`. It contains
   only public client configuration and **is not committed**.
   The Gradle build applies the `google-services` plugin automatically when the
   file is present; without it the app still builds and runs, just without FCM.
3. In **Project settings → Service accounts**, generate a new private key. This
   JSON is the gateway credential. Store it on the desktop only; never commit it
   and never copy it to a phone.

### 5.2 Desktop (embedded gateway — recommended)

**Settings → Devices → Background push notifications → Choose service account…**
and select the service-account JSON. The desktop validates it, persists an
owner-only `push-gateway.json` (with a generated bearer token), and starts the
gateway on `127.0.0.1:8787` (falling back to an ephemeral port if taken). It
restarts automatically on every launch while enabled. Use **Check gateway** and
**Send test question push** to verify.

### 5.3 Remote / shared gateway (optional)

To point the desktop at a gateway elsewhere, run the standalone binary:

```sh
export PUSH_GATEWAY_TOKEN="$(openssl rand -hex 32)"          # desktop↔gateway secret
export GOOGLE_APPLICATION_CREDENTIALS=/etc/solayge/fcm-service-account.json
export PUSH_GATEWAY_BIND=127.0.0.1:8787                      # put TLS in front
cargo run -p solayge-push-gateway
```

Health check: `GET /healthz`. Push endpoint: `POST /v1/push`. Deploy behind TLS
(the desktop posts over HTTPS); the gateway is stateless and can be scaled
horizontally.

Then set the same URL/token in the desktop process environment:

```sh
export SOLAYGE_PUSH_GATEWAY_URL="https://push.example.com/v1/push"
export SOLAYGE_PUSH_GATEWAY_TOKEN="<the same PUSH_GATEWAY_TOKEN>"
```

Environment configuration takes precedence over the embedded gateway. A manual
`push-dev.json` (written by `pnpm push:gateway`) is the next precedence, and the
embedded gateway is the final fallback. With none configured, push dispatch is a
no-op and local notifications keep working.

### 5.4 Mobile

Nothing to configure beyond installing the app from a build that includes
`google-services.json`. On first use, enable notifications in **Settings →
Notifications**; this requests the Android 13+ `POST_NOTIFICATIONS` permission
and creates the `solayge` channel. The chosen per-kind settings are sent to the
desktop at registration.

## 6. Manual steps to test on a Pixel 8a

These are the steps that cannot be automated here.

1. **Build.** Install JDK 17, Android SDK, NDK, and the Rust Android targets,
   then:
   ```sh
   export JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
   export ANDROID_HOME="$HOME/Library/Android/sdk"
   export NDK_HOME="$ANDROID_HOME/ndk/27.0.12077973"
   pnpm --filter @solayge/mobile tauri android dev      # or android build
   ```
   Confirm `google-services.json` is present and the build logs show the
   `google-services` plugin applied.
2. **Permission.** On first launch, open Settings → Notifications, turn them on,
   and accept the system prompt. Confirm a test notification appears
   (`Send test notification`).
3. **Register.** Pair the phone with the desktop. Confirm the desktop has a
   token (`push-tokens.json`) after the app connects.
4. **Background, app alive.** Start a task on the desktop, send the phone to the
   background (do not swipe it away), and let the task finish. Expect a "Task
   completed" notification.
5. **Background, app killed.** Swipe the app away from Recents. Trigger a
   question/approval or completion on the desktop. Expect an FCM notification
   within a few seconds. This is the key case local notifications cannot cover.
6. **Deep link.** Tap the notification. The app should open the correct session
   or approval and reconnect to the desktop.
7. **High priority.** A question/approval should arrive promptly even in Doze;
   a completion may be deferred. Verify the notification channel importance.
8. **Dedup.** Trigger the same event twice (or toggle airplane mode to force a
   redelivery). Only one notification should remain.
9. **Revocation.** Revoke the phone on the desktop. Confirm no further
   background alerts arrive, and reconnecting fails.
10. **Do not disturb / denied permission.** Deny `POST_NOTIFICATIONS` and
    confirm the app degrades gracefully (no crash, in-app state still correct).

### 6.1 On-demand test (production)

Waiting for a real agent to ask a question is slow, so the desktop exposes a
**Send test question push** button. It fires a synthetic `question` push through
the same production path (gateway bearer auth → FCM HTTP v1 → Android
notification with high priority), so a green result means the real path works.

Use it from **Settings → Devices → Background push notifications**. The optional
**Session id** field makes the phone open that session when the notification is
tapped. Each call uses a unique event id so the phone's deduplication does not
swallow a repeated test, and it deliberately bypasses the per-device kind
settings so the transport can always be verified. Without a running gateway or a
token in `push-tokens.json`, it reports the missing piece instead of sending.

`pnpm push:gateway` still exists for running a standalone loopback gateway and
writing the `push-dev.json` override used for development; it is no longer
required, because the desktop embeds the gateway.

## 7. Verification status

Verified here (automated):

| Layer | Check | Result |
|---|---|---|
| `solayge-protocol` | unit + cross-language fixture tests (incl. `push.register` preferences) | pass |
| `solayge-push-gateway` | routing, auth, payload, priority, unregistered-token, embedded-server tests | pass |
| `solayge` desktop | 245 lib tests (incl. host handshake, push filtering/token pruning, embedded gateway lifecycle) | pass |
| `solayge-mobile` (Rust) | `cargo check` | pass |
| mobile TypeScript | `tsc` + 114 vitest tests | pass |
| desktop TypeScript | `tsc` + 106 vitest tests + production `vite build` | pass |
| shared types | build + fixture tests | pass |
| Android (debug APK) | `tauri android build --debug --apk --target aarch64` with `google-services.json` present; merged manifest carries `POST_NOTIFICATIONS`, `SolaygeMessagingService`, and the `solayge` default channel; `google_app_id`/`gcm_defaultSenderId` resources generated | pass |
| Android (physical device) | Installed the debug APK, launched, and confirmed the native `FirebaseMessagingService` wrote a live FCM registration token to `<app_data_dir>/push-token.json` (the directory Rust reads), and that a notification-tap intent round-trips through `<app_data_dir>/push-tap.json` and is consumed by the webview | pass |

**Device verification (reported by the team):** an end-to-end FCM push was
delivered to a physical Android phone — including a backgrounded/killed app —
and the notification was displayed and deep-linked. The automated layers above
pass. The remaining item is iOS (see
[Known limitations](#8-known-limitations--follow-ups)); do not treat iOS
background delivery as working.

## 8. Known limitations / follow-ups

* **iOS:** this path is Android-first. iOS needs APNs credentials on the gateway
  and an `AppDelegate` push handler; the payload contract is already
  platform-neutral.
* **Per-kind preferences and the push path:** the phone sends its
  `onQuestions`/`onCompleted`/`onFailed` preferences with `push.register` (and
  re-registers whenever they change), so the desktop only pushes a kind that
  device enabled. The master switch is also honored: turning notifications off
  unregisters the token, so the desktop stops pushing entirely.
* **Missed-event sync:** on reconnect the app refreshes its full snapshot and
  re-reads content over Iroh; there is no incremental event-replay cursor yet
  (`sessions.events.subscribe` still returns an empty batch).
* **Desktop settings UI:** the embedded gateway is configured in
  **Settings → Devices → Background push notifications** (enable, service
  account, test). A remote gateway remains environment-configured
  (`SOLAYGE_PUSH_GATEWAY_URL`/`_TOKEN`), which takes precedence.
* **Sound is per channel:** Android notification sound is a property of the
  `solayge` channel, created from the phone's sound preference; the gateway
  cannot change it per message.
* **Foreground duplicate suppression:** FCM stands down in the foreground and
  the Rust poller stands down once a token exists, so an event is announced by
  exactly one path. A notification message delivered while the app is
  backgrounded is shown by the system, not the service, so the service's
  seen-id set is not consulted for it; the gateway's collapse tag prevents
  redeliveries from stacking.
