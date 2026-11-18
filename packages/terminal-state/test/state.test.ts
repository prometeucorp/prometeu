import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { Terminal as HeadlessTerminal } from "@xterm/headless";
import { Emulator } from "../src/emulator.ts";
import { LIMITS, type Snapshot } from "../src/protocol.ts";
import { Utf8Stream } from "../src/utf8.ts";
import { Terminal, collectLinks, readModes, viewport, viewportStyles, writeTerminal } from "../src/xterm-state.ts";
import { replayAnsi } from "../src/replay.ts";

function state(t: HeadlessTerminal) {
  return {
    cols: t.cols, rows: t.rows, active: t.buffer.active.type, modes: readModes(t), links: collectLinks(t), ...viewport(t), viewportStyles: viewportStyles(t),
    buffers: [t.buffer.normal, t.buffer.alternate].map(b => ({
      x: b.cursorX, y: b.cursorY, base: b.baseY,
      lines: Array.from({ length: b.length }, (_, row) => {
        const line = b.getLine(row)!;
        return { wrapped: line.isWrapped, cells: Array.from({ length: t.cols }, (_, col) => {
          const c = line.getCell(col)!;
          return [c.getChars(), c.getWidth(), c.getFgColorMode(), c.getFgColor(), c.getBgColorMode(), c.getBgColor(),
            c.isBold(), c.isDim(), c.isItalic(), c.isUnderline(), c.isInverse(), c.isInvisible(), c.isStrikethrough()];
        }) };
      }),
    })),
  };
}
async function restore(snapshot: Snapshot) {
  const terminal = new Terminal({ cols: snapshot.cols, rows: snapshot.rows, scrollback: snapshot.scrollback, allowProposedApi: true });
  let title = "";
  terminal.onTitleChange(value => { title = value; });
  await writeTerminal(terminal, replayAnsi(snapshot));
  const utf8 = new Utf8Stream();
  assert.equal(utf8.write(Buffer.from(snapshot.pendingUtf8, "base64")), "");
  return { terminal, utf8, title: () => title };
}
async function compare(text: string, continuation = "", size = { cols: 12, rows: 4 }) {
  const source = new Emulator("session", "generation", size.cols, size.rows, 100);
  const bytes = Buffer.from(text);
  await source.write(bytes, bytes.length);
  const target = await restore(source.snapshot());
  try {
    assert.deepEqual(state(target.terminal), state(source.terminal));
    assert.equal(target.title(), source.snapshot().title);
    if (continuation) {
      const next = Buffer.from(continuation);
      await source.write(next, source.seq + next.length);
      await writeTerminal(target.terminal, target.utf8.write(next));
      assert.deepEqual(state(target.terminal), state(source.terminal));
      assert.equal(target.title(), source.snapshot().title);
    }
  } finally { source.dispose(); target.terminal.dispose(); }
}

test("headless and serializer versions match the installed renderer", () => {
  const json = (path: string) => JSON.parse(readFileSync(new URL(path, import.meta.url), "utf8"));
  assert.equal(json("../node_modules/@xterm/headless/package.json").version, json("../../../node_modules/@xterm/xterm/package.json").version);
  assert.equal(json("../node_modules/@xterm/addon-serialize/package.json").version, "0.13.0");
});

test("normal screen, scrollback, ANSI attributes, wide text and live continuation round trip", async () => {
  await compare("line one\r\nline two\r\nline three\r\nline four\r\n\x1b[31;44;1mred 界\x1b[0m", "!\r\nnext");
  await compare("\x1b[4;3;7;9;2mstyled", " more\x1b[0m plain");
});

test("alternate screen preserves normal buffer, cursor and input modes", async () => {
  await compare("normal\x1b[?1049h\x1b[?2004h\x1b[?1h\x1b[?1003h\x1b[?1006h\x1b[?25l\x1b[5 qTUI\x1b[3;5Hrow", "\x1b[?1049l next");
});

test("saved cursor, scroll margins and origin mode survive a continued frame", async () => {
  await compare("header\x1b[2;3r\x1b[?6h\x1b[2;3Hsaved\x1b7\x1b[1;1Htop", "\x1b8!");
  await compare("\x1b[31mred\x1b7\x1b[32mgreen", "\x1b8 saved");
});

test("wrap-pending cursor preserves the next printable byte, including a wide final cell", async () => {
  await compare("abcdefghijkl", "M");
  await compare("abcdefghij界", "M");
});

test("OSC8 links retain targets in normal scrollback, alt screen and an open link pen", async () => {
  await compare("\x1b]8;id=docs;https://example.test/docs\x1b\\linked\x1b[31m text\x1b]8;;\x1b\\\r\nx\r\nx\r\nx\r\nx", "more");
  await compare("normal\x1b[?1049h\x1b]8;;https://example.test/tui\x1b\\界link", "next\x1b]8;;\x1b\\ plain");
});

test("split CSI and OSC restore the pending parser suffix once", async () => {
  await compare("before\x1b[", "31mred");
  await compare("before\x1b]8;id=split;https://example", ".test\x1b\\link\x1b]8;;\x1b\\");
  await compare("before\x1b]2;partial", " title\x1b\\after");
  await compare("before\x1b[\n", "31mred");
});

test("split UTF-8 survives snapshots at every byte boundary", async () => {
  for (const char of ["é", "界", "😀"]) {
    const bytes = Buffer.from(char);
    for (let split = 1; split < bytes.length; split++) {
      const source = new Emulator("session", "generation", 12, 4, 100);
      await source.write(bytes.subarray(0, split), split);
      const snapshot = source.snapshot();
      const target = await restore(snapshot);
      try {
        assert.equal(snapshot.pendingUtf8, bytes.subarray(0, split).toString("base64"));
        await source.write(bytes.subarray(split), bytes.length);
        await writeTerminal(target.terminal, target.utf8.write(bytes.subarray(split)));
        assert.deepEqual(state(target.terminal), state(source.terminal));
      } finally { source.dispose(); target.terminal.dispose(); }
    }
  }
});

test("malformed UTF-8 matches streaming replacement behavior at every boundary", () => {
  const bytes = Buffer.from([0xc3, 0xa9, 0xff, 0xf0, 0x9f, 0x92, 0xa9, 0xe0, 0x80, 0x80, 0xed, 0xa0, 0x80, 0xe2, 0x82, 0x21]);
  const expected = new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes);
  for (let split = 1; split < bytes.length; split++) {
    const decoder = new Utf8Stream();
    assert.equal(decoder.write(bytes.subarray(0, split)) + decoder.write(bytes.subarray(split)), expected);
    assert.equal(decoder.tail(), "");
  }
});

test("resize is an ordered snapshot boundary and reflows like the live renderer", async () => {
  const source = new Emulator("session", "generation", 12, 4, 100);
  const data = Buffer.from("first line\r\nsecond line\r\nthird line\x1b]8;;https://example.test\x1b\\link\x1b]8;;\x1b\\");
  await source.write(data, data.length);
  source.resize(9, 5, source.seq);
  const target = await restore(source.snapshot());
  try {
    assert.deepEqual(state(target.terminal), state(source.terminal));
    source.resize(16, 3, source.seq);
    target.terminal.resize(16, 3);
    assert.deepEqual(state(target.terminal), state(source.terminal));
  } finally { source.dispose(); target.terminal.dispose(); }
});

test("queries are captured only for live writes; snapshot replay cannot feed the process", async () => {
  const source = new Emulator("session", "generation", 12, 4, 100);
  const data = Buffer.from("hello\x1b[6n\x1b[");
  const first = await source.write(data, data.length);
  assert.deepEqual(first.replies.map(v => Buffer.from(v, "base64").toString()), ["\x1b[1;6R"]);
  const target = await restore(source.snapshot());
  try {
    const replies: string[] = [];
    target.terminal.onData(data => replies.push(data));
    const live = await source.write(Buffer.from("6n"), source.seq + 2);
    await writeTerminal(target.terminal, "6n");
    assert.deepEqual(live.replies.map(v => Buffer.from(v, "base64").toString()), replies);
    assert.deepEqual(replies, ["\x1b[1;6R"]);
  } finally { source.dispose(); target.terminal.dispose(); }
});

test("title is bounded metadata and oversized incomplete escapes explicitly refuse snapshots", async () => {
  const source = new Emulator("session", "generation", 12, 4, 100);
  const data = Buffer.from("\x1b]2;title\x1b\\\x1b]8;;" + "x".repeat(LIMITS.escapeChars));
  await source.write(data, data.length);
  assert.throws(() => source.snapshot(), { message: "snapshot_unavailable" });
  await source.write(Buffer.from("\x1b\\"), source.seq + 2);
  assert.equal(source.snapshot().title, "title");
  assert.equal(source.snapshot().pendingEscape, "");
  source.dispose();
});

test("snapshot link expansion is bounded before serializing repeated long URLs", async () => {
  const source = new Emulator("session", "generation", 100, 10, 100);
  const data = Buffer.from("\x1b]8;;https://example.test/" + "x".repeat(20_000) + "\x1b\\" + "a".repeat(1_000));
  await source.write(data, data.length);
  assert.throws(() => source.snapshot(), { message: "snapshot_unavailable" });
  source.dispose();
});

test("active viewport is exposed without scrollback or ANSI controls", async () => {
  const source = new Emulator("session", "generation", 12, 3, 100);
  const input = Buffer.from("old\r\none\r\ntwo\r\n\x1b[31mthree  ");
  await source.write(input, input.length);
  assert.deepEqual(source.snapshot().viewportLines, ["one", "two", "three  "]);
  const next = Buffer.from("\x1b[?1049h\x1b[Hnative\x1b[3;1Hprompt");
  await source.write(next, source.seq + next.length);
  assert.deepEqual(source.snapshot().viewportLines, ["native", "", "prompt"]);
  source.dispose();
});

test("Unicode scalar columns follow painted cells through combining marks, emoji, wide text and resize", async () => {
  const source = new Emulator("session", "generation", 20, 3, 100);
  const bytes = Buffer.from("e\u0301界🐣x  ");
  await source.write(bytes, bytes.length);
  const first = source.snapshot();
  assert.equal(first.viewportLines[0], "e\u0301界🐣x  ");
  // This pinned renderer uses Unicode 6: the chick is one cell, while the CJK scalar uses two.
  assert.deepEqual(first.viewportCharacterColumns, [[0, 0, 1, 3, 4, 5, 6], [], []]);
  source.resize(5, 4, source.seq);
  const snapshot = source.snapshot();
  const target = await restore(snapshot);
  try {
    assert.deepEqual(viewport(target.terminal), viewport(source.terminal));
    for (let row = 0; row < snapshot.rows; row++) {
      const scalars = Array.from(snapshot.viewportLines[row]);
      assert.equal(snapshot.viewportCharacterColumns[row].length, scalars.length);
      for (const column of snapshot.viewportCharacterColumns[row]) assert.ok(column >= 0 && column < snapshot.cols);
    }
  } finally { source.dispose(); target.terminal.dispose(); }
});

test("combining scalar metadata has an explicit bound independent of terminal cell count", async () => {
  const source = new Emulator("session", "generation", 12, 3, 100);
  const text = "e" + "\u0301".repeat(LIMITS.viewportCharacters);
  const bytes = Buffer.from(text);
  await source.write(bytes, bytes.length);
  assert.throws(() => source.snapshot(), { message: "snapshot_unavailable" });
  source.dispose();
});

test("native color queries use shared appearance, preserve changes and restore defaults", async () => {
  const source = new Emulator("session", "generation", 12, 3, 100);
  const write = async (text: string) => {
    const bytes = Buffer.from(text);
    return (await source.write(bytes, source.seq + bytes.length)).replies.map(value => Buffer.from(value, "base64").toString());
  };
  assert.deepEqual(await write("\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]12;?\x1b\\"), [
    "\x1b]10;rgb:ffff/ffff/ffff\x1b\\", "\x1b]11;rgb:2828/2c2c/3434\x1b\\", "\x1b]12;rgb:ffff/ffff/ffff\x1b\\",
  ]);
  assert.deepEqual(await write("\x1b]11;#aabbcc\x1b\\\x1b]11;?\x1b\\"), ["\x1b]11;rgb:aaaa/bbbb/cccc\x1b\\"]);
  assert.equal(source.snapshot().appearance.background, "#aabbcc");
  assert.ok(replayAnsi(source.snapshot()).startsWith("\x1b]10;#ffffff\x1b\\\x1b]11;#aabbcc\x1b\\"));
  await write("\x1b]111\x1b\\");
  assert.equal(source.snapshot().appearance.background, "#282c34");
  assert.deepEqual(await write("\x1b]10;"), []);
  assert.deepEqual(await write("?\x07"), ["\x1b]10;rgb:ffff/ffff/ffff\x1b\\"]);
  source.dispose();
});

test("SGR pixel mouse encoding survives replay", async () => {
  await compare("\x1b[?1003h\x1b[?1016htext");
});

test("viewport style runs distinguish a dim placeholder from identical literal input", async () => {
  const source = new Emulator("session", "generation", 40, 3, 100);
  const write = async (text: string) => { const bytes = Buffer.from(text); await source.write(bytes, source.seq + bytes.length); };
  await write("→ \x1b[7mA\x1b[27;2mdd a follow-up\x1b[0m");
  const placeholder = source.snapshot();
  await write("\r\x1b[2K→ \x1b[7mA\x1b[27mdd a follow-up");
  const literal = source.snapshot();
  assert.equal(placeholder.viewportLines[0], literal.viewportLines[0]);
  const attribute = (snapshot: Snapshot, col: number) => snapshot.viewportStyles[0].find(run => run.start <= col && col < run.end)!;
  assert.equal(attribute(placeholder, 2).inverse, true);
  assert.equal(attribute(literal, 2).inverse, true);
  assert.equal(attribute(placeholder, 3).dim, true);
  assert.equal(attribute(literal, 3).dim, false);
  await write("\r\x1b[2K\x1b[38;5;8;48;2;1;2;3mcolored\x1b[0m界");
  const styled = source.snapshot();
  assert.deepEqual(attribute(styled, 0).foreground, { mode: "palette", value: 8 });
  assert.deepEqual(attribute(styled, 0).background, { mode: "rgb", value: 0x010203 });
  for (const row of styled.viewportStyles) {
    assert.equal(row[0].start, 0);
    assert.equal(row.at(-1)!.end, styled.cols);
    for (let i = 1; i < row.length; i++) assert.equal(row[i].start, row[i - 1].end);
  }
  source.dispose();
});
