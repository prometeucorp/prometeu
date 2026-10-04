import { expect, test } from "@playwright/test";

// Browser exception: pointer capture, graph rerender focus, and saved coordinates can
// lose a person's visual edits. Pure DAG tests cannot exercise these DOM lifetimes.
test("workflow graph preserves pointer and keyboard edits through save and reload", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto("/");
  await expect(page.locator("#deskView")).toBeVisible();
  await page.locator("#railbody").getByRole("button", { name: "Automations", exact: true }).click();
  const editor = page.locator("#automationsView");
  await editor.getByRole("button", { name: "New workflow", exact: true }).first().click();
  const create = page.getByRole("dialog", { name: "New workflow", exact: true });
  await create.getByLabel("Workflow name", { exact: true }).fill("Durable graph edits");
  await create.getByRole("button", { name: "New workflow", exact: true }).click();
  await expect(create).toBeHidden();
  for (const name of ["Event", "Local condition"]) {
    await editor.getByRole("button", { name: "Add step", exact: true }).click();
    const palette = page.getByRole("dialog", { name: "Add step", exact: true });
    await palette.getByRole("button", { name: new RegExp(`^${name} `) }).click();
    await expect(palette).toBeHidden();
    await editor.locator(".automations-inspector").getByRole("button", { name: "Close", exact: true }).click();
  }
  const event = editor.locator(".automations-node.trigger");
  const condition = editor.locator(".automations-node.condition");
  await event.getByRole("button", { name: "Connect Continue", exact: true }).click();
  await condition.getByRole("button", { name: "Local condition", exact: true }).click();
  await expect(editor.locator(".automations-edges path")).toHaveCount(1);

  const handle = event.getByRole("button", { name: "Event", exact: true });
  const start = await handle.boundingBox();
  expect(start).not.toBeNull();
  await page.mouse.move(start!.x + 100, start!.y + 16);
  await page.mouse.down();
  await page.mouse.move(start!.x + 170, start!.y + 96, { steps: 8 });
  await page.mouse.up();
  await expect(event).toHaveCSS("left", "130px");
  await expect(event).toHaveCSS("top", "140px");
  await handle.focus();
  await handle.press("ArrowRight");
  await expect(handle).toBeFocused();
  await expect(event).toHaveCSS("left", "150px");
  await editor.getByRole("button", { name: "Save revision", exact: true }).click();
  await expect(editor.locator(".automations-notice")).toHaveText("Revision saved");
  await page.reload();
  await expect(page.locator("#deskView")).toBeVisible();
  await page.locator("#railbody").getByRole("button", { name: "Automations", exact: true }).click();
  await expect(editor).toBeVisible();
  await expect(editor.locator(".automations-node.trigger")).toHaveCSS("left", "150px");
  await expect(editor.locator(".automations-node.trigger")).toHaveCSS("top", "140px");
  await expect(editor.locator(".automations-edges path")).toHaveCount(1);
  await editor.getByRole("button", { name: "Simulate", exact: true }).click();
  await expect(editor.locator(".automations-evidence")).toContainText("Local condition · True");
  await expect(editor.locator(".automations-edges path.traversed")).toHaveCount(1);
  await page.setViewportSize({ width: 1024, height: 768 });
  await editor.getByRole("button", { name: "Assistant", exact: true }).click();
  await expect(editor.getByLabel("Describe a workflow or a change…", { exact: true })).toBeVisible();
  await editor.getByLabel("Describe a workflow or a change…", { exact: true }).fill("Preserve this request while switching views");
  await editor.getByRole("button", { name: "Assistant", exact: true }).click();
  await expect(editor.locator(".automations-node.trigger")).toHaveCSS("left", "150px");
  await editor.getByRole("button", { name: "Assistant", exact: true }).click();
  await expect(editor.getByLabel("Describe a workflow or a change…", { exact: true })).toHaveValue("Preserve this request while switching views");
});
