import { afterEach, describe, expect, it, vi } from "vitest";
import { icon } from "./icons";
import { t } from "./i18n";
import { aliases, badge, codexLine, creatorBadge, projectsOn, readinessOf, type MachineView } from "./machines";

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

it("tells what keeps Codex from running on a machine, in the order to fix it", () => {
  const ready = { installed: true, version: "codex-cli 0.169.0", supported: true, loggedIn: true, minimum: "0.169.0" };
  expect(codexLine(ready)).toBe(t("machines.codex.signedIn"));
  expect(codexLine({ ...ready, loggedIn: false })).toBe(t("machines.codex.signedOut"));
  expect(codexLine({ ...ready, version: "codex-cli 0.100.0", supported: false, loggedIn: false }))
    .toBe(t("machines.codex.old", { version: "codex-cli 0.100.0", minimum: "0.169.0" }));
  expect(codexLine({ installed: false, version: null, supported: false, loggedIn: false, minimum: "0.169.0" }))
    .toBe(t("machines.codex.missing"));
});

it("asks only the chosen agent to be ready on the machine", () => {
  const machine = {
    id: "vps", name: "VPS", ssh: "vps", limits: {}, phase: "connected", detail: null, host: null, update_pending: false,
    claude: { installed: true, loggedIn: false, email: null, method: null },
    codex: { installed: true, version: "codex-cli 0.169.0", supported: true, loggedIn: true, minimum: "0.169.0" },
  } as unknown as MachineView;
  expect(readinessOf(machine, "codex")).toEqual({ problem: "", account: t("machines.codex.account") });
  expect(readinessOf(machine, "claude").problem).toBe(t("launcher.remoteSignIn", { name: "VPS" }));
  const signedOut = { ...machine, codex: { ...machine.codex!, loggedIn: false } };
  expect(readinessOf(signedOut, "codex").problem).toBe(t("err.remote.codexSignIn", { name: "VPS" }));
  // An unchecked machine does not block the launcher; the launch checks again.
  expect(readinessOf({ ...machine, codex: null }, "codex").problem).toBe("");
  expect(readinessOf(machine, "cursor").problem).toBe(t("err.remote.provider"));
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
