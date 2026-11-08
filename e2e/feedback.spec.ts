import { expect, test, type Page } from "./fixtures";

const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aHlcAAAAASUVORK5CYII=", "base64");

const account = { user: { id: "cloud-user", name: "Gustavo Brancaglione", email: "gustavo@example.com" }, origin: "https://app.prometeu.co", offline: false };

const signedIn = (failures = 0) => `
  localStorage.setItem("mock:cloud", ${JSON.stringify(JSON.stringify(account))});
  localStorage.setItem("mock:feedbackFailures", "${failures}");
`;

test("feedback preserves draft and attachment on failure and retries the same submission", async ({ page }) => {
  await page.addInitScript(signedIn(1));
  await page.goto("/");
  const trigger = page.getByRole("button", { name: "Feedback", exact: true });
  await expect(page.locator(".railfoot").getByRole("button", { name: "Feedback", exact: true })).toBeVisible();
  // Native Tab navigation must follow the visible rows when the updater appears.
  const update = page.locator("#update");
  await update.evaluate(button => {
    button.hidden = false;
    button.textContent = "Restart to update";
  });
  await page.locator("#settings").focus();
  await page.keyboard.press("Tab");
  await expect(trigger).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(update).toBeFocused();
  const feedbackBounds = (await trigger.boundingBox())!;
  expect((await update.boundingBox())!.y).toBeGreaterThanOrEqual(feedbackBounds.y + feedbackBounds.height);
  await update.evaluate(button => { button.hidden = true; });
  await expect(trigger).toHaveText("Feedback");
  await expect(page.locator(".ui-feedback .ui-feedback-trigger")).toHaveCount(0);
  await trigger.click();
  const panel = page.getByRole("dialog", { name: "Leave your feedback" });
  await expect(panel).toContainText("handled privately");
  const publicReport = panel.getByRole("link", { name: "Report a bug publicly" });
  await expect(publicReport).toHaveAttribute("href", "https://github.com/prometeucorp/prometeu/issues/new");
  await expect(publicReport).toHaveAttribute("target", "_blank");
  await expect(publicReport).toHaveAccessibleDescription(/without sending the description or capture/);
  const description = panel.getByRole("textbox");
  await expect(description).toBeFocused();
  const send = panel.getByRole("button", { name: "Send private feedback" });
  await description.fill("   "); await send.click();
  expect(await attempts(page)).toHaveLength(0);
  await description.fill("Test suggestion <script>do not execute</script>");
  await panel.getByRole("button", { name: "Idea", exact: true }).click();
  await panel.getByLabel("Attach an image", { exact: false }).setInputFiles({ name: "example.png", mimeType: "image/png", buffer: png });
  await expect(panel.getByRole("img")).toBeVisible();
  await send.click();
  await expect(panel.getByRole("alert")).toContainText("Could not send");
  await expect(description).toHaveValue("Test suggestion <script>do not execute</script>");
  await expect(panel.getByRole("img")).toBeVisible();
  await page.keyboard.press("Escape"); await expect(panel).toBeHidden(); await expect(trigger).toBeFocused();
  await trigger.click(); await send.click();
  await expect(panel.getByRole("status")).toContainText("Feedback sent");
  await expect(panel.getByRole("link")).toHaveCount(1);
  const sent = await attempts(page);
  expect(sent).toHaveLength(2);
  expect(sent[1]).toEqual(sent[0]);
  expect(sent[1]).toMatchObject({ kind: "idea", source: "desktop", image: { type: "image/png", data: png.toString("base64") } });
  await expect(description).toHaveValue(""); await expect(panel.getByRole("img")).toBeHidden();
});

test("feedback stays usable above modal dialogs with image drop, capture and a narrow viewport", { tag: "@webkit" }, async ({ page }) => {
  await page.addInitScript(signedIn());
  await page.goto("/");
  const trigger = page.getByRole("button", { name: "Feedback", exact: true });
  await trigger.click();
  await page.evaluate(() => {
    const dialog = document.createElement("dialog"); dialog.id = "host-dialog";
    dialog.innerHTML = '<input aria-label="Host field">';
    document.body.append(dialog); dialog.showModal();
  });
  const panel = page.getByRole("dialog", { name: "Leave your feedback" });
  await panel.getByRole("textbox").fill("Modal failure");
  const transfer = await page.evaluateHandle(([base64]) => {
    const data = new DataTransfer();
    data.items.add(new File([Uint8Array.from(atob(base64), char => char.charCodeAt(0))], "dropped.png", { type: "image/png" }));
    return data;
  }, [png.toString("base64")]);
  await panel.dispatchEvent("dragover", { dataTransfer: transfer });
  await expect(panel).toHaveClass(/ui-feedback-dropping/);
  await panel.dispatchEvent("drop", { dataTransfer: transfer });
  await expect(panel).not.toHaveClass(/ui-feedback-dropping/);
  await expect(panel.getByRole("img")).toBeVisible();
  await panel.getByRole("button", { name: "Remove image" }).click();
  await expect(panel.getByRole("img")).toBeHidden();
  await panel.getByRole("button", { name: "Capture screen" }).click();
  await expect(panel.getByRole("img")).toBeVisible();
  await panel.getByRole("button", { name: "Remove image" }).click();
  await expect(panel.getByRole("img")).toBeHidden();
  await panel.getByLabel("Attach an image", { exact: false }).setInputFiles({ name: "bad.svg", mimeType: "image/svg+xml", buffer: Buffer.from("<svg/>") });
  await expect(panel.getByRole("alert")).toContainText("up to 5 MB");
  await page.setViewportSize({ width: 390, height: 640 });
  expect(await panel.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden(); await expect(page.locator("#host-dialog")).toBeVisible();
  await page.evaluate(() => { const dialog = document.querySelector<HTMLDialogElement>("#host-dialog")!; dialog.close(); dialog.remove(); });
  await trigger.click();
  await expect(panel.getByRole("textbox")).toHaveValue("Modal failure");
});

// Control IPC completion explicitly so attachment races do not depend on timers.
async function deferredImages(page: Page) {
  await expect(page.getByRole("button", { name: "Feedback", exact: true })).toBeVisible();
  await page.evaluate(() => {
    const host = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
      finishImage: (path: string, fail: boolean) => Promise<void>;
    };
    const invoke = host.__TAURI_INTERNALS__.invoke;
    const pending = new Map<string, { resolve: (value: unknown) => void; reject: (cause: unknown) => void }>();
    host.__TAURI_INTERNALS__.invoke = (command, args) => command === "feedback_image"
      ? new Promise((resolve, reject) => pending.set(String(args?.path), { resolve, reject }))
      : invoke(command, args);
    host.finishImage = async (path, fail) => {
      const request = pending.get(path)!;
      pending.delete(path);
      if (fail) request.reject(new Error("Image loading failed"));
      else request.resolve(await invoke("feedback_image", { path }));
    };
  });
  return {
    drop: (path: string) => page.evaluate(path => {
      const box = document.querySelector(".ui-feedback-panel")!.getBoundingClientRect();
      (window as unknown as { mock: { drop: (paths: string[], x: number, y: number) => void } })
        .mock.drop([path], box.x + box.width / 2, box.y + box.height / 2);
    }, path),
    finish: (path: string, fail = false) => page.evaluate(({ path, fail }) =>
      (window as unknown as { finishImage: (path: string, fail: boolean) => Promise<void> }).finishImage(path, fail), { path, fail }),
  };
}

test("feedback waits for the latest native image and ignores older results after submission", async ({ page }) => {
  await page.addInitScript(signedIn());
  await page.goto("/");
  const images = await deferredImages(page);
  await page.getByRole("button", { name: "Feedback", exact: true }).click();
  const panel = page.getByRole("dialog", { name: "Leave your feedback" });
  await panel.getByRole("textbox").fill("Latest image");
  const send = panel.getByRole("button", { name: "Send private feedback" });
  await images.drop("/oldest.png");
  await images.drop("/older.png");
  await images.drop("/latest.png");
  await expect(send).toBeDisabled();
  await expect(panel.locator("form")).toHaveAttribute("aria-busy", "true");
  await panel.locator("form").evaluate(form => (form as HTMLFormElement).requestSubmit());
  expect(await attempts(page)).toHaveLength(0);
  await images.finish("/latest.png");
  await expect(send).toBeEnabled();
  await expect(panel.getByRole("img")).toBeVisible();
  const preview = await panel.getByRole("img").getAttribute("src");
  await images.finish("/oldest.png");
  await expect(panel.getByRole("img")).toHaveAttribute("src", preview!);
  await send.click();
  await expect(panel.getByRole("status")).toContainText("Feedback sent");
  expect(await attempts(page)).toMatchObject([{ image: { type: "image/png" } }]);
  await images.finish("/older.png");
  await expect(panel.getByRole("img")).toBeHidden();
});

test("feedback ignores stale image errors and recovers from the current load failure", async ({ page }) => {
  await page.addInitScript(signedIn());
  await page.goto("/");
  const images = await deferredImages(page);
  await page.getByRole("button", { name: "Feedback", exact: true }).click();
  const panel = page.getByRole("dialog", { name: "Leave your feedback" });
  await panel.getByLabel("Attach an image", { exact: false }).setInputFiles({ name: "kept.png", mimeType: "image/png", buffer: png });
  const preview = await panel.getByRole("img").getAttribute("src");
  await images.drop("/older.png");
  await images.drop("/latest.png");
  await images.finish("/older.png", true);
  await expect(panel.getByRole("alert", { includeHidden: true })).toBeEmpty();
  await expect(panel.getByRole("button", { name: "Send private feedback" })).toBeDisabled();
  await images.finish("/latest.png", true);
  await expect(panel.getByRole("alert")).toHaveText("Image loading failed");
  await expect(panel.getByRole("button", { name: "Send private feedback" })).toBeEnabled();
  await expect(panel.getByRole("img")).toHaveAttribute("src", preview!);
});

test("feedback ignores a pending image after closing and reopening the draft", async ({ page }) => {
  await page.addInitScript(signedIn());
  await page.goto("/");
  const images = await deferredImages(page);
  const trigger = page.getByRole("button", { name: "Feedback", exact: true });
  await trigger.click();
  const panel = page.getByRole("dialog", { name: "Leave your feedback" });
  await panel.getByRole("textbox").fill("Preserved draft");
  await panel.getByLabel("Attach an image", { exact: false }).setInputFiles({ name: "kept.png", mimeType: "image/png", buffer: png });
  const preview = await panel.locator("img").getAttribute("src");
  await images.drop("/pending.png");
  await expect(panel.getByRole("button", { name: "Send private feedback" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await trigger.click();
  await expect(panel.getByRole("button", { name: "Send private feedback" })).toBeEnabled();
  await expect(panel.locator("img")).toHaveAttribute("src", preview!);
  await images.finish("/pending.png");
  await expect(panel.locator("img")).toHaveAttribute("src", preview!);
  await expect(panel.getByRole("textbox")).toHaveValue("Preserved draft");
  await expect(panel.getByRole("alert", { includeHidden: true })).toBeEmpty();
});

function attempts(page: Page) {
  return page.evaluate(() => JSON.parse(localStorage.getItem("mock:feedbackAttempts") ?? "[]") as Record<string, unknown>[]);
}
