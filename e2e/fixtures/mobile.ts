import { encodeReviewContext } from "../../src/review-context";
import "../../packages/design-system/components.css";
import { companionId } from "../../src/mobile/shell";
import * as member from "../../src/team-member";
import { simulatedSocket } from "../../src/team-mock";
import { encodeBrowserContext } from "../../src/browser-context";
import type { BrowserSelection } from "../../src/browser-types";
import type { Share } from "../../relay/src/protocol";

const long = "gh pr view 51 --json url,state,mergeable,headRefOid,statusCheckRollup && ".repeat(20);
const selection: BrowserSelection = {
  url: "https://example.com/design", selector: "#mobile-design-button", tag: "button", text: "Continue",
  html: '<button id="mobile-design-button">Continue</button>', styles: { color: "#123456" },
  rect: { x: 12, y: 24, width: 160, height: 44 }, viewport: { width: 390, height: 844 },
};
const sample = [
  { type: "user", message: { role: "user", content: "https://example.com/" + "workspace/".repeat(80) } },
  { type: "user", message: { role: "user", content: `Adjust this element on mobile.\n\n${encodeBrowserContext({ selection, image: "/tmp/mobile-browser-context.png" })}\n\nKeep the button text.\n\n${encodeReviewContext({ batch: "mobile-review", comments: [{ n: 1, repo: "app", file: "src/button.ts", in: "worktree", old: null, new: [4, 4], hunk: "@@ -4 +4 @@", excerpt: ["+button"], body: "Preserve keyboard access." }] })}` } },
  { type: "assistant", message: { id: "answer", role: "assistant", content: [
    { type: "tool_use", id: "tool", name: "Bash", input: { command: long } },
    { type: "text", text: `Long message ${long}\n\n\`\`\`sh\n${long}\n\`\`\`\n\n| File | Result |\n| --- | --- |\n| ${"code".repeat(150)} | Done |` },
  ] } },
].map(line => JSON.stringify(line)).join("\n") + "\n";
const share: Share = {
  id: "workspace1", title: "Implement the issue suggestion", repo_name: "prometeu", branch: "main", stage: "", issue: null,
  active: "mt1", tabs: [{ id: "mt1", title: "Conversation", status: "pronta", note: null, tokens: null }], sizes: {}, audience: null,
};
localStorage.setItem("mock:team", JSON.stringify({ team: "organization1", cloud: { origin: "https://cloud.test" } }));
let failSend = false;
let online = true;
let socket: ReturnType<typeof simulatedSocket>;
member.useTransport({
  needsRelay: false,
  create: async () => { throw new Error("unused"); }, enroll: async () => { throw new Error("unused"); },
  socket: () => {
    socket = simulatedSocket(`ws://mock?m=${companionId(localStorage)}&n=Alice`, sample, share, () => online);
    const send = socket.send;
    socket.send = data => {
      if (failSend && typeof data === "string" && data.startsWith('{"t":"write"')) {
        failSend = false;
        throw new Error("test send failed");
      }
      send(data);
    };
    return socket;
  },
});
Object.assign(window, { mobileTest: {
  failSend: () => { failSend = true; },
  offline: () => { online = false; socket.presence(); },
} });
await import("../../src/mobile/main");
