/** Retain only a valid incomplete UTF-8 code point, at most three bytes. */
export class Utf8Stream {
  private pending = Buffer.alloc(0);
  private decoder = new TextDecoder("utf-8", { ignoreBOM: true });

  write(bytes: Uint8Array): string {
    const data = Buffer.concat([this.pending, bytes]);
    let end = data.length;
    for (let i = Math.max(0, end - 3); i < end; i++) {
      const lead = data[i];
      const size = lead >= 0xc2 && lead <= 0xdf ? 2 : lead >= 0xe0 && lead <= 0xef ? 3
        : lead >= 0xf0 && lead <= 0xf4 ? 4 : 0;
      if (!size || end - i >= size) continue;
      const suffix = data.subarray(i + 1);
      if ([...suffix].every(byte => byte >= 0x80 && byte <= 0xbf)
        && !(suffix.length && ((lead === 0xe0 && suffix[0] < 0xa0)
          || (lead === 0xed && suffix[0] > 0x9f) || (lead === 0xf0 && suffix[0] < 0x90)
          || (lead === 0xf4 && suffix[0] > 0x8f)))) { end = i; break; }
    }
    this.pending = Buffer.from(data.subarray(end));
    return this.decoder.decode(data.subarray(0, end));
  }
  tail(): string { return this.pending.toString("base64"); }
}
