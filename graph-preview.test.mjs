import assert from 'node:assert/strict';
import { chromium, expect } from '@playwright/test';

// Run with: node graph-preview.test.mjs. Opens the standalone file without a server.
const browser = await chromium.launch();
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  const network = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (/^https?:/.test(request.url())) network.push(request.url()); });
  await page.goto(new URL('./graph-preview.html?lang=en', import.meta.url).href);

  const mapTab = page.getByRole('tab', { name: 'Map', exact: true });
  await expect(page.getByRole('tab')).toHaveCount(3);
  await expect(mapTab).toHaveAttribute('aria-selected', 'true');
  await mapTab.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Files', exact: true })).toBeFocused();
  await expect(page.locator('#panel-files')).toBeVisible();
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('#panel-changes')).toBeVisible();
  await page.keyboard.press('End');
  await expect(mapTab).toBeFocused();
  await expect(page.locator('#panel-map')).toBeVisible();

  const nodes = page.locator('.node');
  const count = await nodes.count();
  assert.ok(count > 10, 'The initial map must show the repository overview.');
  assert.ok(await page.locator('.edge').count() > 0);
  const search = page.locator('#search');
  await search.fill('search');
  await expect.poll(() => page.locator('.node.match').count()).toBeGreaterThan(0);
  assert.ok(await page.locator('.node.match').count() < count);
  assert.ok(await page.locator('.edge.lit').count() > 0);
  assert.ok(await page.locator('.node.dim').count() > 0);
  assert.equal(await nodes.count(), count, 'Search must retain the surrounding graph.');
  const first = page.locator('#results button').first();
  const symbol = await first.locator('.result-name > span:last-child').textContent();
  await first.click();
  await expect(page.locator('#detail .detail-name')).toHaveText(symbol);
  await expect(page.locator('#detail .code')).toBeVisible();
  await expect(page.locator('.node.selected')).toHaveCount(1);

  await search.fill('no_such_symbol_93847');
  await expect(page.locator('.node.match')).toHaveCount(0);
  await expect(page.locator('#results button')).toHaveCount(0);
  await expect(page.locator('#no-results')).toBeVisible();
  await page.locator('#clear-search').click();
  await expect(search).toHaveValue('');
  await expect(page.locator('.node.dim')).toHaveCount(0);
  await expect(page.locator('#no-results')).toBeHidden();
  assert.equal(await nodes.count(), count);

  const originalTransform = await page.locator('#scene').getAttribute('transform');
  await page.locator('#zoom-in').click();
  await expect(page.locator('#zoom-level')).toHaveText('120%');
  assert.notEqual(await page.locator('#scene').getAttribute('transform'), originalTransform);
  await page.locator('#zoom-out').click();
  await expect(page.locator('#zoom-level')).toHaveText('100%');
  const resetTransform = await page.locator('#scene').getAttribute('transform');
  await page.locator('#zoom-in').click();
  await page.locator('#fit').click();
  await expect(page.locator('#scene')).toHaveAttribute('transform', resetTransform);
  const width = (await page.locator('.map-panel').boundingBox()).width;
  await page.locator('#expand').click();
  await expect(page.locator('#expand')).toHaveAttribute('aria-pressed', 'true');
  assert.ok((await page.locator('.map-panel').boundingBox()).width > width);
  await page.locator('#expand').click();
  await expect(page.locator('#expand')).toHaveAttribute('aria-pressed', 'false');

  const sync = page.locator('#sync');
  const previousSync = await page.locator('#sync-time').getAttribute('datetime');
  await sync.click();
  await expect(sync).toBeDisabled();
  await expect(sync).toBeEnabled();
  await expect.poll(() => page.locator('#sync-time').getAttribute('datetime')).not.toBe(previousSync);

  await search.fill('search');
  await page.screenshot({ path: '/tmp/prometeu-graph-preview.png', fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.screenshot({ path: '/tmp/prometeu-graph-narrow.png', fullPage: true });
  assert.deepEqual(errors, []);
  assert.deepEqual(network, [], 'The prototype must work offline.');
  console.log('Graph preview: tabs, overview, search connections, selection, clear, zoom, expand, sync, narrow layout and offline use passed.');
} finally {
  await browser.close();
}
