import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import type { Share } from "../relay/src/protocol";
import * as comments from "./team-comments";
import * as member from "./team-member";
import { simulatedSocket } from "./team-mock";
import * as viewer from "./team-viewer";

/** A browser shell composes the core without Tauri: member, viewer and comments only, storage and transport injected. */

const share: Share = {
  id: "ws-marcus", title: "Archive completed items", repo_name: "capim", branch: "fix/archive", stage: "Fazendo", issue: null,
  active: "mt1", tabs: [{ id: "mt1", title: "", status: "rodando", note: null, tokens: null }], sizes: { mt1: [80, 24] }, audience: null,
};
const storage = new Map<string, string>();
let rights: { send: string[]; control: string[] } | undefined;

beforeEach(() => {
  rights = undefined;
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key),
  });
  storage.set("mock:team", JSON.stringify({ team: "fakeTeam", relay: "ws://mock" }));
  let security: unknown = null;
  member.useSecurityStore({ read: async () => structuredClone(security), write: async (state) => { security = structuredClone(state); } });
  member.useTransport({
    needsRelay: false, create: vi.fn(), enroll: vi.fn(),
    socket: (url) => simulatedSocket(url, "first conversation line\n", share, () => true, rights),
  });
});
// Features register once per module, as in a real shell.
beforeAll(() => {
  member.register(comments.install);
  member.register(viewer.install);
});
afterEach(() => { member.reset(); vi.unstubAllGlobals(); });

it("watches a remote share and reads its inbox with only the viewer and comments features", async () => {
  const scope = JSON.stringify(["team", "ws://mock", "fakeTeam"]);
  await member.connect({
    member: "eu_mock", scope, privateScope: JSON.stringify([scope, "", "eu_mock"]), shareScope: "team:fakeTeam", legacy: true,
    url: async () => "ws://mock/team/fakeTeam?m=eu_mock&n=Me",
  });
  await vi.waitFor(() => expect(member.current().phase).toBe("online"));
  await vi.waitFor(() => expect(viewer.remotes()).toHaveLength(1));
  const [remote] = viewer.remotes();
  expect(remote.share.title).toBe(share.title);
  expect(viewer.isRemote(remote.id)).toBe(true);
  expect(viewer.relayId(remote.id)).toBe(share.id);

  const attached = await viewer.attach(remote.id, "mt1");
  expect(new TextDecoder().decode(attached!.bytes)).toContain("first conversation line");
  expect(viewer.attachedTab()).toBe("mt1");

  await vi.waitFor(() => expect(comments.inboxCount()).toBe(1));
  expect(comments.inboxList()[0]).toMatchObject({ ws: share.id, author: "Marcus Hale", title: share.title });
  expect(comments.inboxList()[0].text).toContain("@Me");
  // The mention arrived as a live note; the thread is already cached without a request.
  expect(comments.notesOf(share.id)).toHaveLength(1);
  expect(await comments.addNote(share.id, null, null, "reply from the phone", [], null)).toBe(true);
  await vi.waitFor(() => expect(comments.notesOf(share.id).some(note => note.text === "reply from the phone")).toBe(true));
});

it("reflects the rights the owner announced, so a viewer without them sees no action controls", async () => {
  rights = { send: [], control: ["eu_mock"] };
  const scope = JSON.stringify(["team", "ws://mock", "fakeTeam"]);
  await member.connect({
    member: "eu_mock", scope, privateScope: JSON.stringify([scope, "", "eu_mock"]), shareScope: "team:fakeTeam", legacy: true,
    url: async () => "ws://mock/team/fakeTeam?m=eu_mock&n=Me",
  });
  await vi.waitFor(() => expect(viewer.remotes()).toHaveLength(1));
  const [remote] = viewer.remotes();
  expect(viewer.rightsIn(remote.id)).toEqual({ send: false, control: true });
  expect(viewer.announcedRights(remote.id)).toEqual({ send: [], control: ["eu_mock"] });
});
