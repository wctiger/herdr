# sy-term

sy-term is an offline fork of [herdr](https://github.com/herdrdev/herdr), the
terminal workspace manager for AI coding agents. It keeps herdr's local terminal
runtime and removes every feature that needs a network connection, so it can be
used in sensitive or air-gapped environments.

sy-term is distributed under the [Apache License 2.0](LICENSE), the same license
as herdr. See [NOTICE](NOTICE) for attribution and a summary of the changes.

## What it does

- Keeps terminals running in a background server when the client detaches, and
  restores the saved layout after a restart.
- Marks every pane as working, blocked or idle, so agents that need an answer
  are easy to find.
- Lets agents drive panes, tabs and workspaces through the `sy-term` CLI and the
  local socket API.
- Supports tmux-style prefix keys as well as mouse click, drag and split.
- Runs local plugins linked from a directory on disk.

## Removed from herdr

These features contacted the network and are not present in sy-term:

- **Self-update and update checks:** `herdr update`, `herdr channel`, background
  version checks, update-ready notices and product announcements.
- **Remote machines over SSH:** `--remote`, `--machine`, the `machine` command,
  saved SSH machines in the sidebar, and the remote API/client bridges.
- **Online agent-detection catalog:** the background fetch from herdr.dev and
  `server update-agent-manifests`. Detection uses the bundled manifests and
  optional local overrides in `~/.config/sy-term/agent-detection/`.
- **GitHub plugin installs:** `plugin install` and `plugin update`. Use
  `sy-term plugin link <path>` for plugins on disk.

herdr has no telemetry, so there was none to remove.

## Compatibility notes

- Environment variables keep their `HERDR_` prefix (for example
  `HERDR_CONFIG_PATH`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH`), so existing agent
  hooks and scripts keep working.
- Protocol and file-format identifiers are unchanged: the `herdr:<agent>` hook
  source labels, the `herdr-plugin.toml` plugin manifest and its
  `min_herdr_version` field, and the socket API schema.
- Config, state, logs and sockets live under `sy-term` paths, for example
  `~/.config/sy-term/config.toml` and `sy-term-server.log`. Config files may use
  `delivery = "sy-term"` and `[ui.toast.sy-term]`; the herdr spellings still work.
- The log filter target is the crate name, so verbose logging is
  `HERDR_LOG=sy_term=debug` (upstream used `herdr=debug`).
- Upstream herdr's website docs, changelog and release tooling are not part of
  this fork; see the herdr repository for its history. `sy-term --help`,
  `sy-term --default-config` and `sy-term --skill` describe this build.

## Build

Building requires Rust (see `rust-toolchain.toml`) and Zig 0.16.0 for the
vendored libghostty-vt.

```bash
cargo build --release
./target/release/sy-term
```

The first build fetches Zig package dependencies. To build fully offline,
populate the Zig package cache on a connected machine first and copy it over.

See [BUILDING-WINDOWS.md](BUILDING-WINDOWS.md) for Windows prerequisites and a
smoke-test checklist.

```bash
just test        # unit tests
just check       # formatting, tests, and maintenance checks
```

## Usage

```bash
sy-term                 # launch or attach to the persistent session
sy-term --help          # list commands
sy-term --default-config
```

Press `ctrl+b q` to detach, and run `sy-term` again to reattach.

## License

sy-term is licensed under the [Apache License 2.0](LICENSE). It is a derivative
work of herdr; the original copyright notices are preserved.
