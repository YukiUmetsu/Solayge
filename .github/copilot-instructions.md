# Copilot instructions

## Build and checks

Run commands from the repository root unless noted:

```sh
pnpm install
pnpm build                         # TypeScript check + Vite production build
pnpm tauri dev                     # Run the desktop app with frontend hot reload
pnpm tauri build                   # Package the app for the host OS
(cd src-tauri && cargo test)       # Rust unit tests
(cd src-tauri && cargo test NAME)  # Run matching Rust tests, e.g. cargo test save_load_round_trips
(cd src-tauri && cargo clippy --all-targets)
```

Rust tests are colocated in the modules under `src-tauri/src/`. The frontend
scripts currently provide build/dev/preview but no test or lint command.

## Architecture

Solayge is a Tauri 2 desktop app. The React/TypeScript UI in `src/` renders
projects and task graphs; Rust in `src-tauri/src/` owns persistence, scheduling,
process execution, Git/worktree operations, and provider integrations.

The frontend calls Rust through the typed wrappers in `src/api.ts`. A command
crosses three parts of the IPC contract: its Tauri handler in
`src-tauri/src/commands.rs`, registration in `src-tauri/src/lib.rs`, and
frontend wrapper/types in `src/api.ts` and `src/types.ts`. Keep the Rust
serialization and TypeScript shapes in sync.

`AppState` in `src-tauri/src/state.rs` holds the persisted app state and
runtime process handles. Project/task/settings data is saved as JSON in the
platform app-data directory; task logs live alongside it. The scheduler in
`src-tauri/src/scheduler.rs` ticks once per second to promote, dispatch, and
reap tasks, and emits state and live-log events consumed by `src/App.tsx`.
Task execution is split among provider/process logic (`agent.rs` and
`opencode_server.rs`), Git/worktree operations (`git.rs`), and task commands
(`commands.rs`).

## Repository-specific conventions

- Task status and lifecycle logic exists on both sides of the IPC boundary.
  When changing statuses, terminal/clearable behavior, dependencies, or review
  progression, check the Rust model/scheduler and the related UI helpers in
  `src/lib/tasks.ts` and `src/lib/format.ts`.
- Preserve the IPC naming and shape conventions: Rust command names are
  snake_case; frontend wrappers invoke those exact names and pass the argument
  names expected by the Tauri command. Update the shared TypeScript types when
  command payloads or results change.
- Rust behavior is tested with module-local `#[cfg(test)]` tests. Put focused
  tests alongside the implementation they cover and run them with a Cargo test
  name filter.
- User-facing feature or behavior changes should include a short README note,
  consistent with the contributing guidance in `README.md`.
