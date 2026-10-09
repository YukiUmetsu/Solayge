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
  - `mock.ts` — an in-memory loopback client seeded like the Rust `MockHost`,
    used for local development and every UI test. Networking, identity, and
    host logic are intentionally **not** implemented here.
  - `ClientContext.tsx` — connection state, cached last-known data, transport
    preference, and the offline/read-only projection.
- `src-tauri/` — the Rust side (`solayge_mobile_lib`). Registers the official
  barcode-scanner plugin on mobile and exposes `protocol_version`.
- `src-tauri/capabilities/default.json` — capability granting `core` plus the
  barcode-scanner commands.
- `src-tauri/Info.ios.plist` — iOS `NSCameraUsageDescription` (merged into the
  generated Xcode project at build time).
- `src-tauri/gen/` — generated Android (`android/`) and iOS (`apple/`) projects.

The P0 information architecture is validated only against the in-memory
loopback client: transport, pairing identity, host authorization, and agent
control are later phases. See `docs/MOBILE_FEATURES.md` sections 7 and 9.

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
