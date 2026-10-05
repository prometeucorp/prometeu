import { expect, test } from "@playwright/test";
import type { Workflow } from "../src/automations-model";

type AutomationWindow = Window & {
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
  automationRequests: Record<string, unknown>[];
  finishAutomation: (reply: unknown) => void;
  failAutomation: (error: string) => void;
};

// Browser exception: navigation preserves unsent drafts; Mermaid's SVG boundary must
// render hostile labels as text without executing markup or changing the saved graph.
test("workflow library and read-only diagram preserve drafts and graph data @webkit", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.locator("#railbody").getByRole("button", { name: "Automations", exact: true }).click();
  const editor = page.locator("#automationsView");
  const library = page.getByRole("region", { name: "My workflows", exact: true });
  await expect(library).toContainText("No saved workflows in this app yet.");
  await expect(library.getByRole("button", { name: "Use template", exact: true })).toHaveCount(4);
  await editor.getByRole("button", { name: "New workflow", exact: true }).click();
  const create = page.getByRole("dialog", { name: "New workflow", exact: true });
  await create.getByLabel("Workflow name", { exact: true }).fill("Unfinished draft");
  await create.getByRole("button", { name: "New workflow", exact: true }).click();
  const request = editor.getByLabel("Describe a workflow or a change…", { exact: true });
  await request.fill("Preserve this request while browsing workflows");
  await editor.getByRole("button", { name: "My workflows", exact: true }).click();
  await library.getByRole("button", { name: "Continue editing", exact: true }).click();
  await expect(request).toHaveValue("Preserve this request while browsing workflows");
  const original = await page.evaluate(async () => {
    const workflow: Workflow = {
      id: "diagram-example", name: "Durable graph", revision: 0, enabled: false, scope: {},
      policy: { maxConcurrentRuns: 1, requireMergeApproval: true, allowWrites: false, maxRetries: 0, maxAgentTurns: 1 },
      nodes: [
        { id: "event", label: "Event", position: { x: 150, y: 140 }, config: { type: "trigger", event: "manual", baseline: "ignoreExisting" } },
        { id: "condition", label: "Ready?", position: { x: 320, y: 140 }, config: { type: "condition", path: "event.ready", operator: "truthy" } },
        { id: "review", label: '<img src=x onerror="window.diagramInjected=true">', position: { x: 600, y: 400 }, config: { type: "approval", message: "Review a failed check" } },
      ],
      edges: [{ from: "event", to: "condition", port: "next" }, { from: "condition", to: "review", port: "error" }],
    };
    return await (window as AutomationWindow).__TAURI_INTERNALS__.invoke("automations_save", { workflow, expectedRevision: null }) as Workflow;
  });
  await editor.getByRole("button", { name: "My workflows", exact: true }).click();
  await library.getByRole("button", { name: "Open workflow", exact: true }).click();
  const discard = page.getByRole("dialog", { name: "Discard unsaved changes and switch workflows?", exact: true });
  await discard.getByRole("button", { name: "Cancel", exact: true }).click();
  await library.getByRole("button", { name: "Continue editing", exact: true }).click();
  await expect(request).toHaveValue("Preserve this request while browsing workflows");
  await editor.getByRole("button", { name: "My workflows", exact: true }).click();
  await library.getByRole("button", { name: "Open workflow", exact: true }).click();
  await discard.getByRole("button", { name: "Discard", exact: true }).click();
  const diagram = editor.getByRole("img", { name: "Durable graph", exact: true });
  await expect(diagram).toBeVisible();
  await expect(diagram.locator("g.node")).toHaveCount(2);
  await expect(diagram).toContainText("Ready?");
  await expect(editor).toContainText("1 exception paths hidden");
  await expect(editor.getByRole("button", { name: "Add step", exact: true })).toHaveCount(0);
  await expect(editor.getByRole("button", { name: "Arrange steps", exact: true })).toHaveCount(0);
  const detailed = editor.getByLabel("Include error and uncertainty paths", { exact: true });
  await detailed.check();
  await expect(diagram.locator("g.node")).toHaveCount(3);
  await expect(diagram.locator("g.node").last()).toContainText(/<img src=x\s*onerror="window.diagramInjected=true">/);
  await expect(diagram.locator("img, image, script, a, foreignObject")).toHaveCount(0);
  expect(await page.evaluate(() => (window as Window & { diagramInjected?: boolean }).diagramInjected)).toBeUndefined();
  await detailed.uncheck(); await detailed.check(); await detailed.uncheck();
  await expect(diagram.locator("g.node")).toHaveCount(2);
  await editor.getByRole("button", { name: "Edit in conversation", exact: true }).click();
  await request.fill("Keep this draft while inspecting the diagram");
  await editor.getByRole("button", { name: "Workflow", exact: true }).click();
  await expect(diagram).toBeVisible();
  await editor.getByRole("button", { name: "Steps", exact: true }).click();
  await expect(editor.locator(".automations-outline-item")).toHaveCount(3);
  await editor.getByRole("button", { name: "Conversation", exact: true }).click();
  await expect(request).toHaveValue("Keep this draft while inspecting the diagram");
  const current = await page.evaluate(async () => {
    const snapshot = await (window as AutomationWindow).__TAURI_INTERNALS__.invoke("automations_snapshot") as { workflows: Workflow[] };
    return snapshot.workflows[0];
  });
  expect(current).toEqual(original);
  await editor.getByRole("button", { name: "My workflows", exact: true }).click();
  await page.screenshot({ path: testInfo.outputPath("workflow-library.png") });
});

// Browser exception: an unavailable provider must keep the first request editable.
test("conversation starts a draft and preserves the request after provider failure", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(() => {
    const host = window as AutomationWindow;
    const original = host.__TAURI_INTERNALS__.invoke;
    host.automationRequests = [];
    host.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command !== "automations_propose") return original(command, args);
      host.automationRequests.push(args ?? {});
      if (host.automationRequests.length === 1) return Promise.reject("automation_proposal_format: Invalid schema for response_format 'codex_output_schema': unsupported enum literal");
      return Promise.resolve({ workflow: null, summary: "Which repository should I watch?" });
    };
  });
  await page.locator("#railbody").getByRole("button", { name: "Automations", exact: true }).click();
  const editor = page.locator("#automationsView");
  await editor.getByRole("button", { name: "New workflow", exact: true }).click();
  await page.getByRole("dialog", { name: "New workflow", exact: true }).getByRole("button", { name: "New workflow", exact: true }).click();
  const request = editor.getByLabel("Describe a workflow or a change…", { exact: true });
  await request.fill("Review my pull requests when checks fail");
  await editor.getByRole("button", { name: "Send", exact: true }).click();
  await expect(editor.locator('[role="alert"]')).toContainText("This is an app integration error");
  await expect(request).toHaveValue("Review my pull requests when checks fail");
  await expect(editor.locator(".turn.user")).toHaveText("Review my pull requests when checks fail");
  expect(await page.evaluate(() => (window as AutomationWindow).automationRequests.length)).toBe(1);
  // Permissions can be chosen once without losing the failed conversation draft.
  await editor.getByRole("button", { name: "Scope and policy", exact: true }).click();
  const policy = page.getByRole("dialog", { name: "Scope and policy", exact: true });
  await policy.getByRole("button", { name: "Allow automatic fixes and publication", exact: true }).click();
  await expect(policy.getByLabel("Require local checks before commit and publication", { exact: true })).not.toBeChecked();
  await expect(policy.getByLabel("Require approval of the exact commit before publication", { exact: true })).not.toBeChecked();
  await policy.getByRole("button", { name: "Apply settings", exact: true }).click();
  await expect(policy).toBeHidden();
  await expect(request).toHaveValue("Review my pull requests when checks fail");
  await request.fill("Keep my next message while retrying");
  await editor.getByRole("button", { name: "Try again", exact: true }).click();
  await expect(editor.locator(".turn.bot").last()).toContainText("Which repository should I watch?");
  await expect(request).toHaveValue("Keep my next message while retrying");
  await expect(editor.locator(".turn.user")).toHaveCount(1);
  expect(await page.evaluate(() => (window as AutomationWindow).automationRequests[1].history)).toEqual([]);
  expect(await page.evaluate(() => (window as AutomationWindow).automationRequests[1].prompt)).toBe("Review my pull requests when checks fail");
  expect(await page.evaluate(() => ((window as AutomationWindow).automationRequests[1].workflow as Workflow).policy)).toMatchObject({
    allowWrites: true, allowCommit: true, allowPush: true, requirePublishApproval: false, requireLocalChecks: false, requireMergeApproval: true,
  });
});

// Browser exception: delayed replies must not hide sent text, shrink long messages,
// or replace a follow-up being typed. Both engines exercise real scroll/focus behavior.
test("automation conversation keeps pending messages readable and carries follow-up context @webkit", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(() => {
    const host = window as AutomationWindow;
    const original = host.__TAURI_INTERNALS__.invoke;
    host.automationRequests = [];
    host.__TAURI_INTERNALS__.invoke = (command, args) => {
      if (command !== "automations_propose") return original(command, args);
      host.automationRequests.push(args ?? {});
      return new Promise((resolve, reject) => { host.finishAutomation = resolve; host.failAutomation = reject; });
    };
  });
  await page.locator("#railbody").getByRole("button", { name: "Automations", exact: true }).click();
  const editor = page.locator("#automationsView");
  await editor.getByRole("button", { name: "New workflow", exact: true }).click();
  await page.getByRole("dialog", { name: "New workflow", exact: true }).getByRole("button", { name: "New workflow", exact: true }).click();
  const request = editor.getByLabel("Describe a workflow or a change…", { exact: true });
  const original = "Watch my pull requests.\n" + "If a check fails, explain the failure and ask before publishing changes.\n".repeat(12);
  await request.fill(original);
  await request.press("Enter");
  await expect(editor.locator(".turn.user")).toHaveText(original.trim());
  await expect(editor.getByRole("status", { name: "Preparing a reply…" })).toBeVisible();
  await expect(request).toHaveValue("");
  await expect(editor.getByRole("button", { name: "Save revision", exact: true })).toHaveCount(0);
  await expect(editor.locator(".composer .actionsbtn")).toBeHidden();
  await request.fill("Only the njord repository");
  await request.evaluate(element => { (element as HTMLTextAreaElement).setSelectionRange(5, 9); });
  await page.screenshot({ path: testInfo.outputPath("conversation-pending.png") });
  const question = "Which repository should I watch?\n\nI will ask before publishing changes.";
  await page.evaluate(summary => (window as AutomationWindow).finishAutomation({ workflow: null, summary }), question);
  await expect(editor.locator(".turn.bot")).toContainText("Which repository should I watch?");
  await expect(request).toHaveValue("Only the njord repository");
  await expect(request).toBeFocused();
  expect(await request.evaluate(element => [(element as HTMLTextAreaElement).selectionStart, (element as HTMLTextAreaElement).selectionEnd])).toEqual([5, 9]);
  await expect(editor.locator(".automations-proposal-review")).toHaveCount(0);
  await page.clock.install();
  await request.press("Enter");
  const followup = await page.evaluate(() => (window as AutomationWindow).automationRequests[1]);
  expect(followup.prompt).toBe("Only the njord repository");
  expect(followup.history).toEqual([{ role: "user", text: original.trim() }, { role: "assistant", text: question }]);
  await request.fill("Do not merge automatically");
  await page.clock.fastForward(31_000);
  await expect(editor.getByRole("status", { name: "Preparing a reply…" })).toContainText("Still waiting for the agent");
  await expect(request).toHaveValue("Do not merge automatically");
  await page.evaluate(() => (window as AutomationWindow).failAutomation("automation_proposal_timeout"));
  await expect(editor.getByRole("alert").last()).toContainText("did not finish within 5 minutes");
  await expect(editor.getByRole("alert").last()).not.toContainText("Configure a supported agent");
  await editor.getByRole("button", { name: "Try again", exact: true }).click();
  expect(await page.evaluate(() => (window as AutomationWindow).automationRequests[2])).toEqual(followup);
  await expect(editor.locator(".turn.user")).toHaveCount(2);
  await expect(request).toHaveValue("Do not merge automatically");
  const workflow: Workflow = { ...(followup.workflow as Workflow), name: "Review check failures", nodes: [{ id: "start", label: "Start manually", position: { x: 60, y: 60 }, config: { type: "trigger", event: "manual", baseline: "ignoreExisting" } }] };
  const summary = "## Proposed behavior\n\nKeep human approval before publication.\n\n" + "This is a preview for review. It does not run any checks or change a repository.\n\n".repeat(12);
  await page.evaluate(reply => (window as AutomationWindow).finishAutomation(reply), { workflow, summary });
  await expect(editor.getByRole("heading", { name: "Proposed behavior", exact: true })).toBeInViewport();
  await expect(request).toHaveValue("Do not merge automatically");
  expect(await editor.evaluate(root => {
    const thread = root.querySelector<HTMLElement>(".feed")!;
    const composer = root.querySelector<HTMLElement>(".composer")!;
    const lastReply = root.querySelectorAll<HTMLElement>(".turn.bot")[1];
    return thread.clientHeight > 250 && thread.scrollHeight > thread.clientHeight
      && lastReply.clientHeight > thread.clientHeight && composer.clientHeight < 250
      && thread.getBoundingClientRect().bottom <= composer.getBoundingClientRect().top + 1;
  })).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("conversation-reply.png") });
  const apply = editor.getByRole("button", { name: "Apply to draft", exact: true });
  await apply.scrollIntoViewIfNeeded();
  await editor.getByText("View 1 proposed steps", { exact: true }).click();
  await expect(editor.locator(".automations-proposal-outline")).toContainText("Start manually");
  await apply.click();
  await expect(editor.getByRole("button", { name: "Save revision", exact: true })).toBeVisible();
  await expect(request).toHaveValue("Do not merge automatically");
  await request.press("Enter");
  const next = await page.evaluate(() => (window as AutomationWindow).automationRequests[3]);
  expect((next.workflow as Workflow).nodes[0].id).toBe("start");
  expect(next.history).toEqual([
    { role: "user", text: original.trim() }, { role: "assistant", text: question },
    { role: "user", text: "Only the njord repository" }, { role: "assistant", text: summary },
  ]);
  await request.fill("Fix the proposal without changing the scope");
  await page.evaluate(() => (window as AutomationWindow).failAutomation("automation_proposal_schema: missing field `config`"));
  await expect(editor.getByRole("alert").last()).toContainText("missing field `config`");
  await expect(editor.getByRole("alert").last()).not.toContainText("Configure a supported agent");
  await expect(request).toHaveValue("Fix the proposal without changing the scope");
  await expect(request).toBeFocused();
  await request.press("Enter");
  const retry = await page.evaluate(() => (window as AutomationWindow).automationRequests[4]);
  expect(retry.workflow).toEqual(next.workflow);
  expect((retry.history as {role: string; text: string}[]).at(-1)).toMatchObject({ role: "validation", text: expect.stringContaining("missing field `config`") });
  await page.evaluate(() => (window as AutomationWindow).finishAutomation({ workflow: null, summary: "I will keep the approval requirement." }));
  await expect(editor.locator(".turn.bot").last()).toContainText("keep the approval requirement");
});

// Browser exception: Vite can reload chat CSS after feature CSS. A populated
// workflow, failed save and open inspector must still occupy separate views.
test("automation save recovery keeps conversation and graph in separate readable views @webkit", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.evaluate(async () => {
    const host = window as AutomationWindow;
    const original = host.__TAURI_INTERNALS__.invoke;
    const snapshot = await original("automations_snapshot") as { workflows: Workflow[] };
    const board = await original("load_board") as { projects: { id: string }[] };
    const workflow: Workflow = {
      id: "save-recovery", name: "Watch CI, reviews and risk on my pull requests", revision: 0, enabled: false,
      policy: { maxConcurrentRuns: 1, requireMergeApproval: true, allowWrites: false, maxRetries: 0, maxAgentTurns: 1 },
      scope: { targets: [{ projectId: board.projects[0].id, repository: "owner/project" }] },
      nodes: [
        { id: "start", label: "My pull requests", position: { x: 60, y: 60 }, config: { type: "trigger", event: "github.authored_pr", baseline: "ignoreExisting" } },
        { id: "diagnose", label: "Diagnose CI", position: { x: 320, y: 60 }, config: { type: "agent", prompt: "Explain the failed checks", outputSchema: { type: "object", required: ["result"], properties: { result: { type: "string" } } } } },
        ...Array.from({ length: 8 }, (_, i) => ({ id: `review-${i}`, label: `Review step ${i + 1}`, position: { x: 60 + (i % 4) * 260, y: 250 + Math.floor(i / 4) * 190 }, config: { type: "approval" as const, message: "Review before proceeding" } })),
      ],
      edges: [],
    };
    workflow.edges = workflow.nodes.slice(1).map((node, i) => ({ from: workflow.nodes[i].id, to: node.id, port: "next" }));
    let firstSnapshot = true;
    host.automationRequests = [];
    host.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "automations_snapshot" && firstSnapshot) { firstSnapshot = false; return { ...snapshot, workflows: [workflow] }; }
      if (command === "automations_propose") {
        host.automationRequests.push(args ?? {});
        const corrected = structuredClone(args!.workflow as Workflow);
        const agent = corrected.nodes.find(node => node.config.type === "agent")!;
        if (agent.config.type === "agent") agent.config.outputSchema = { type: "object", required: ["summary", "outcome"], properties: { summary: { type: "string" }, outcome: { type: "string" } } };
        corrected.scope.targets![0].identity = "connected-user";
        return { workflow: corrected, summary: "The agent response now matches the supported format. The connected GitHub account is included for review." };
      }
      return original(command, args);
    };
  });
  await page.locator("#railbody").getByRole("button", { name: "Automations", exact: true }).click();
  // Reproduce the dev/HMR cascade from the report, using the real shared stylesheet.
  await page.addStyleTag({ path: "src/components/chat/chat.css" });
  const editor = page.locator("#automationsView");
  await editor.getByRole("button", { name: "Open workflow", exact: true }).click();
  await editor.getByRole("button", { name: "Conversation", exact: true }).click();
  const request = editor.getByLabel("Describe a workflow or a change…", { exact: true });
  const nextDraft = "Keep human approval before every publication";
  await request.fill(nextDraft);
  await editor.getByRole("button", { name: "Save revision", exact: true }).click();
  await expect(editor.getByText("Resolve 2 items before saving", { exact: true })).toBeVisible();
  await expect(request).toBeVisible();
  await expect(editor.locator(".automations-main")).toBeHidden();
  await editor.getByText("Resolve 2 items before saving", { exact: true }).click();
  await expect(editor.getByText("Diagnose CI: The agent step uses an unsupported response format. Ask the assistant to correct it.")).toBeVisible();
  await editor.getByRole("button", { name: "Edit in conversation", exact: true }).click();
  const editedDraft = nextDraft + "\n\nI want to change the step “Diagnose CI”. ";
  await expect(request).toHaveValue(editedDraft);
  await editor.getByRole("button", { name: "Workflow", exact: true }).click();
  await expect(editor.getByRole("img", { name: "Watch CI, reviews and risk on my pull requests", exact: true })).toBeVisible();
  await editor.getByRole("button", { name: "Steps", exact: true }).click();
  const steps = editor.locator(".automations-outline-item");
  await expect(steps).toHaveCount(10);
  await steps.nth(1).getByRole("button", { name: "3. Review step 1", exact: true }).click();
  await expect(steps.nth(2)).toBeFocused();
  for (const width of [1280, 1024]) {
    await page.setViewportSize({ width, height: 800 });
    expect(await steps.nth(2).evaluate(item => {
      const title = item.querySelector(".desktop-item-title")!;
      const list = item.closest(".automations-outline")!;
      return parseFloat(getComputedStyle(title).fontSize) >= 14 && list.scrollWidth <= list.clientWidth + 1;
    })).toBe(true);
  }
  await page.screenshot({ path: testInfo.outputPath("workflow-steps.png") });
  await editor.getByRole("button", { name: "Fix with assistant", exact: true }).click();
  await expect(request).toBeVisible();
  await expect(editor.locator(".automations-main")).toBeHidden();
  await expect(request).toHaveValue(editedDraft);
  await expect(editor.getByRole("button", { name: "Apply to draft", exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("conversation-recovery.png") });
  await editor.getByRole("button", { name: "Apply to draft", exact: true }).click();
  await expect(editor.locator(".automations-validation")).toBeHidden();
  await editor.getByRole("button", { name: "Save revision", exact: true }).click();
  await expect(editor.locator(".automations-notice")).toHaveText("Revision saved");
  await expect(request).toHaveValue(editedDraft);
  const requests = await page.evaluate(() => (window as AutomationWindow).automationRequests);
  expect(requests).toHaveLength(1);
  expect((requests[0].workflow as Workflow).nodes[1].config).toMatchObject({ outputSchema: { required: ["result"] } });
});
