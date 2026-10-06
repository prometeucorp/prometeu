import { describe, expect, it } from "vitest";
import { md } from "./markdown";

describe("untrusted Markdown", () => {
  it("renders raw HTML as text", () => {
    const html = md(`<img src=x onerror="globalThis.pwned=true">`);
    expect(html).toContain("&lt;img");
    expect(html).not.toContain("<img");
  });

  it("removes executable link protocols", () => {
    expect(md("[click](javascript:alert(1))")).toContain('href="#"');
    expect(md("[site](https://example.com/a)")).toContain('href="https://example.com/a"');
  });
});

describe("code blocks", () => {
  it("adds copy controls only to blocks and escapes their source", () => {
    const html = md('```html\n<img title="x"> & test\n```\n\n`inline`');
    expect(html.match(/class="ui-button ghost sm md-code-copy"/g)).toHaveLength(1);
    expect(html).toContain('data-code="&lt;img title=&quot;x&quot;&gt; &amp; test"');
    expect(html).not.toContain('<img');
    expect(md('```diff\n-old\n+new\n```')).toContain('class="code tdiff"');
  });
});

describe("GitHub HTML", () => {
  it("keeps the safe subset and drops the rest", () => {
    const html = md(`<!-- bot -->\n<h2><a href="javascript:alert(1)" onclick="x()"><img alt="Retrigger" src="https://x/y.svg"></a>Score</h2>\n\n<details open><summary>More</summary>\n\n**bold** <script>alert(1)</script> &nbsp;<img src=x onerror="globalThis.pwned=true">\n\n</details>`, { html: true });
    expect(html).toContain('<h2><a class="lnk" href="#" rel="noreferrer noopener"><span class="img">Retrigger</span></a>Score</h2>');
    expect(html).toContain("<details open><summary>More</summary>");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("&nbsp;");
    expect(html).not.toMatch(/<script|<img|onclick|onerror|bot -->/);
  });

  it("stays off for agent Markdown", () => {
    expect(md("<h2>x</h2>")).toContain("&lt;h2&gt;");
    md("<h2>x</h2>", { html: true });
    expect(md("<b>x</b>")).toContain("&lt;b&gt;");
  });
});
