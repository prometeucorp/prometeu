import { describe, expect, it } from "vitest";
import { remoteControl, rightOf } from "./team-control";
import { NO_RIGHTS, parseRights, viewerRights } from "./team-rights";

describe("rights in shared workspaces", () => {
  it("needs Control to answer requests and Send messages for everything else", () => {
    const respond = { v: 1, type: "request.respond", requestId: "r1", response: { outcome: "allow" } };
    expect(rightOf(remoteControl(JSON.stringify(respond)))).toBe("control");
    expect(rightOf(remoteControl(JSON.stringify({ type: "control_response", response: { subtype: "success", request_id: "r1", response: { behavior: "allow" } } })))).toBe("control");
    expect(rightOf(remoteControl(JSON.stringify({ v: 1, type: "turn.interrupt" })))).toBe("send");
    expect(rightOf(remoteControl(JSON.stringify({ type: "control_request", request_id: "r2", request: { subtype: "interrupt" } })))).toBe("send");
    expect(rightOf(remoteControl("Merge it"))).toBe("send");
  });

  it("reads people from an announcement and grants nothing for malformed or unknown input", () => {
    expect(parseRights({ send: ["bob", "bob", "not an id"], control: ["ana"], review: ["carol"] })).toEqual({ send: ["bob"], control: ["ana"] });
    expect(parseRights({ send: "bob" })).toEqual(NO_RIGHTS);
    expect(parseRights(null)).toEqual(NO_RIGHTS);
    expect(parseRights({ send: Array.from({ length: 80 }, (_, i) => `p${i}`) }).send).toHaveLength(64);
  });

  it("lets a viewer act only where the owner's announcement says so", () => {
    const rights = { send: ["bob"], control: ["ana"] };
    expect(viewerRights(rights, "bob", false)).toEqual({ send: true, control: false });
    expect(viewerRights(rights, "ana", false)).toEqual({ send: false, control: true });
    expect(viewerRights(rights, "carol", false)).toEqual({ send: false, control: false });
    // The owner's devices act through remote control, and owners from before rights announce none.
    expect(viewerRights(NO_RIGHTS, "alice", true)).toEqual({ send: true, control: true });
    expect(viewerRights(undefined, "carol", false)).toEqual({ send: true, control: true });
  });
});
