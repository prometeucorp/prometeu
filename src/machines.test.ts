import { describe, expect, it } from "vitest";
import { aliases } from "./machines";

describe("aliases", () => {
  it("shows a machine's clone of a local repository as the local project", () => {
    const map = aliases([
      { id: "/Users/me/site", name: "site", path: "/Users/me/site", origin: "github.com/me/site" },
      { id: "remote:vps:/home/me/site", name: "site", path: "/home/me/site", host: "vps", origin: "github.com/me/site" },
      { id: "remote:vps:/home/me/only", name: "only", path: "/home/me/only", host: "vps", origin: "github.com/me/only" },
      { id: "remote:vps:/home/me/bare", name: "bare", path: "/home/me/bare", host: "vps" },
    ]);
    expect([...map]).toEqual([["remote:vps:/home/me/site", "/Users/me/site"]]);
  });
});
