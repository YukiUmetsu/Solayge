# Solayge Mobile

The Tauri 2 companion client (React + TypeScript) for Solayge-managed agents. It
is a control surface: it pairs with, authenticates to, and drives a desktop host
through the shared [`solayge-protocol`](../../crates/solayge-protocol) contract,
and never runs coding agents itself.

## Layout

- `src/App.tsx` — the shell: bottom tabs (Sessions / Inbox / Projects), a
  header desktop switcher, and a Settings entry; owns the route state.
- `src/screens/` — one component per screen: sessions home, session/chat
  detail, inbox, approval/question detail, projects list/detail, settings, and
  the first-launch desktop selector.
- `src/components/` — presentational pieces (status pills, composer, header,
  modal, offline banner, empty/error states).
- `src/client/` — the UI's only data boundary:
  - `remote.ts` — the TypeScript `RemoteClient` mirroring the Rust
    `solayge-protocol` trait, returning `@solayge/shared-types` DTOs.
  - `tauri.ts` — the real client: every call is one `remote_call` command the
    Rust side turns into an Iroh frame on the authenticated connection.
  - `mock.ts` — an in-memory loopback client used only by tests and local UI
    work; it is never the default host.
  - `ClientContext.tsx` — connection state, cached last-known data, transport
    preference, and the offline/read-only projection.
- `src/pairing.ts` — the identity/pairing command bindings (including the real
  `pair_with_qr` / `pair_poll` handshake over the transport).
- `src/AddDesktopFlow.tsx` — the step-by-step pairing wizard.
- `src-tauri/` — the Rust side (`solayge_mobile_lib`). It owns the phone's
  Ed25519 identity, the live authenticated `IrohRemoteClient`, and the persisted
  list of paired desktops (`paired-desktops.json`, owner-only). Registers the
  official barcode-scanner plugin on mobile and exposes the `remote_call` bridge.
- `src-tauri/capabilities/default.json` — capability granting `core` plus the
  barcode-scanner commands.
- `src-tauri/Info.ios.plist` — iOS `NSCameraUsageDescription` (merged into the
  generated Xcode project at build time).
- `src-tauri/gen/` — generated Android (`android/`) and iOS (`apple/`) projects.

The mobile app speaks the real Iroh transport against the desktop host: it pairs
with a QR code (or a pasted pairing link), compares the six-digit verification
code, waits for desktop approval, authenticates with a signed challenge, and
then reads the desktop's projects, tasks, and pending questions. Remote prompt
sends and approval responses are the next phase; those actions are reported as
unsupported rather than silently accepted. See `docs/MOBILE_FEATURES.md`
sections 7 and 9.

## Commands

```sh
pnpm install                                   # from the repository root
pnpm --filter @solayge/mobile typecheck
pnpm --filter @solayge/mobile build
pnpm --filter @solayge/mobile test
pnpm --filter @solayge/mobile tauri android dev   # requires the env below
pnpm --filter @solayge/mobile tauri ios dev
```

## Native prerequisites

Already initialized here. To re-run or build native targets, export:

```sh
export JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME="$HOME/Library/Android/sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export NDK_HOME="$ANDROID_HOME/ndk/27.0.12077973"
export PATH="$JAVA_HOME/bin:$PATH"

# Rust targets used by the native builds
rustup target add aarch64-linux-android armv7-linux-androideabi \
  i686-linux-android x86_64-linux-android \
  aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

# (Re)generate the native projects
pnpm --filter @solayge/mobile tauri android init --ci
pnpm --filter @solayge/mobile tauri ios init --ci
```

iOS builds also need Xcode and CocoaPods (`pod`).

## Android networking note

The Iroh transport resolves the desktop's endpoint through the public n0
discovery service, which uses the system DNS resolver. On Android that resolver
reads the system configuration over JNI; Tauri's Android runtime publishes the
process context, and iroh otherwise falls back to its bundled fallback
resolvers. If pairing cannot resolve the desktop on a particular device, treat
it as a diagnostics case: the connection still authenticates cryptographically,
so a wrong host cannot be trusted.
