# Native WSL conversation preview

Status: experimental delivery under [ADR 0080](../decisions/0080-native-wsl-conversation-preview.md).
This is a separate native desktop executable, not Windows support for the complete
Prometeu application. It exposes one Codex conversation per workspace through the
[headless runtime](headless-runtime.md). The production desktop remains unchanged.

The native executable now defaults to the [shared desktop interface](windows-application.md)
under ADR 0084. The isolated interface documented here remains at `/wsl.html` for
transport diagnostics. `test:wsl:native` navigates to that route explicitly;
`test:windows:native` exercises the default shared interface. Neither is full
application parity yet.

## Run it

Use separate checkouts of this revision: one inside the selected WSL distribution
for the Linux executable, and one on Windows for the native desktop build.
The distribution must already have Rust and a working, authenticated Codex CLI.
In WSL, build the host (no Linux GUI packages required):

```sh
cargo build --manifest-path src-tauri/Cargo.toml -p prometeu-runtime --locked
realpath src-tauri/target/debug/prometeu-runtime
command -v codex
```

On Windows, install the repository's Node/npm versions, Rust with the MSVC
build tools, and WebView2. From the Windows checkout in PowerShell:

```powershell
npm ci
npm run app:wsl
```

The native form takes the installed distribution name (`wsl --list --quiet`),
absolute Linux runtime/Codex executable paths, a Linux project directory, and
an empty or previously used **preview** root. Use a separate directory such as
`/home/you/.local/share/prometeu-wsl-preview`; the host rejects an existing desktop
root. Replace the example paths in the form with your actual paths. No project,
distribution, authentication or runtime installation is performed automatically.

Select Connect, then Start / resume. Readiness waits for the provider's thread
identity, not merely process creation. Messages, tool output and permission or
question cards use the shared conversation reducer and Desktop components.
Open terminal starts a supporting login shell in the project directory. It accepts
keyboard input and resizes with the pane. Close terminal ends that shell without
stopping the conversation; conversation Stop leaves the terminal running.
Disconnect preserves both; Shut down runtime ends both. Rebuild the Linux runtime
to enable the [resident attachment protocol](resident-runtime.md).
Approval remains Ask; this shell offers no persistent bypass setting.
Stop ends the agent process group. Start / resume reopens its saved native thread.
Disconnect detaches the client; connect again with the same configuration to
recover live conversation and terminal snapshots without Start. A typed draft survives those controls while the window stays open.
Drafts and connection settings are not yet persisted across window closure.

Add workspace registers an existing Linux folder and a separate conversation.
Select a workspace to recover its history and terminal while the others continue
running. The selected workspace and work stages are saved in WSL. Unsent drafts
survive switching in the open window. Shut down runtime ends all workspaces.
This supports existing folders and new-branch Git worktrees; multiple conversation tabs
are still pending. See the [workspace contract](wsl-workspaces.md).

`npm run build:app:wsl` creates the native executable without bundling, signing,
installing or publishing it. Run it on Windows for native validation; a Linux build validates Tauri
composition but does not prove the Windows webview. WSL interop can separately
exercise the `wsl.exe` transport.
For the synthetic browser composition, run `npm run dev` and open
`/wsl-preview.html`. That entry injects a deterministic mock and never starts WSL.
The native entry `wsl.html` injects only the Tauri adapter.

## Native Windows acceptance

Build the runtime in WSL and the native preview with `npm run build:app:wsl -- --debug`.
Use Windows Node to run the opt-in acceptance script against that executable.
The selected distribution needs Python 3 for the synthetic provider; it needs no
Codex login. Create a JSON configuration with your actual paths:

```json
{
  "executable": "C:\\src\\prometeu\\src-tauri\\target\\debug\\prometeu-wsl-desktop.exe",
  "distribution": "Ubuntu-24.04",
  "runtime": "/home/you/prometeu/src-tauri/target/debug/prometeu-runtime",
  "artifacts": "C:\\temp\\prometeu-native-results"
}
```

```powershell
npm run test:wsl:native -- C:\temp\prometeu-native.json
```

The script opens real native windows and connects Playwright to their WebView2
through a process-local debugging argument and a fresh temporary profile. It
does not install a browser or change registry settings. Embedded assets and native
IPC must load before it exercises messages, approval, shell input, reconnect,
window closure/reopening, stop/resume and explicit runtime shutdown. Linux paths
include spaces, quotes and Unicode. Provider launch counts and retained shell
state prove reattachment to existing execution. The report and screenshots go
to `artifacts`; failed fixture roots are retained for diagnosis after shutdown.
The executable must be built by Tauri, not just by `cargo build`, so the frontend
assets and production URL are embedded. This checks the Windows composition with
a deterministic provider; authenticated Codex behavior has its separate check below.

## Ownership and interfaces

- `src/wsl/session.ts` consumes `SessionPort`. It owns readiness, snapshot replay,
  generation/sequence checks and UI availability; it imports no Tauri code.
- `src/wsl/tauri.ts` implements that port. It subscribes before connection,
  filters delivery by a fresh connection identity and removes listeners on detach.
- `prometeu-wsl-desktop` receives `RuntimeConnector` and stores a typed
  `RuntimeClient` combining the narrow session and terminal ports. Blocking operations run off the webview thread. Its only
  concrete composition selects `StdioConnector` plus `ResidentWslLauncher`.
- `prometeu-bridge` depends on the portable core's catalog types, serde and the standard library, not Tauri or Unix
  execution crates. `RuntimeLauncher` supplies a piped process. Tests inject a
  direct Linux launcher; Windows composition invokes `wsl.exe --distribution NAME
  --exec RUNTIME --root ROOT --workdir PROJECT --codex CODEX --transport resident` with separate arguments.
  Paths remain Linux strings. No shell command interpolation or WSLg is used.

This separate executable has its own typed command registry in
`src/wsl/ipc.ts`, Rust handlers and `MockSession` counterpart. Its `wsl_connect`,
`wsl_start`, `wsl_send`, `wsl_respond`, `wsl_stop`, `wsl_snapshot` and
`wsl_disconnect` commands are intentionally absent from production `src/ipc.ts`
and `src/mock.ts`: that executable does not register them. The same isolated
registry now includes `wsl_terminal_open`, `wsl_terminal_write`,
`wsl_terminal_resize`, `wsl_terminal_snapshot`, `wsl_terminal_acknowledge` and
`wsl_terminal_close`, `wsl_terminal_current` and `wsl_shutdown`; `wsl_connect` returns whether terminal support is available. Normal desktop IPC
compatibility is unchanged. The preview CSP allows local IPC, with no remote
navigation or network capability added.

`WorkspaceClient` adds the isolated typed workspace operations described in the
[workspace contract](wsl-workspaces.md#ports-and-operations). The runtime advertises
`workspaces.v1`; older runtimes keep conversation controls but cannot supply a catalog.

## Transport and recovery

The bridge implements headless development envelope v1 unchanged. It validates
`ready` plus the Codex provider before accepting requests. Each command receives
a monotonically increasing ID; replies are correlated independently of events.
The reader runs continuously, including when no request is outstanding. Command
payloads are at most 1 MiB, incoming JSON lines at most 8 MiB, and bootstrap
stderr retains only its final 4 KiB. A larger transcript snapshot fails explicitly;
chunked replay remains future work. Startup and reply waits expire after 30 seconds.
The underlying synchronous pipe write is not an interruptible deadline.
After a failed handshake, the bridge closes the child and gives the stderr drain
up to one additional second to finish before reporting its bounded diagnostic tail.
An early stdout EOF publishes its failure before releasing the handshake waiter;
it must not race into a misleading startup timeout.

A wrong version, malformed/truncated/oversized frame, unexpected reply, EOF or
reply timeout invalidates the connection and disables sending. Diagnostic errors
are shown as text. Mutations are never retried automatically: a missing reply
has an unknown outcome. Reconnect, inspect history, and decide whether to send
again. There is no operation deduplication or transparent retry claim.

At start, events are buffered while the client obtains its generation and
snapshot. Replay replaces the timeline and applies only newer sequence numbers;
identity events still establish readiness when already covered by the snapshot.
Old connections/generations and duplicate events are ignored. A sequence gap
blocks sending until explicit stop/resume reloads history.

Normal disconnect, window exit and client drop release only the proxy attachment.
The host retains the root lease, live conversation and terminal. Explicit
`wsl_shutdown` stops execution and releases the host. The bridge still waits up
to ten seconds for its proxy to exit. A second active client is rejected; launch
configuration mismatches require shutting down the existing host before changing
settings. Reconnection restores snapshots of the same generation and terminal ID.
There are no mutation retries or process-survival guarantees after host/WSL death.
See the [resident lifecycle and limits](resident-runtime.md).

## Verification and remaining work

- `crates/bridge/src/lib.rs`: explicit WSL arguments, path validation, framing and
  reply/event separation; portable Windows target check and CI tests.
- `crates/runtime/tests/bridge.rs`: real host process, streamed events while idle,
  stop, reconnect, native thread reuse, EOF cleanup and bootstrap diagnostics.
- `src/wsl/session.test.ts`: events before replies, snapshot deduplication,
  stale generations/connections, stream gaps, failed sends and reconnect.
- `e2e/wsl-preview.spec.ts`: keyboard sending, DOM history replacement and unsent
  draft retention in this independent composition. These browser behaviors cannot
  be proven by the transport tests; existing desktop E2Es use a different shell.
- Windows CI is configured to build the native executable separately from Unix execution crates.
  This does not prove a native Windows UI launch.
- On 2026-09-28, the actual `WslLauncher` passed an explicit Ubuntu-24.04 roundtrip
  through Windows `wsl.exe` invoked from WSL interop, including spaces, quotes and
  Unicode in Linux paths. A second opt-in test used authenticated Codex, reconnected
  the host and verified same-thread recall of the first reply. These tests prove
  the WSL transport; they do not prove a Windows-built GUI or its argument encoding.

On 2026-09-30, the x86_64 MSVC executable passed the native acceptance script on
Windows 11 with WebView2 and Ubuntu-24.04. The executable was cross-compiled in
WSL with cargo-xwin 0.23.1 and LLVM 18 through the Tauri build command, then run
on Windows with embedded assets and native IPC. Messages, approval, terminal
input, disconnect/reconnect, actual window closure/reopening, stop/resume and
explicit shutdown passed. Reopening preserved one provider process and the same
shell environment. This used the synthetic provider, not authenticated model calls.
The report retained missing `favicon.ico` diagnostics; no JavaScript exceptions
or failed workflow assertions occurred.
The native GUI gate for the second delivery milestone is complete.
Resident reconnection and supporting terminal reattachment are implemented. Installation/update assets,
other providers, managed accounts/tools, board/workspace parity, file transfers,
collaboration and native integrations are still release prerequisites. Existing
native Codex configuration is inherited; a CLI that requires a shell-specific
PATH (for example Node under a shell version manager) needs that environment
available to direct WSL execution. Shell environment discovery is not implemented.

Opt-in WSL checks (run inside the distribution holding this checkout):

```sh
PROMETEU_TEST_WSL_DISTRIBUTION=Ubuntu-24.04 \
  cargo test --manifest-path src-tauri/Cargo.toml -p prometeu-runtime \
  --test bridge actual_wsl_transport_roundtrip -- --ignored
PROMETEU_TEST_WSL_DISTRIBUTION=Ubuntu-24.04 \
  PROMETEU_TEST_CODEX=/home/you/.local/bin/codex \
  cargo test --manifest-path src-tauri/Cargo.toml -p prometeu-runtime \
  --test bridge actual_wsl_codex_recalls_after_reconnect -- --ignored
```

Both tests use isolated temporary roots. The second requires native Codex login
and makes two real model requests. Neither installs or changes the distribution.
After a development page reload, use Disconnect to release a previous client
before connecting again.

Repository validation on 2026-09-29 passed 616 Rust and 575 web tests,
formatting, documentation, architecture, frontend builds and the portable Windows
MSVC target check. The preview browser scenario now includes resident reconnect.
`npm run check` passed 179 of 180 browser scenarios; the existing WebKit Git-review
scenario exceeded its 30-second timeout under parallel load and passed unchanged
in isolation in 16.9 seconds. Clippy passed separately after that interrupted
check chain. No timeout or assertion was relaxed. The preview's
native Linux build and actual WSL resident roundtrip also passed. Native Windows
GUI build/run validation subsequently passed through `scripts/test-wsl-native.mjs`
on 2026-09-30, as described above.

Supporting terminals use [the headless terminal contract](headless-runtime.md#supporting-terminal)
and [ADR 0081](../decisions/0081-wsl-supporting-terminal.md). `TerminalPort` and
`TerminalScreen` isolate presentation from IPC and xterm. Input queues, snapshot
replay and renderer acknowledgements are covered by `src/wsl/terminal.test.ts`.
The browser scenario also checks xterm keyboard routing without changing an
unsent chat draft. Actual WSL testing is available with:

```sh
PROMETEU_TEST_WSL_DISTRIBUTION=Ubuntu-24.04 \
  cargo test --manifest-path src-tauri/Cargo.toml -p prometeu-runtime \
  --test bridge actual_wsl_terminal_roundtrip -- --ignored
```

This uses the real PTY and bridge with a deterministic agent fixture. It validates
raw bytes, resize, one MiB of credited output, conversation independence, exit
status and cleanup; it does not substitute for the native Windows UI run.

Resident attachment verification on 2026-09-29 passed locally and through actual
`wsl.exe`: a turn completed while detached, the provider launched only once, the
same shell retained its environment, and malformed input/proxy death preserved
the host. A competing client and mismatched configuration were rejected. See
[the reproducible test command](resident-runtime.md#verification).
