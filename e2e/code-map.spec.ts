import { expect, test, type Locator } from "./fixtures";

async function expectCompactMapLayout(map: Locator) {
  const layout = await map.evaluate(root => {
    const box = (selector: string) => root.querySelector<HTMLElement>(selector)!.getBoundingClientRect();
    const blocks = [".code-map-canvas", ".code-map-inspector", ".code-map-truncated", ".code-map-footer"]
      .filter(selector => !root.querySelector<HTMLElement>(selector)!.hidden);
    const overlaps = blocks.slice(1).filter((selector, index) => box(blocks[index]).bottom > box(selector).top + 1);
    const pages = root.querySelector<HTMLElement>(".code-map-pages")!;
    if (!pages.hidden && box(".code-map-results").bottom > box(".code-map-pages").top + 1) overlaps.push(".code-map-pages");
    return { overlaps, graphHeight: box(".code-map-stage").height, resultsHeight: box(".code-map-results").height };
  });
  expect(layout.overlaps, "Compact Map regions must not overlap").toEqual([]);
  expect(layout.graphHeight, "The graph keeps usable drawing space").toBeGreaterThanOrEqual(100);
  expect(layout.resultsHeight, "The result list keeps room for a row").toBeGreaterThanOrEqual(40);
}

// The old SVG stalled WebKit while typing and panning. Exercise the replacement Canvas renderer:
// model tests cannot prove browser painting, pointer handling or the accompanying list's focus.
test("large code map preserves canvas and keyboard interaction continuity", { tag: "@webkit" }, async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.setViewportSize({ width: 1280, height: 960 });
  await page.clock.install({ time: new Date("2026-10-31T12:00:00Z") });
  await page.goto("/design-system.html?component=code-map&state=large&embed=1");
  const map = page.locator(".code-map"), graph = map.locator(".code-map-graph");
  const canvases = graph.locator("canvas");
  const rows = map.locator(".code-map-result");
  await expect(canvases.first()).toBeVisible();
  await expect(rows).toHaveCount(4);

  // Canvas nodes have no DOM focus; the real directory/file list remains keyboard accessible.
  const firstDirectory = map.locator('.code-map-result[data-kind="directory"]').first();
  await firstDirectory.focus();
  await expect(firstDirectory).toBeFocused();
  await firstDirectory.press("Enter");
  await expect(rows).toHaveCount(40);
  await expect(map.getByRole("button", { name: "Back to directories", exact: true })).toBeVisible();
  const firstPage = await rows.first().textContent();

  // A short sidebar used to paint the warning and footer over the result list and pager.
  // Constrain the real gallery host to the space available above an open script dock.
  await page.setViewportSize({ width: 425, height: 500 });
  await map.evaluate(element => { element.style.height = "366px"; });
  await expectCompactMapLayout(map);
  await graph.hover();
  await page.mouse.wheel(0, 300);
  await expect.poll(() => map.evaluate(element => element.scrollTop)).toBeGreaterThan(0);
  await rows.last().scrollIntoViewIfNeeded();
  await expect(rows.last()).toBeInViewport();
  await map.getByRole("button", { name: "Sync now", exact: true }).scrollIntoViewIfNeeded();
  await expect(map.locator(".code-map-truncated")).toBeInViewport();
  await expect(map.locator(".code-map-pages")).toBeInViewport();
  await expect(map.getByRole("button", { name: "Sync now", exact: true })).toBeInViewport();
  await map.getByRole("button", { name: "Next page", exact: true }).click();
  await expect(rows).toHaveCount(40);
  await expect(rows.first()).not.toHaveText(firstPage!);
  await map.evaluate(element => { element.style.height = "720px"; });
  await page.setViewportSize({ width: 1280, height: 960 });
  await expect.poll(() => map.evaluate(element => element.scrollHeight - element.clientHeight)).toBeLessThanOrEqual(1);
  await graph.scrollIntoViewIfNeeded();

  const retained = await graph.evaluateHandle(element => ({ element, canvases: [...element.querySelectorAll("canvas")] }));
  const pixels = () => canvases.evaluateAll(items => items.map(canvas => (canvas as HTMLCanvasElement).toDataURL()).join("\n"));
  await graph.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const beforePan = await pixels();
  const box = (await graph.boundingBox())!;
  await page.mouse.move(box.x + 5, box.y + 5);
  await page.mouse.down();
  await page.mouse.move(box.x + 65, box.y + 35, { steps: 8 });
  await page.mouse.up();
  await expect.poll(pixels).not.toBe(beforePan);

  // Hold the story's asynchronous response so a slow machine cannot skip the pending state.
  await page.clock.pauseAt(new Date("2026-10-31T13:00:00Z"));
  const search = map.getByRole("searchbox", { name: "Search symbols, files or keywords", exact: true });
  await search.fill("search");
  await expect(search).toBeFocused();
  expect(await retained.evaluate(state => {
    const current = state.element.querySelectorAll("canvas");
    return current.length === state.canvases.length && state.canvases.every((canvas, index) => canvas === current[index]);
  })).toBe(true);
  await retained.dispose();
  await expect(rows).toHaveCount(40);
  await page.clock.runFor(300);
  await page.clock.resume();
  await expect(rows).toHaveCount(2);
  await rows.first().press("Enter");
  await map.getByRole("button", { name: "Open source", exact: true }).click();
  await expect(page.locator(".story-feedback")).toHaveText("src/search/module-1.ts:12");
  expect(errors).toEqual([]);
});

// Opening an indexed excerpt must use the live file viewer without replacing an unsaved draft.
test("map sync and expanded source navigation preserve the editor draft", { tag: "@webkit" }, async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.locator("#railbody .navitem.sub .lbl").getByText("Hello", { exact: true }).click();
  await page.locator("#tab-map").click();
  const map = page.locator("#code-map .code-map");
  await expect(map.locator("canvas").first()).toBeVisible();
  await expect(page.locator("#dock")).not.toHaveClass(/\bclosed\b/);
  await expectCompactMapLayout(map);
  await testInfo.attach("map-sidebar", { body: await map.screenshot({ animations: "disabled" }), contentType: "image/png" });
  const search = map.getByRole("searchbox", { name: "Search symbols, files or keywords", exact: true });
  await search.fill("findSymbol");
  const match = map.locator(".code-map-result").filter({ has: page.getByText("findSymbol", { exact: true }) });
  await match.press("Enter");
  await map.getByRole("button", { name: "Open source", exact: true }).click();
  const editor = page.locator("#vtext");
  await expect(editor).toHaveValue(/export function findSymbol/);
  const draft = `${await editor.inputValue()}\n// Keep this unsaved editor draft.\n`;
  await editor.fill(draft);

  await map.getByRole("button", { name: "Sync now", exact: true }).click();
  await expect(map.getByRole("button", { name: "Sync now", exact: true })).toBeEnabled();
  await expect(search).toHaveValue("findSymbol");
  await map.getByRole("button", { name: "Expand map", exact: true }).click();
  await expect(map.getByRole("button", { name: "Collapse map", exact: true })).toBeVisible();
  await testInfo.attach("map-expanded", { body: await map.screenshot({ animations: "disabled" }), contentType: "image/png" });
  await match.press("Enter");
  await map.getByRole("button", { name: "Open source", exact: true }).click();
  await expect(editor).toBeVisible();
  await expect(editor).toHaveValue(draft);
  await expect(page.locator("#vsave")).toBeVisible();
});
