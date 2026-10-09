import { expect, it } from "vitest";
import payload from "../fixtures/backend-contract.json";
import { parseConversationEvent } from "./conversation";
import { Timeline } from "./timeline";
import { statusOf, repoLabel } from "./types";
import type { Board } from "./types";

it("consumes serialized backend workspaces without requiring frontend-only fields", () => {
  // The script checks exact literal discriminants; JSON imports widen strings.
  const board = payload.board as Board;
  expect(statusOf(board.workspaces[0])).toBe("querendo");
  expect(repoLabel(board.workspaces[0])).toBe("Project");
  expect(board.workspaces[0].remote).toBeUndefined();
  expect(board.workspaces[1].port).toBeNull();
  // Owner confirmation is a saved choice; a board from before it holds teammates' messages (ADR 0090).
  expect(board.workspaces.map(w => w.confirm_messages)).toEqual([false, true]);
  expect(board.workspaces[0].tabs[0].kickoff).toBe("package/skill");
});

it("replays the serialized snapshot with the same result as live events", () => {
  const replay = new Timeline();
  replay.load(payload.snapshot.text);
  const live = new Timeline();
  for (const event of payload.events.claude) live.push(JSON.stringify(event));
  expect(replay.items).toEqual(live.items);
  expect(replay.working).toBe(live.working);
  expect(payload.snapshot.seq).toBe(payload.events.claude.length);
});

for (const [provider, events] of Object.entries(payload.events)) {
  it(`validates and renders serialized ${provider} adapter output`, () => {
    const timeline = new Timeline();
    for (const event of events) {
      expect(parseConversationEvent(event), JSON.stringify(event)).not.toBeNull();
      timeline.push(JSON.stringify(event));
    }
    expect(timeline.items).toEqual(expect.arrayContaining([
      expect.objectContaining({ kind: "assistant", streaming: false,
        blocks: expect.arrayContaining([expect.objectContaining({ kind: "text", text: "Reviewed" })]) }),
    ]));
    expect(timeline.working).toBe(false);
  });
}
