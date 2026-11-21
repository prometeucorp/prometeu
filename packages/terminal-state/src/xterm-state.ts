import headless from "@xterm/headless";
import serialize from "@xterm/addon-serialize";
import type { IBuffer, IBufferCell, Terminal as HeadlessTerminal } from "@xterm/headless";
import type { Terminal as BrowserTerminal } from "@xterm/xterm";
import { LIMITS, requireThat, type Cursor, type Modes, type OscLink, type Screen } from "./protocol.ts";
import { NATIVE_TERMINAL_APPEARANCE, type Appearance } from "./appearance.ts";
import type { Snapshot, TerminalColor, ViewportStyleRun } from "./types.ts";

export const { Terminal } = headless;
const { SerializeAddon } = serialize;
const ESC = "\x1b";

/** All version-specific xterm internals live here. The package pins and tests the 6.0.0 builds. */
type Attr = IBufferCell & { fg: number; extended?: { urlId?: number }; hasExtendedAttrs(): number };
type CoreBuffer = {
  x: number; y: number; ybase: number; scrollTop: number; scrollBottom: number;
  savedX: number; savedY: number; savedCurAttrData: Attr;
};
type Core = {
  _oscLinkService: { getLinkData(id: number): { uri: string; id?: string } | undefined };
  _inputHandler: { _curAttrData: Attr; onColor(listener: (requests: ColorRequest[]) => void): { dispose(): void } };
  coreService: { isCursorHidden: boolean };
  coreMouseService: { activeEncoding: "DEFAULT" | "SGR" | "SGR_PIXELS" };
  _bufferService: { buffers: { normal: CoreBuffer; alt: CoreBuffer } };
};
function core(terminal: HeadlessTerminal): Core {
  return (terminal as unknown as { _core: Core })._core;
}
type ColorRequest = { type: 0 | 1 | 2; index?: number; color?: [number, number, number] };
/** The headless build emits parsed color requests but has no browser ThemeService to answer them. */
export function terminalColors(terminal: HeadlessTerminal, reply: (data: string) => void) {
  const current: Appearance = { ...NATIVE_TERMINAL_APPEARANCE };
  const subscription = core(terminal)._inputHandler.onColor(requests => {
    for (const request of requests) {
      const name = ({ 256: "foreground", 257: "background", 258: "cursor" } as const)[request.index as 256];
      if (!name) continue;
      if (request.type === 0) {
        const rgb = current[name].slice(1).match(/../g)!.map(channel => channel.repeat(2)).join("/");
        reply(`${ESC}]${request.index! - 246};rgb:${rgb}${ESC}\\`);
      } else if (request.type === 1 && request.color) {
        current[name] = "#" + request.color.map(channel => channel.toString(16).padStart(2, "0")).join("");
      } else if (request.type === 2) current[name] = NATIVE_TERMINAL_APPEARANCE[name];
    }
  });
  return { snapshot: (): Appearance => ({ ...current }), dispose: () => subscription.dispose() };
}
function oscLink(uri: string, id?: string): string {
  // Parsed OSC values cannot contain a terminator. IDs use the same colon-separated OSC grammar.
  return `${ESC}]8;${id ? `id=${id}` : ""};${uri}${ESC}\\`;
}
function linkOf(terminal: HeadlessTerminal, cell: IBufferCell) {
  const id = (cell as Attr).hasExtendedAttrs() ? (cell as Attr).extended?.urlId : undefined;
  return id ? core(terminal)._oscLinkService.getLinkData(id) : undefined;
}
function attributes(attr: IBufferCell): string {
  const codes = [0];
  if (attr.isBold()) codes.push(1);
  if (attr.isDim()) codes.push(2);
  if (attr.isItalic()) codes.push(3);
  // isUnderline() includes OSC8's implicit underline; turning it into SGR leaks past link closure.
  if ((attr as Attr).fg & 0x10000000) codes.push(4);
  if (attr.isBlink()) codes.push(5);
  if (attr.isInverse()) codes.push(7);
  if (attr.isInvisible()) codes.push(8);
  if (attr.isStrikethrough()) codes.push(9);
  if (attr.isOverline()) codes.push(53);
  for (const foreground of [true, false]) {
    const value = foreground ? attr.getFgColor() : attr.getBgColor();
    const rgb = foreground ? attr.isFgRGB() : attr.isBgRGB();
    const palette = foreground ? attr.isFgPalette() : attr.isBgPalette();
    if (rgb) codes.push(foreground ? 38 : 48, 2, value >>> 16 & 255, value >>> 8 & 255, value & 255);
    else if (palette) {
      const mode = foreground ? attr.getFgColorMode() : attr.getBgColorMode();
      if (mode === 0x1000000) codes.push((foreground ? 30 : 40) + (value < 8 ? value : value + 52));
      else codes.push(foreground ? 38 : 48, 5, value);
    }
  }
  return `${ESC}[${codes.join(";")}m`;
}

/** The upstream serializer reads getChars() for text and truthiness, never to compute cell width.
 * Add OSC8 around each nonempty linked cell while leaving its measured width and attributes intact.
 * Stable proxy identities preserve the serializer's two-cell scratch-buffer alternation.
 */
function linkedTerminal(terminal: HeadlessTerminal): BrowserTerminal {
  const cells = new WeakMap<IBufferCell, IBufferCell>();
  const originals = new WeakMap<IBufferCell, IBufferCell>();
  const wrapCell = (cell: IBufferCell): IBufferCell => {
    let wrapped = cells.get(cell);
    if (!wrapped) {
      wrapped = new Proxy(cell, { get(target, key) {
        if (key === "getChars") return () => {
          const chars = target.getChars();
          const link = chars && linkOf(terminal, target);
          return link ? `${oscLink(link.uri, link.id)}${chars}${oscLink("")}` : chars;
        };
        const value = Reflect.get(target, key, target);
        return typeof value === "function" ? value.bind(target) : value;
      } });
      cells.set(cell, wrapped);
      originals.set(wrapped, cell);
    }
    return wrapped;
  };
  const wrapBuffer = (buffer: IBuffer): IBuffer => new Proxy(buffer, { get(target, key) {
    if (key === "getNullCell") return () => wrapCell(target.getNullCell());
    if (key === "getLine") return (row: number) => {
      const line = target.getLine(row);
      return line && new Proxy(line, { get(targetLine, property) {
        if (property === "getCell") return (col: number, scratch?: IBufferCell) => {
          const cell = targetLine.getCell(col, scratch && (originals.get(scratch) ?? scratch));
          return cell && wrapCell(cell);
        };
        const value = Reflect.get(targetLine, property, targetLine);
        return typeof value === "function" ? value.bind(targetLine) : value;
      } });
    };
    return Reflect.get(target, key, target);
  } });
  const buffers = { normal: wrapBuffer(terminal.buffer.normal), alternate: wrapBuffer(terminal.buffer.alternate) };
  return new Proxy(terminal, { get(target, key) {
    if (key === "buffer") return { ...buffers, active: target.buffer.active };
    return Reflect.get(target, key, target);
  } }) as unknown as BrowserTerminal;
}

function cursor(buffer: IBuffer, cols: number): Cursor {
  return { x: buffer.cursorX, y: buffer.cursorY, wrapPending: buffer.cursorX >= cols };
}
function screen(terminal: HeadlessTerminal, name: "normal" | "alternate", ansi: string): Screen {
  const b = core(terminal)._bufferService.buffers[name === "normal" ? "normal" : "alt"];
  const api = terminal.buffer[name];
  let wrapCell: Screen["wrapCell"] = null;
  if (api.cursorX >= terminal.cols) {
    const line = api.getLine(api.baseY + api.cursorY)!;
    let col = terminal.cols - 1;
    while (col > 0 && line.getCell(col)?.getWidth() === 0) col--;
    const cell = line.getCell(col)!;
    const link = linkOf(terminal, cell);
    wrapCell = { col, ansi: attributes(cell) + (link ? oscLink(link.uri, link.id) : oscLink("")) + (cell.getChars() || " ") };
  }
  return {
    ansi,
    cursor: cursor(terminal.buffer[name], terminal.cols),
    scrollTop: b.scrollTop,
    scrollBottom: b.scrollBottom,
    savedCursor: { x: Math.min(b.savedX, terminal.cols - 1),
      y: Math.max(0, Math.min(b.savedY - b.ybase, terminal.rows - 1)), origin: false,
      attributes: attributes(b.savedCurAttrData) },
    wrapCell,
  };
}
export function readModes(terminal: HeadlessTerminal): Modes {
  return { ...terminal.modes, mouseEncoding: core(terminal).coreMouseService.activeEncoding,
    cursorHidden: core(terminal).coreService.isCursorHidden,
    cursorStyle: terminal.options.cursorStyle ?? "block", cursorBlink: terminal.options.cursorBlink ?? false };
}
export function viewport(terminal: HeadlessTerminal): Pick<Snapshot, "viewportLines" | "viewportCharacterColumns"> {
  const buffer = terminal.buffer.active;
  const viewportLines: string[] = [];
  const viewportCharacterColumns: number[][] = [];
  let total = 0;
  for (let row = 0; row < terminal.rows; row++) {
    const line = buffer.getLine(buffer.baseY + row);
    const text = line?.translateToString(true) ?? "";
    let count = 0;
    for (const _ of text) { count++; requireThat(++total <= LIMITS.viewportCharacters, "snapshot_unavailable"); }
    const columns: number[] = [];
    const scratch = buffer.getNullCell();
    // Follow xterm's BufferLine translation widths, including a standalone width-zero cell.
    // A combined cell can hold several scalars; all belong to the same painted column.
    for (let col = 0; col < terminal.cols && columns.length < count;) {
      const cell = line!.getCell(col, scratch)!;
      for (const _ of cell.getChars() || " ") columns.push(col);
      col += cell.getWidth() || 1;
    }
    requireThat(columns.length === count, "snapshot_unavailable");
    viewportLines.push(text);
    viewportCharacterColumns.push(columns);
  }
  return { viewportLines, viewportCharacterColumns };
}

export function viewportStyles(terminal: HeadlessTerminal): ViewportStyleRun[][] {
  const buffer = terminal.buffer.active;
  const color = (cell: IBufferCell, foreground: boolean): TerminalColor => {
    const rgb = foreground ? cell.isFgRGB() : cell.isBgRGB();
    const palette = foreground ? cell.isFgPalette() : cell.isBgPalette();
    return { mode: rgb ? "rgb" : palette ? "palette" : "default",
      value: foreground ? cell.getFgColor() : cell.getBgColor() };
  };
  return Array.from({ length: terminal.rows }, (_, row) => {
    const line = buffer.getLine(buffer.baseY + row);
    const runs: ViewportStyleRun[] = [];
    const scratch = buffer.getNullCell();
    for (let col = 0; col < terminal.cols; col++) {
      const cell = line?.getCell(col, scratch) ?? buffer.getNullCell();
      const next = { start: col, end: col + 1, dim: !!cell.isDim(), inverse: !!cell.isInverse(),
        foreground: color(cell, true), background: color(cell, false) };
      const last = runs[runs.length - 1];
      if (last && last.dim === next.dim && last.inverse === next.inverse
        && last.foreground.mode === next.foreground.mode && last.foreground.value === next.foreground.value
        && last.background.mode === next.background.mode && last.background.value === next.background.value) last.end++;
      else runs.push(next);
    }
    return runs;
  });
}
function modesAnsi(m: Modes): string {
  const set = (code: number, enabled: boolean, privateMode = true) => `${ESC}[${privateMode ? "?" : ""}${code}${enabled ? "h" : "l"}`;
  return set(1, m.applicationCursorKeysMode) + set(66, m.applicationKeypadMode)
    + set(2004, m.bracketedPasteMode) + set(4, m.insertMode, false)
    + set(45, m.reverseWraparoundMode) + set(1004, m.sendFocusMode)
    + set(7, m.wraparoundMode) + set(25, !m.cursorHidden)
    + ([9, 1000, 1002, 1003].map(code => set(code, false)).join(""))
    + (m.mouseTrackingMode === "none" ? "" : set({ x10: 9, vt200: 1000, drag: 1002, any: 1003 }[m.mouseTrackingMode], true))
    + set(1006, false) + set(1016, false)
    + (m.mouseEncoding === "DEFAULT" ? "" : set(m.mouseEncoding === "SGR" ? 1006 : 1016, true))
    + `${ESC}[${({ block: 2, underline: 4, bar: 6 }[m.cursorStyle]) - (m.cursorBlink ? 1 : 0)} q`;
}
export function collectLinks(terminal: HeadlessTerminal): OscLink[] {
  const links: OscLink[] = [];
  for (const name of ["normal", "alternate"] as const) {
    const buffer = terminal.buffer[name];
    for (let row = 0; row < buffer.length; row++) {
      const line = buffer.getLine(row)!;
      for (let col = 0; col < Math.min(line.length, terminal.cols); col++) {
        const cell = line.getCell(col)!;
        const link = cell.getWidth() > 0 && linkOf(terminal, cell);
        if (!link) continue;
        const prev = links.at(-1);
        if (prev && prev.buffer === name && prev.row === row && prev.end === col
          && prev.uri === link.uri && prev.id === link.id) prev.end += cell.getWidth();
        else links.push({ buffer: name, row, start: col, end: col + cell.getWidth(), ...link });
      }
    }
  }
  return links;
}

export function serializeTerminal(terminal: HeadlessTerminal, view = false) {
  const modes = readModes(terminal);
  const activeLink = linkOf(terminal, core(terminal)._inputHandler._curAttrData);
  const rehydrate = modesAnsi(modes) + attributes(core(terminal)._inputHandler._curAttrData)
    + (activeLink ? oscLink(activeLink.uri, activeLink.id) : oscLink(""));
  const active = terminal.buffer.active.type;
  if (view) return {
    active, modes, rehydrate, links: [],
    normal: screen(terminal, "normal", ""),
    alternate: active === "alternate" ? screen(terminal, "alternate", "") : null,
  };
  // OSC8 is emitted per linked cell. Refuse an oversized expansion before allocating it.
  let linkExpansionBytes = 0;
  const links = collectLinks(terminal);
  for (const link of links) {
    linkExpansionBytes += (link.end - link.start) * (Buffer.byteLength(link.uri) + Buffer.byteLength(link.id ?? "") + 20);
    requireThat(linkExpansionBytes <= LIMITS.snapshotBytes, "snapshot_unavailable");
  }
  const addon = new SerializeAddon();
  addon.activate(linkedTerminal(terminal));
  const normal = addon.serialize({ excludeAltBuffer: true, excludeModes: true });
  const full = active === "alternate" ? addon.serialize({ excludeModes: true }) : normal;
  const separator = `${ESC}[?1049h${ESC}[H`;
  if (active === "alternate" && !full.slice(normal.length).startsWith(separator))
    throw new Error("Unsupported xterm serializer shape");
  return {
    active,
    normal: screen(terminal, "normal", normal),
    alternate: active === "alternate" ? screen(terminal, "alternate", full.slice(normal.length + separator.length)) : null,
    modes,
    rehydrate,
    links,
  };
}

export function writeTerminal(terminal: HeadlessTerminal, text: string): Promise<void> {
  return new Promise(resolve => terminal.write(text, resolve));
}
