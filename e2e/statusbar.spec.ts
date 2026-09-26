import { expect, test } from "@playwright/test";

test("resource readings preserve the sleep menu anchor and open process row", async ({ page }) => {
  await page.goto("/");
  const awake = page.locator('#status [data-chip="awake"]');
  const resource = page.locator('#status [data-chip="res"]');
  await awake.click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { savedAwake: Element | null }).savedAwake = document.querySelector('#status [data-chip="awake"]');
    (window as unknown as { mock: { machine: (value: unknown) => void } }).mock.machine({
      rss: 1024, cpu: 10, terms: 1, ports: [],
      procs: [{ kind: "app", name: "Prometeu", detail: "", rss: 1024, cpu: 10, hist: [0, 10] }],
    });
  });
  await expect(resource).toContainText("1.0 KB");
  expect(await page.evaluate(() => document.querySelector('#status [data-chip="awake"]') === (window as unknown as { savedAwake: Element }).savedAwake)).toBe(true);
  await expect(page.getByRole("menu")).toBeVisible();

  await page.keyboard.press("Escape");
  await resource.click();
  const panel = page.getByRole("dialog", { name: "Memory & CPU" });
  await expect(panel.locator(".prow")).toHaveCount(1);
  await page.evaluate(() => {
    (window as unknown as { savedRow: Element | null }).savedRow = document.querySelector(".upop.res .prow");
    (window as unknown as { mock: { machine: (value: unknown) => void } }).mock.machine({
      rss: 2048, cpu: 15, terms: 1, ports: [],
      procs: [{ kind: "app", name: "Prometeu", detail: "", rss: 2048, cpu: 15, hist: [0, 15] }],
    });
  });
  await expect(panel.locator(".prss")).toHaveText("2.0 KB");
  expect(await page.evaluate(() => document.querySelector(".upop.res .prow") === (window as unknown as { savedRow: Element }).savedRow)).toBe(true);
});
