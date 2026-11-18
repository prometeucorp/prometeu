import { expect, it, vi } from "vitest";
import type { Shared } from "../relay/src/protocol";
import type { Context } from "./team-ports";
import * as viewer from "./team-viewer";

it("blocks native decision replies before transmission while retaining prompt and interrupt input", async () => {
  const share = { id: "workspace", owner: "alice", online: false,
    tabs: [{ id: "tab", decision_surface: "terminal" }],
  } as Shared;
  const sendConfirmed = vi.fn(async () => true), fail = vi.fn();
  const feature = viewer.install({ shares: () => new Map([[share.id, share]]), you: () => "bob", sendConfirmed, fail } as unknown as Context);
  try {
    await viewer.attach(viewer.remotes()[0].id, "tab");
    share.online = true;
    const respond = JSON.stringify({ v: 1, type: "request.respond", requestId: "r", response: { outcome: "allow" } });
    expect(await viewer.write(respond)).toBe(false);
    expect(await viewer.write(JSON.stringify({ type: "control_response", response: {
      subtype: "success", request_id: "r", response: { behavior: "allow" },
    } }))).toBe(false);
    expect(sendConfirmed).not.toHaveBeenCalled();
    expect(fail).toHaveBeenCalledTimes(2);
    expect(await viewer.write("Continue after the owner answers")).toBe(true);
    expect(await viewer.write(JSON.stringify({ v: 1, type: "turn.interrupt" }))).toBe(true);
    delete share.tabs[0].decision_surface;
    expect(await viewer.write(respond)).toBe(true);
    expect(sendConfirmed).toHaveBeenCalledTimes(3);
  } finally {
    feature.reset?.();
  }
});

it("rejects native slash input before transmission while preserving paths, responses and legacy chat", async () => {
  const share = { id: "workspace", owner: "alice", online: false,
    tabs: [{ id: "tab", conversation_mode: "terminal" }],
  } as Shared;
  const sendConfirmed = vi.fn(async () => true), fail = vi.fn();
  const feature = viewer.install({ shares: () => new Map([[share.id, share]]), you: () => "bob", sendConfirmed, fail } as unknown as Context);
  try {
    await viewer.attach(viewer.remotes()[0].id, "tab");
    share.online = true;
    for (const text of ["/compact", " /plugin:command argument", "/"]) expect(await viewer.write(text)).toBe(false);
    expect(sendConfirmed).not.toHaveBeenCalled();
    expect(fail).toHaveBeenCalledTimes(3);
    expect(await viewer.write("/Users/file")).toBe(true);
    expect(await viewer.write("Continue normally")).toBe(true);
    expect(await viewer.write(JSON.stringify({ v: 1, type: "turn.interrupt" }))).toBe(true);
    expect(await viewer.write(JSON.stringify({ v: 1, type: "request.respond", requestId: "r", response: { outcome: "allow" } }))).toBe(true);
    delete share.tabs[0].conversation_mode;
    expect(await viewer.write("/compact")).toBe(true);
    expect(sendConfirmed).toHaveBeenCalledTimes(5);
  } finally {
    feature.reset?.();
  }
});
