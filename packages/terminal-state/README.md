# Terminal state helper

Private, provider-neutral terminal emulation for the native conversation host.
It owns no PTY, account, provider process or canonical transcript. The package
is consumed by the guarded desktop native transport. WSL does not consume it.

```sh
npm ci --prefix packages/terminal-state
npm test --prefix packages/terminal-state
npm run typecheck --prefix packages/terminal-state
npm start --prefix packages/terminal-state
```

`src/cli.ts` starts the process. `src/stdio.ts` reads bounded NDJSON requests and writes only NDJSON responses.
`src/service.ts` also exposes the ordered service without process I/O.
`src/types.ts` owns browser-safe wire types; `src/protocol.ts` owns validation and limits. The browser-safe
`src/replay.ts` produces the restore sequence; its caller must reset/resize,
gate input through the parser callback, seed the UTF-8 tail, and discard live
output at or before the snapshot sequence.

The host is the single owner of terminal query responses. Only a live `write`
returns `replies`, encoded as base64. Forward them once to that generation's
PTY. Renderer-generated responses must never also reach the process.

The renderer and headless xterm are pinned to `5.5.0`; serialize `0.13.0` is the
addon from that upstream release. Tests compare the installed renderer version,
screen cells, cursors, input modes, hyperlinks and continuation after replay in
a second emulator. `xterm-state.ts` is the only owner of version-specific
internals. Upgrade these dependencies together and rerun the differential tests.

The protocol, restoration limits and integration obligations are documented in
[the terminal state contract](../../docs/contracts/terminal-state.md).
The escape scanner is adapted from Orca; see
[third-party notices](THIRD_PARTY_NOTICES.md).

Those commands use the development Node installation. The SEA build
bundles Node 24.14.0 and dependencies into one executable, with verified archive
hashes and bundled license notices:

```sh
npm run build:sea --prefix packages/terminal-state
npm run test:sea --prefix packages/terminal-state
npm run terminal-state:check
npm run app
# Local application bundle, without publishing:
npm run build:app -- --debug --bundles app --config src-tauri/tauri.dev.conf.json
```

On macOS the standard app/build wrapper installs the helper's locked dependencies,
builds it, provides its explicit debug path and adds the
Tauri sidecar/resource overlay. The packaged helper lives beside the application
binary and needs no installed Node. Native macOS arm64 bundling and signing are
locally verified; the Linux x64 build recipe is not locally verified. WSL keeps
rejecting native conversation mode. CI checks the package, self-contained executable
and host protocol; the macOS release includes and verifies the helper and notices.
Linux release and WSL do not consume it. Native conversation mode remains disabled.

The macOS helper currently adds about 113 MiB before compression. It runs with
`--jitless`, so the existing app signing entitlements also work for the nested
executable. Runtime options cannot be injected through `NODE_OPTIONS` or command
line flags. Build metadata records Node and executable hashes in ignored `dist/`.
