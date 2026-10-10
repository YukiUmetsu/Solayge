# Firebase Cloud Messaging setup for Solayge on Android (from zero)

This is the complete, start-to-finish guide to making Solayge deliver
background push notifications to an Android phone. It assumes you have **never**
created a Firebase project. By the end you will have:

* a Firebase project with an Android app for `com.solayge.mobile`;
* `google-services.json` in the mobile build;
* a Firebase **service-account** key on the desktop;
* the desktop running its own push gateway automatically;
* a paired phone receiving a notification when the app is closed or killed.

For how the pieces fit together and the privacy model, read
[PUSH_NOTIFICATIONS.md](./PUSH_NOTIFICATIONS.md) alongside this.

---

## 0. What you are building

```
desktop (Solayge)                          phone (Solayge Mobile)
 ┌─────────────────────────────┐            ┌──────────────────────────────┐
 │ agent event                 │            │ SolaygeMessagingService (FCM)│
 │  → push dispatch            │            │  → system notification       │
 │  → embedded push gateway ───┼── FCM ─────┤  → tap deep-links            │
 │    (127.0.0.1, auto-started)│            │  → reconnect over Iroh       │
 └─────────────────────────────┘            └──────────────────────────────┘
```

The desktop runs a small gateway on loopback. It authenticates to itself with a
generated bearer token and uses a Firebase **service account** to call the FCM
HTTP v1 API. The phone's FCM token is registered with the desktop over the
already-authenticated Solayge protocol. **Only opaque identifiers are sent to
FCM** — never prompts, code, tool output, or results.

---

## 1. Prerequisites

| Tool | Why | Notes |
|---|---|---|
| A Google account | Owns the Firebase project | Free Spark plan is enough for FCM |
| JDK 17 | Android Gradle build | `brew install openjdk@17` on macOS |
| Android SDK + NDK | Build the native app | Android Studio's SDK Manager is easiest |
| Rust Android targets | Compile the Tauri shell | `rustup target add aarch64-linux-android` (and `armv7`, `i686`, `x86_64`) |
| Node + pnpm | Build the React front end | As used by the repo |
| A physical Android device (Android 13+ recommended) | Real FCM behaviour | Emulators work but are less realistic |

The repo pins the NDK in the build docs; check the current
`src-tauri/gen/android` Gradle files if a version changes.

---

## 2. Create the Firebase project and Android app

1. Open the [Firebase console](https://console.firebase.google.com/) and click
   **Add project**. Give it a name (for example `solayge-push`). Google
   Analytics is optional and not required.
2. In the project, click the **Android** icon to **Add app**.
3. **Android package name:** `com.solayge.mobile` — this must match the app id
   exactly (see `apps/mobile/src-tauri/gen/android/app/build.gradle.kts`).
4. **App nickname:** anything (for example `Solayge Mobile`).
5. **Debug signing certificate SHA-1:** leave blank. FCM does not need it; it is
   only for Google Sign-In and similar APIs.
6. Click **Register app**, then **Download `google-services.json`**.

> If you later use Google Sign-In or App Check, you must add the SHA-1 of your
> signing key and re-download this file. For push notifications alone it is not
> needed.

---

## 3. Put `google-services.json` in the mobile build

Place the downloaded file here (create the folder if needed):

```
apps/mobile/src-tauri/gen/android/app/google-services.json
```

* This file contains **public client configuration** (project id, API key,
  sender id). It is **not** a secret, but it is git-ignored in this repository
  because it is environment-specific — do not commit it.
* The Gradle build applies the `com.google.gms.google-services` plugin
  automatically **only when the file is present**
  (`apps/mobile/src-tauri/gen/android/app/build.gradle.kts`). Without it the app
  still builds and runs, just without FCM.

Sanity check after a build:

```sh
# Entries named google_app_id and gcm_defaultSenderId should be generated.
grep -r "google_app_id\|gcm_defaultSenderId" \
  apps/mobile/src-tauri/gen/android/app/build/generated/res/ 2>/dev/null | head
```

---

## 4. Generate the service-account key (desktop side)

This is a **different file** from `google-services.json`. It is a private key and
must stay on the desktop. Never commit it, never ship it to the phone.

1. In the Firebase console open **Project settings** (gear) → **Service
   accounts**.
2. Click **Generate new private key**, confirm, and download the JSON.
3. Move it to a private location, for example:

   ```sh
   mkdir -p ~/.solayge
   mv ~/Downloads/solayge-push-*.json ~/.solayge/fcm-service-account.json
   chmod 600 ~/.solayge/fcm-service-account.json
   ```

The JSON looks like this (keys abbreviated):

```json
{
  "type": "service_account",
  "project_id": "solayge-push",
  "private_key_id": "…",
  "private_key": "-----BEGIN PRIVATE KEY-----\n…\n-----END PRIVATE KEY-----\n",
  "client_email": "firebase-adminsdk-…@solayge-push.iam.gserviceaccount.com",
  "token_uri": "https://oauth2.googleapis.com/token"
}
```

Make sure the **Firebase Cloud Messaging API** is enabled for that project
(Google Cloud console → APIs & Services → Enabled APIs). It is normally enabled
automatically when Firebase Cloud Messaging is first used. If sends fail with
`403`/`PERMISSION_DENIED`, enable it explicitly.

---

## 5. Build and install the Android app

From the repository root:

```sh
export JAVA_HOME="/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home"
export ANDROID_HOME="$HOME/Library/Android/sdk"
export NDK_HOME="$ANDROID_HOME/ndk/27.0.12077973"   # match your installed NDK

# Register the Rust Android targets once.
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

# First run / development build on a connected device:
pnpm --filter @solayge/mobile tauri android dev

# Or a release APK:
pnpm --filter @solayge/mobile tauri android build --apk
```

During the build, confirm the `google-services` plugin was applied and the
generated resources contain `google_app_id`.

---

## 6. Configure the desktop (one-time)

The desktop starts its embedded gateway automatically once it has the service
account; there is nothing to run in a terminal.

1. Launch the Solayge desktop app.
2. Open **Settings → Devices**.
3. Under **Background push notifications**, click **Choose service account…**
   and select `~/.solayge/fcm-service-account.json`.
4. The **Enable background push** switch turns on and the status row shows
   something like `Push gateway  http://127.0.0.1:8787/v1/push`.
5. The desktop saves this (owner-only `push-gateway.json` in its app-data
   directory) and starts the gateway on every launch from now on.

App-data locations (where `push-gateway.json`, `push-tokens.json`, and
`connected-devices.json` live):

| OS | Directory |
|---|---|
| macOS | `~/Library/Application Support/com.solayge.desktop/` |
| Windows | `%APPDATA%\com.solayge.desktop\` |
| Linux | `$XDG_DATA_HOME/com.solayge.desktop/` (usually `~/.local/share/...`) |

> **Advanced / remote gateway.** If you would rather host the gateway elsewhere,
> build and run `crates/solayge-push-gateway` (or `pnpm push:gateway` for a local
> one) and set `SOLAYGE_PUSH_GATEWAY_URL` and `SOLAYGE_PUSH_GATEWAY_TOKEN` in the
> desktop's environment. Environment configuration takes precedence over the
> embedded gateway. See [PUSH_NOTIFICATIONS.md](./PUSH_NOTIFICATIONS.md) §5.

---

## 7. Pair the phone and enable notifications

1. On the desktop: **Settings → Devices → Connected devices → Add phone**. Show
   the QR code.
2. On the phone: open Solayge Mobile, **Add desktop**, scan the QR, and approve
   the pairing on the desktop.
3. On the phone: **Settings → Notifications → Enable notifications**, accept the
   Android permission prompt, and choose which kinds you want
   (questions/permissions, completions, failures).
4. Keep the phone connected once so it can register its FCM token with the
   desktop. The desktop's **Background push notifications** panel should show a
   non-zero **Phones registered** count.

The phone sends its per-kind settings with the token. The desktop only pushes a
kind that device enabled; turn the master switch off and the token is
unregistered, so no background pushes are sent until it is turned back on.

---

## 8. Verify end to end

1. In **Settings → Devices**, click **Send test question push**. A successful
   result (for example `Sent a test question push to 1 device(s).`) means the
   desktop → gateway → FCM hop worked.
2. **Background the phone** (home button, do not swipe it away) and send the test
   again. Expect an "Agent has a question" notification.
3. **Kill the app** (swipe it out of Recents) and send the test again. This is
   the case local notifications cannot cover; FCM should still wake the app.
4. **Tap the notification.** The app should open the named session (or the
   approval) and reconnect to the desktop.
5. Start a real task, let it finish or fail, or trigger an agent question, and
   confirm a matching notification arrives according to the phone's settings.

> The test push deliberately ignores the per-kind settings so you can always
> verify the transport. Real events honor them.

---

## 9. How the phone stores things

| File | Location on Android | Purpose |
|---|---|---|
| `google-services.json` | build input only | Firebase client config |
| `push-token.json` | `<app data dir>/push-token.json` (`/data/user/0/com.solayge.mobile/`) | FCM registration token bridge to Rust |
| `push-tap.json` | same directory | A notification tap awaiting deep-link |
| notification channel `solayge` | Android settings | One channel for all alerts |

The Kotlin `FirebaseMessagingService` writes the token; the Rust shell reads it
and registers it with the desktop over the authenticated protocol. The tap file
is consumed once by the webview to navigate.

---

## 10. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| App builds but no `google_app_id` resource | `google-services.json` is in the wrong directory or the plugin did not run. Confirm the exact path in §3 and rebuild. |
| Panel shows `Phones registered 0` | Notifications are off on the phone, or the phone never connected since enabling them. Enable notifications and reconnect. |
| Panel shows `Push gateway Off` | No service account selected, or it could not be parsed. Re-select the file; check the JSON is a service-account key, not `google-services.json`. |
| Test push reports gateway unreachable | The embedded gateway failed to bind; another process may hold port 8787. It falls back to an ephemeral port automatically — reopen Settings to refresh, or free the port. |
| `401 unauthorized` from the gateway | The desktop's stored token and the running gateway disagree (rare). Toggle **Enable background push** off and on to regenerate and restart. |
| FCM rejects a message (`502`) | Bad service account, FCM API disabled, or the project does not match `google-services.json`. Check §4 and that both files come from the **same** Firebase project. |
| A device silently stops receiving | Its FCM token was reported as unregistered (app reinstalled/cleared). The desktop forgets it automatically; reopen the phone app and reconnect to re-register. |
| The desktop keeps using a local gateway you no longer run | A leftover `push-dev.json` (written by `pnpm push:gateway`) overrides the embedded gateway. Delete it from the desktop app-data directory (§6), or set the environment variables explicitly. |
| Notification arrives late in Doze | Questions/permissions use FCM **high** priority; completions/failures use normal priority and may be deferred to the next maintenance window. |
| No notification at all on Android 13+ | `POST_NOTIFICATIONS` was denied. Allow notifications for Solayge in the phone's app settings. |
| Two notifications for one event | The phone's dedup should prevent this; if it recurs, note the event id and report it. |

---

## 11. iOS (not yet supported)

This path is Android-first. iOS needs APNs credentials configured on the
gateway and an `AppDelegate`/notification handler in the mobile shell. The
gateway payload is already platform-neutral, so the desktop and gateway code can
be reused when iOS work begins.

---

## 12. Security checklist

* `fcm-service-account.json` exists only on the desktop (owner-only, `0600`).
  Never commit it and never copy it to a phone.
* `google-services.json` contains public config and is git-ignored.
* A gateway bearer token is generated on the desktop and stored owner-only in
  `push-gateway.json`. It is never logged or displayed.
* Only opaque ids (`event_id`, `desktop_id`, `session_id`, `approval_id`) and a
  kind transit FCM; the gateway rejects oversized, empty, or newline-containing
  identifiers so content cannot be smuggled through.
* Revoking a device on the desktop removes its token immediately and excludes it
  from future pushes. A revoked device cannot reconnect on any transport.
