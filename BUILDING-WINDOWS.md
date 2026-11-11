# Building and running sy-term on Windows

This guide builds sy-term from source on a fresh Windows 10 (1809 or newer) or
Windows 11 x64 machine and walks through a manual smoke test. Commands are for
PowerShell unless noted.

## Option A: download a CI build

The **Windows build** workflow (`.github/workflows/windows-build.yml`) builds a
release `sy-term.exe` on every pull request and push to `master`, and on demand
from the repository's **Actions** tab (**Windows build** > **Run workflow**).

1. Open the workflow run and download the `sy-term-windows-x86_64` artifact
   (you need to be signed in to GitHub).
2. Unzip it, then unzip `sy-term-windows-x86_64.zip` inside it into a folder of
   your choice, for example `%LOCALAPPDATA%\Programs\sy-term`. Keep the
   `conpty` folder next to `sy-term.exe`; it is the bundled Microsoft ConPTY
   that sy-term verifies at startup.
3. `BUILD_INFO.txt` records the commit and the zip's SHA-256.

Then continue at [Run it](#4-run-it). Windows SmartScreen may warn about an
unsigned executable; choose **More info** > **Run anyway** if you trust the build.

## Option B: build from source

## 1. Install the prerequisites

1. **Git for Windows**: <https://git-scm.com/download/win>.
2. **Visual Studio 2022 Build Tools** with the **Desktop development with C++**
   workload, which includes the MSVC compiler and a Windows 10/11 SDK:
   <https://visualstudio.microsoft.com/visual-cpp-build-tools/>. A full Visual
   Studio install with the same workload also works.
3. **Rust** via rustup: <https://rustup.rs>. Accept the default
   `x86_64-pc-windows-msvc` host. You do not need to pick a version: the repo's
   `rust-toolchain.toml` pins 1.96.1, and rustup installs it on the first build.
4. **Zig 0.16.0** (exactly this version) for the vendored libghostty-vt:
   download `zig-x86_64-windows-0.16.0.zip` from
   <https://ziglang.org/download/>, then extract it, for example to
   `C:\tools\zig-0.16.0`. Tell the build where it is:

   ```powershell
   $env:ZIG = "C:\tools\zig-0.16.0\zig.exe"
   & $env:ZIG version   # must print 0.16.0
   ```

   To make this permanent, set `ZIG` as a user environment variable, or run
   `setx ZIG C:\tools\zig-0.16.0\zig.exe` and open a new terminal.

Open a new PowerShell window after installing so `git`, `cargo` and the MSVC
tools are on `PATH`.

## 2. Get the source

```powershell
git clone https://github.com/wctiger/herdr.git sy-term
cd sy-term
git checkout claude/remove-upstream-docs-release   # PR #2 head: includes PR #1
```

Use `claude/fork-sensitive-environment-customize-w8quih` instead to test PR #1
on its own.

## 3. Build

```powershell
cargo build --release --locked
```

The first build takes several minutes. It needs network access to the Rust
crate registry and, for Zig packages, to `deps.files.ghostty.org`,
`codeberg.org` and `github.com`. Zig caches those packages under
`vendor\libghostty-vt\zig-pkg` and its global cache, so later builds work
offline.

For an air-gapped machine, run the build once on a connected Windows machine,
then copy the whole checkout (including `vendor\libghostty-vt\zig-pkg`) and
`%LOCALAPPDATA%\zig` across.

The binary is `target\release\sy-term.exe`. Release builds use the
`sy-term` config directory; debug builds (`cargo build` without `--release`)
use `sy-term-dev`, so the two don't share sessions.

If the build fails with `zig executable not found` or a Zig version error,
`ZIG` is not set in this terminal. If it fails with missing `stdlib.h`,
`windows.h` or `link.exe`, the C++ Build Tools workload or the Windows SDK is
missing.

## 4. Run it

```powershell
$exe = (Resolve-Path target\release\sy-term.exe).Path
& $exe --version          # sy-term 0.9.5
& $exe --help
& $exe                    # launches the server and attaches the TUI
```

Optionally copy `sy-term.exe` to a folder on `PATH`, such as
`%LOCALAPPDATA%\Programs\sy-term`.

Run sy-term inside Windows Terminal for the best key and mouse handling.

| What | Where |
| --- | --- |
| Config file | `%APPDATA%\sy-term\config.toml` (`sy-term --default-config` prints a template) |
| Logs | `%APPDATA%\sy-term\sy-term-server.log`, `sy-term-client.log` |
| Verbose logs | set `$env:HERDR_LOG = "sy_term=debug"` before starting |
| Default shell | set `default_shell = "pwsh.exe"` (or `powershell.exe`, `cmd.exe`) under `[terminal]` |

sy-term uses the Windows system ConPTY unless an app-local ConPTY bundle sits
next to the executable. A plain source build has no bundle, so no extra setup
is needed.

## 5. Smoke test checklist

In the TUI:

- [ ] The sidebar shows a workspace and the pane runs your shell.
- [ ] `ctrl+b` then `c` opens a new tab; the mouse can click tabs and panes.
- [ ] Split a pane (right-click a pane, or `ctrl+b` then `?` to see the keys).
- [ ] `ctrl+b` then `q` detaches; running `sy-term` again reattaches with the
      panes still running.
- [ ] The help, settings and notification UI say sy-term, not herdr.

From a second PowerShell window, while the TUI is running:

```powershell
sy-term status
sy-term workspace list
sy-term pane list
sy-term workspace create --label smoke --cwd $HOME
sy-term server stop      # stops the server and its panes
```

Removed online features should be gone:

```powershell
sy-term update           # unknown command
sy-term machine list     # unknown command
sy-term --remote host    # unknown option
sy-term plugin install x # unrecognized subcommand; plugin link/list/uninstall remain
```

Confirm it makes no network connections. Start `sy-term`, use it for a minute,
then in another window run:

```powershell
$ids = (Get-Process sy-term -ErrorAction SilentlyContinue).Id
Get-NetTCPConnection | Where-Object OwningProcess -in $ids
Get-NetUDPEndpoint  | Where-Object OwningProcess -in $ids
```

Both should print nothing. sy-term talks to its own processes over local named
pipes, which these commands don't list.

## 6. Optional: run the test suite

The full test suite matches what CI runs on Windows. It needs `just`
(`cargo install just`) and Python 3:

```powershell
just check
```

To try the Microsoft-packaged ConPTY that upstream ships, see
`scripts\package_windows_conpty.ps1`; it downloads the
`Microsoft.Windows.Console.ConPTY` NuGet package, so it needs network access.

## Reporting problems

Note the Windows version (`winver`), the terminal you used, the exact command,
and attach the server and client logs from `%APPDATA%\sy-term\`.
