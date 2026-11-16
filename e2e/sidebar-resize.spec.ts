import { expect, test } from "./fixtures";

// Pointer capture, flex geometry and keyboard focus require a real browser.
test("sidebar width follows dragging and keyboard input across reloads @webkit", async ({ page }) => {
  await page.goto("/");
  const grip = page.getByRole("separator", { name: "Resize sidebar", exact: true });
  const rail = page.locator(".rail");
  await expect(grip).toHaveAttribute("aria-valuenow", "256");
  const bounds = (await grip.boundingBox())!;
  const x = bounds.x + bounds.width / 2;
  const y = bounds.y + bounds.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 120, y, { steps: 5 });
  await page.mouse.up();
  await expect(rail).toHaveCSS("width", "376px");
  await expect(grip).not.toHaveClass(/dragging/);
  await page.reload();
  await expect(rail).toHaveCSS("width", "376px");
  await grip.focus();
  await page.keyboard.press("ArrowRight");
  await expect(rail).toHaveCSS("width", "392px");
  await page.keyboard.press("ArrowLeft");
  await expect(grip).toHaveAttribute("aria-valuenow", "376");
  await page.locator("#railtoggle").click();
  await expect(grip).toBeHidden();
  await page.locator("#railshow").click();
  await expect(rail).toHaveCSS("width", "376px");
  await page.setViewportSize({ width: 640, height: 720 });
  await expect(rail).toHaveCSS("width", "320px");
  await expect(grip).toHaveAttribute("aria-valuemax", "320");
  await page.setViewportSize({ width: 1280, height: 720 });
  await expect(rail).toHaveCSS("width", "376px");
  await grip.dblclick();
  await expect(rail).toHaveCSS("width", "256px");
  expect(await page.evaluate(() => localStorage.getItem("rail-w"))).toBeNull();
  await grip.focus();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Home");
  await expect(rail).toHaveCSS("width", "256px");
  expect(await page.evaluate(() => localStorage.getItem("rail-w"))).toBeNull();
  // Existing installations have no preference; malformed values also use the default.
  await page.evaluate(() => localStorage.setItem("rail-w", "Infinity"));
  await page.reload();
  await expect(rail).toHaveCSS("width", "256px");
});
