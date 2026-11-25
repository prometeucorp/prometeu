import { afterEach, describe, expect, it, vi } from "vitest";
import { icon } from "./icons";
import { aliases, badge, creatorBadge, projectsOn } from "./machines";

vi.mock("./ipc", () => ({ invoke: vi.fn() }));

it("offers only registered projects on the selected machine, including clones with a local alias", () => {
  const local = { id: "local", name: "site", path: "/Users/me/site", origin: "github.com/me/site" };
  const vps = { ...local, id: "remote:vps:/home/me/site", path: "/home/me/site", host: "vps" };
  const other = { ...vps, id: "remote:other:/home/me/site", host: "other" };
  const projects = [local, vps, other];
  expect(projectsOn(projects)).toEqual([local]);
  expect(projectsOn(projects, "vps")).toEqual([vps]);
  expect(projectsOn(projects, "empty")).toEqual([]);
});

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

/// Just enough DOM for a chip: its class, markup, text, tooltip and accessible name.
type Chip = { className: string; innerHTML: string; title: string; label: string; text: string };
function chip(element: HTMLElement | null): Chip | null {
  return element as unknown as Chip | null;
}
const realDocument = globalThis.document;
function fakeDocument() {
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: {
      createElement: () => {
        const node: Chip & Record<string, unknown> = { className: "", innerHTML: "", title: "", label: "", text: "" };
        node.querySelector = () => ({ set textContent(value: string) { node.text = value; } });
        node.setAttribute = (name: string, value: string) => { if (name === "aria-label") node.label = value; };
        return node;
      },
    },
  });
}

describe("workspace chips", () => {
  afterEach(() => Object.defineProperty(globalThis, "document", { configurable: true, value: realDocument }));

  it("shows where a workspace runs with the machine icon, whoever created it", () => {
    fakeDocument();
    const where = chip(badge("vps"));
    expect(where?.className).toContain("machine-badge");
    expect(where?.innerHTML).toContain(icon("server", 12));
    expect(where?.innerHTML).not.toContain(icon("sparkles", 12));
    expect(where?.text).toBe("vps");
    expect(where?.title).toBe("vps is offline: agents keep running there and this app updates when it reconnects.");
    expect(badge(undefined)).toBeNull();
  });

  it("shows an agent's authorship apart from the machine, also for work on this computer", () => {
    fakeDocument();
    const named = chip(creatorBadge("openbot"));
    expect(named?.className).toBe("creator-badge");
    expect(named?.innerHTML).toContain(icon("sparkles", 12));
    expect(named?.text).toBe("Created by agent");
    expect(named?.title).toBe("Created by agent openbot");
    expect(named?.label).toBe("Created by agent openbot");
    const unnamed = chip(creatorBadge("", true));
    expect(unnamed?.className).toBe("creator-badge compact");
    expect(unnamed?.text).toBe("");
    expect(unnamed?.title).toBe("Created by agent");
  });

  it("adds nothing for a workspace the person created", () => {
    fakeDocument();
    expect(creatorBadge(undefined)).toBeNull();
  });
});
