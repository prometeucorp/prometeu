# Third-party notices

`src/vendor/terminal-escape-introducer.ts` and
`src/vendor/terminal-partial-escape-tail.ts` are adapted from
[stablyai/orca](https://github.com/stablyai/orca), commit
`6ee09ca1c9901c58e45df966cadd6f2a86e268f0`, paths
`src/shared/terminal/protocol/escape/terminal-escape-introducer.ts` and
`src/shared/terminal/protocol/escape/terminal-partial-escape-tail.ts`.
The unused continuation helpers were removed. Prometeu's wrapper refuses an
oversized pending escape snapshot instead of silently discarding its tail.

The snapshot contract is informed by Orca's headless emulator and terminal
snapshot protocol at the same commit. The state serializer adapter and replay
implementation in this package are Prometeu code.

## Orca license

MIT License

Copyright (c) 2026 Lovecast Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## xterm dependencies

`@xterm/headless`, `@xterm/xterm` and `@xterm/addon-serialize` are MIT-licensed
packages from [xterm.js](https://github.com/xtermjs/xterm.js/tree/5.5.0).
The renderer archive carries the upstream project license; the headless and
serialize npm archives omit that file. The helper build copies the upstream
license as `xterm-LICENSE`, together with Node's distribution license and this
notice. The opt-in Tauri overlay includes that license directory.
