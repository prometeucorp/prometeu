import { describe, expect, it } from "vitest";
import { firstLine } from "./content";

describe("firstLine", () => {
  it("keeps a single line as is", () => {
    expect(firstLine("Audit repository quality")).toBe("Audit repository quality");
  });

  it("marks a multi-line script as cut after its first non-empty line", () => {
    expect(firstLine("\ncd /repo; python3 - <<'EOF'\nprint(1)\nEOF")).toBe("cd /repo; python3 - <<'EOF' …");
  });

  it("ignores trailing blank lines and empty text", () => {
    expect(firstLine("ls -la\n\n")).toBe("ls -la");
    expect(firstLine("")).toBe("");
  });
});
