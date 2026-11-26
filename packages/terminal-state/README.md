# Terminal state helper

This package is a private terminal emulator for the native conversation host. It is the same for all providers. It does not own a PTY, an account, a provider
process or the canonical transcript. The native transport of the desktop uses the package behind a feature gate.

```sh
npm ci --prefix packages/terminal-state
npm test --prefix packages/terminal-state
npm run typecheck --prefix packages/terminal-state
npm start --prefix packages/terminal-state
```

`src/cli.ts` starts the process. `src/stdio.ts` reads NDJSON requests that have a limit and writes only NDJSON responses.
`src/service.ts` also gives access to the ordered service without process I/O.
`src/types.ts` owns the protocol types, which are safe in a browser. `src/protocol.ts` owns the validation and the limits.

`src/replay.ts` is also safe in a browser. It makes the sequence that builds the screen again. Its
caller must do these steps:

1. Set the terminal to its initial state and change its dimensions.
2. Send the input through the parser callback, which controls it.
3. Give the UTF-8 tail to the terminal.
4. Discard the live output at or before the snapshot sequence.

The host is the only owner of the responses to terminal queries. Only a live
`write` returns `replies`, encoded as base64. Send them one time to the PTY of
that generation. The responses that the renderer makes must not also go to the
process.

The renderer and headless xterm are pinned to `6.0.0`. Serialize `0.14.0` is the
addon from that upstream release. The tests compare these items in a second
emulator after the replay:

- the installed renderer version
- the screen cells
- the cursors
- the input modes
- the hyperlinks
- the continuation

`xterm-state.ts` is the only owner of the internals that change with the
version. Upgrade these dependencies together, and run the differential tests
again.

[The terminal state contract](../../docs/contracts/terminal-state.md) gives the
protocol, the limits of the replay and the obligations of the
integration. The escape scanner is an adaptation of code from Orca. See
[the third-party notices](THIRD_PARTY_NOTICES.md).

Those commands use the Node installation for development. The SEA build bundles
Node 24.14.0 and the dependencies into one executable. It records the archive hashes that it examined and bundles the license notices:

```sh
npm run build:sea --prefix packages/terminal-state
npm run test:sea --prefix packages/terminal-state
npm run terminal-state:check
npm run app
# Local application bundle, without publishing:
npm run build:app -- --debug --bundles app --config src-tauri/tauri.dev.conf.json
```

On macOS, the standard wrapper for the app and the build does these steps:

1. It installs the locked dependencies of the helper.
2. It builds the helper.
3. It gives the explicit debug path of the helper.
4. It adds the Tauri overlay for the sidecar and its resources.

The packaged helper is adjacent to the application binary. It does not use an
installed Node. The native macOS arm64 bundle and its signature have a local
verification. The Linux x64 build procedure does not have a local verification.

CI checks the package, the self-contained executable and the host protocol. The
macOS release includes the helper and the notices, and it examines them. The
Linux release does not use the helper. The native conversation mode stays
disabled on Linux.

The macOS helper adds approximately 113 MiB before compression. It does not run
with `--jitless`. The build signs it with the entitlements of the app, which let V8 use its JIT under the hardened runtime. Tauri uses the same entitlements
when it signs the bundled sidecar. `NODE_OPTIONS` and command line flags cannot
add runtime options. The build metadata records the Node hashes and the
executable hashes in the ignored `dist/` folder.
