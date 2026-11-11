# sy-term

Offline terminal based agent runtime for coding agents. sy-term is a fork of
[herdr](https://github.com/herdrdev/herdr) with every network-dependent feature
removed. Do not add features that contact the network (update checks, remote
catalogs, telemetry, SSH transports or downloads at runtime).

## Principles

- **State is separated from runtime.** `AppState` is pure data, testable without PTYs or async. `PaneState` is separate from `PaneRuntime`. Workspace logic doesn't need real terminals.
- **Render is pure.** `compute_view()` handles geometry and mutations. `render()` takes `&AppState` and only draws. Never mutate state during render.
- **No god objects.** If a module is doing too many things, split it. `app/` is already split into state, actions, and input. Keep it that way.
- **Platform code is isolated.** OS-specific behavior lives in the matching `src/platform/<os>.rs` file, with only shared traits, types, wrappers, and testable contracts in `src/platform/mod.rs`. Core modules don't have `#[cfg(target_os)]`.
- **Detection is decoupled.** The detector reads a screen snapshot, never touches the parser or viewport state.
- **Screen detection is evidence-based.** When changing `src/detect/manifests/`, first capture the relevant bottom-buffer state with `sy-term agent read <pane> --source detection --format text` and, when styling or alternate screen behavior matters, `--format ansi`. Decide which visible controls are invariant, which are alternatives, and encode them as explicit AND/OR gates. Do not match whole-pane incidental text, and do not use the user-visible viewport for agent status because users can scroll it.
- **UI patterns should be reused.** sy-term is a mouse-first TUI. New dialogs, onboarding and settings flows should follow the existing UI/UX language and interaction patterns instead of inventing one-off screens.

## Multiplicative performance paths

Treat work reachable from view computation, rendering, background-pane resizing,
PTY parsing, detection, and client frame fanout as multiplicative. Before adding
work, identify its frequency and cardinality: per byte, event, or render × panes,
tabs, or workspaces × attached clients.

Inside pane-scaled render and layout loops:

- Use narrow terminal-state accessors. Do not collect aggregate input state,
  format terminal snapshots, inspect process trees, perform filesystem I/O, or
  allocate when one scalar fact is enough.
- Keep terminal-core lock duration minimal.
- Preserve hidden-source and retained-render early exits.
- When a change adds or widens work in one of these loops, profile fixed geometry
  with 1 and at least 15 populated panes and report the scaling delta. Use
  `just bench-render-scale`.

Prefer deterministic operation or architecture tests to wall-clock CI limits.

## Runtime/client boundary

The server owns runtime state; the TUI is one client. Before adding state, API
fields, events, commands, or socket messages, classify the feature:

- Shared runtime/session fact: belongs in server state and should be exposed through the JSON API/event path when practical.
- TUI presentation state: belongs only in the TUI/client layer.

Do not add shared behavior that only works through the private TUI client socket. Use neutral server/API names, not UI-surface names like sidebar, row, card, or widget.

## Compatibility identifiers

Keep these herdr names unchanged unless a change explicitly migrates them:

- `HERDR_*` environment variables.
- `herdr:<agent>` hook source labels and installed integration asset names.
- The `herdr-plugin.toml` plugin manifest and `min_herdr_version` field.
- API values and the JSON schema in `api/sy-term-api.schema.json`.

### Stable client endpoint contract

- Named core codecs are immutable. Do not add, remove, reorder, or reinterpret fields or enum variants reachable from a published codec. Introduce a new codec name instead.
- Keep baseline JSON handshake and snapshot fields required. New JSON fields must be optional or have field-specific defaults; new enum values need an `Unknown` fallback.
- Frozen endpoint fixtures, bincode digests, wire-tag tests, and `tests/fixtures/endpoint-method-shapes-v1.json` are compatibility contracts. Never update a generation-1 expectation merely to bless a wire change.

## Testing

Use `just` recipes by default instead of invoking cargo or scripts directly.

```bash
just test               # cargo nextest + maintenance script tests
just check              # formatting check + cargo nextest + maintenance script tests
```

Run `just check` before committing. Do not bypass failing checks; fix the failure or explain exactly why a narrower check is enough.

Building requires Zig 0.16.0 for the vendored libghostty-vt (set `ZIG` to its path if needed). Windows MSVC cross-compilation from Unix uses `just setup-windows-cross` and `just windows-lint`.

Unit tests live next to the code (`#[cfg(test)] mod tests`). New `AppState` or `Workspace` behavior should be testable with `AppState::test_new()` and `Workspace::test_new()` without PTYs. Identity/state refactors should use `AppState::assert_invariants_for_test()` or `Workspace::assert_invariants_for_test()` with adversarial state from `AppState::test_with_adversarial_identity_state()` or `Workspace::test_adversarial_identity_state()`.

When testing a new build from inside an existing sy-term session, clear inherited socket overrides so the debug binary talks to the debug `sy-term-dev` server:

```bash
env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- <command>
```

## Agent detection updates

Use the manifest hot-reload loop. Use the project-local `herdr-throwaway-repro` skill to create a disposable named session and drive the real agent UI into the target state. Read the pane with `sy-term agent read <pane> --source detection --format text` and inspect matching with `sy-term agent explain <pane> --json`. Update the bundled manifest in `src/detect/manifests/<agent>.toml`, copy it to `~/.config/sy-term/agent-detection/<agent>.toml`, then run `sy-term server reload-agent-manifests`. Check for a pre-existing override first and never overwrite or remove one without alignment; remove the temporary override when done.

Unit-test the detection engine, not individual CLI agents' screen conventions. Use synthetic manifests and minimal input strings. Validate agent-specific behavior with live smoke tests.

## Vendored libghostty-vt

`vendor/libghostty-vt.vendor.json` records the vendored upstream commit. Local patches must be listed in `vendor/libghostty-vt.patches.md` and stored under `vendor/patches/libghostty-vt/`, each with its reason, base commit, touched files, verification and removal condition. `just check` verifies that indexed patches reverse-apply cleanly.

## Commit style

Use lowercase conventional commits with descriptive subjects and no emojis.

## Code conventions

- Rust: no `unwrap()` in production code. Use `tracing` for logging. Use `#[allow]` only with a comment explaining why.
- Platform-specific code must be compile-gated with `#[cfg(windows)]`, `#[cfg(unix)]` or target-specific `#[cfg(...)]`. Use `cfg!(...)` only for pure cross-platform policy constants.
- Don't add dependencies without a reason.
- Integration asset versions (`HERDR_INTEGRATION_VERSION` markers and matching `*_INTEGRATION_VERSION` constants) are migration versions; bump them once per release when an asset changes.
- When changing the server/client wire protocol incompatibly, bump `src/protocol/wire.rs::PROTOCOL_VERSION` and update protocol expectations and fixtures in tests.
