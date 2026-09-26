import { test, expect } from "@playwright/test";
import { build } from "vite";

let assets: Record<string, string>;
test.beforeAll(async () => {
  // Build the real mobile entry with an encrypted local peer, without shipping test hooks in the app.
  const result = await build({ configFile: false, logLevel: "silent", build: {
    write: false, target: "esnext", minify: false,
    rollupOptions: { input: "e2e/fixtures/mobile.ts", output: { inlineDynamicImports: true, entryFileNames: "mobile-test.js", assetFileNames: "mobile-test.[ext]" } },
  } });
  const output = "output" in result ? result.output : [];
  assets = Object.fromEntries(output.map(asset => [asset.fileName, asset.type === "chunk" ? asset.code : String(asset.source)]));
});

test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });
test.beforeEach(async ({ page }) => {
  await page.route("**/mobile-test", route => route.fulfill({ contentType: "text/html", body: `<!doctype html>
    <html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
    <link rel="stylesheet" href="/mobile-test.css"><script type="module" src="/mobile-test.js"></script></head>
    <body><div id="app" data-origin="https://cloud.test" data-user-id="user1" data-user-name="Alice" data-lang="en"
    data-organizations='[{"id":"organization1","name":"My organization","slug":"one","member":"marcus"}]'></div></body></html>` }));
  await page.route("**/mobile-test.*", route => {
    const name = new URL(route.request().url()).pathname.slice(1);
    return route.fulfill({ contentType: name.endsWith(".js") ? "text/javascript" : "text/css", body: assets[name] });
  });
  await page.route("**/orgs/one/companion-ticket", route => route.fulfill({ json: { ticket: "t".repeat(43), relay: "https://relay.test" } }));
  await page.goto("/mobile-test");
  await page.getByRole("button", { name: "Conversation" }).click();
  await expect(page.locator(".m-tool")).toBeVisible();
});

test("mobile contains long commands and messages and keeps sending accessible", { tag: "@webkit" }, async ({ page }) => {
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    expect(await page.evaluate(() => {
      const transcript = document.querySelector(".m-transcript")!;
      return document.documentElement.scrollWidth <= innerWidth && transcript.scrollWidth <= transcript.clientWidth;
    })).toBe(true);
    const tool = page.locator(".m-tool span");
    expect(await tool.evaluate(node => node.scrollWidth > node.clientWidth)).toBe(true);
    const code = page.locator(".m-md pre");
    expect(await code.evaluate(node => node.scrollWidth > node.clientWidth)).toBe(true);
    // Long file names must not squeeze neighboring columns into one letter per line.
    const table = page.locator(".m-md table");
    expect(await table.evaluate(node => {
      const heading = node.querySelector("th:last-child")!;
      const result = node.querySelector("td:last-child")!;
      const lineHeight = parseFloat(getComputedStyle(heading).lineHeight);
      const range = document.createRange();
      range.selectNodeContents(result);
      return heading.getBoundingClientRect().height <= lineHeight + 4 && range.getBoundingClientRect().height <= lineHeight;
    })).toBe(true);
    const composer = page.locator(".m-composer");
    const send = page.getByRole("button", { name: "Send", exact: true });
    expect((await composer.boundingBox())!.height).toBeLessThan(100);
    expect((await send.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await expect(send).toBeDisabled();
  }
  const box = page.getByRole("textbox", { name: "Write to the agent…", exact: true });
  expect(await box.evaluate(node => parseFloat(getComputedStyle(node).fontSize))).toBeGreaterThanOrEqual(16);
  await box.fill("line\n".repeat(30));
  expect((await box.boundingBox())!.height).toBeLessThanOrEqual(160);
  // The visual viewport shrinks when iOS shows the keyboard, while the layout viewport stays tall.
  await page.evaluate(() => {
    Object.defineProperties(visualViewport!, { height: { configurable: true, value: 380 }, offsetTop: { configurable: true, value: 30 } });
    visualViewport!.dispatchEvent(new Event("resize"));
  });
  const composer = await page.locator(".m-composer").boundingBox();
  expect(composer!.y + composer!.height).toBeLessThanOrEqual(410);
  expect((await page.locator(".m-transcript").boundingBox())!.height).toBeGreaterThan(80);
  await page.locator(".m-comments summary").click();
  const expanded = await page.locator(".m-composer").boundingBox();
  expect(expanded!.y + expanded!.height).toBeLessThanOrEqual(410);
});

test("mobile preserves drafts when sending fails or the Mac disconnects", { tag: "@webkit" }, async ({ page }) => {
  const box = page.getByRole("textbox", { name: "Write to the agent…", exact: true });
  const send = page.getByRole("button", { name: "Send", exact: true });
  await box.fill("Merge it");
  await page.evaluate("window.mobileTest.failSend()");
  await send.tap();
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(box).toHaveValue("Merge it");
  await expect(send).toBeEnabled();
  await send.tap();
  await expect(box).toHaveValue("");
  await expect(box).toBeFocused();
  await box.fill("Continue later");
  await page.evaluate("window.mobileTest.offline()");
  await expect(send).toBeDisabled();
  await expect(box).toHaveValue("Continue later");
  await expect(box).toBeEditable();
  await expect(page.locator(".m-connection")).toContainText("offline");
});

test("mobile shows browser context as a tag and preserves the conversation when closing details", { tag: "@webkit" }, async ({ page }) => {
  const message = page.locator(".m-user").filter({ hasText: "Adjust this element on mobile." });
  await expect(message).toHaveText("Adjust this element on mobile. Selected element Keep the button text. Review · 1 note · 1 file");
  await expect(message).not.toContainText(/prometeu-browser-element|"selector"|<button|mobile-browser-context\.png/);
  await expect(message.locator(".browser-context-remove")).toHaveCount(0);
  const box = page.getByRole("textbox", { name: "Write to the agent…", exact: true });
  await box.fill("Preserve my draft");
  const chip = message.getByRole("button", { name: "Selected element", exact: true });
  await chip.tap();
  const dialog = page.getByRole("dialog", { name: "Selected element", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(".browser-context-selector")).toHaveText("#mobile-design-button");
  await expect(dialog.locator(".browser-context-url")).toHaveText("https://example.com/design");
  await expect(dialog.locator(".browser-context-text")).toHaveText("Continue");
  await expect(dialog.locator(".browser-context-html")).toHaveText('<button id="mobile-design-button">Continue</button>');
  await expect(dialog.locator(".browser-context-css")).toHaveText("color: #123456;");
  await dialog.getByRole("button", { name: "Close", exact: true }).tap();
  await expect(dialog).toHaveCount(0);
  await expect(chip).toBeVisible();
  await expect(box).toHaveValue("Preserve my draft");
  await message.locator(".review-context button").tap();
  const review = page.getByRole("dialog", { name: "Review details", exact: true });
  await expect(review).toContainText("src/button.ts");
  await expect(review).toContainText("Preserve keyboard access.");
  await review.getByRole("button", { name: "Close", exact: true }).tap();
  await expect(box).toHaveValue("Preserve my draft");
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeEnabled();
  expect(await page.evaluate(() => {
    const transcript = document.querySelector(".m-transcript")!;
    const composer = document.querySelector(".m-composer")!.getBoundingClientRect();
    return document.documentElement.scrollWidth <= innerWidth && transcript.scrollWidth <= transcript.clientWidth
      && transcript.clientHeight > 80 && composer.bottom <= innerHeight;
  })).toBe(true);
});
