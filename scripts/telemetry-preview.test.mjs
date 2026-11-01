import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

const html = readFileSync(new URL('../telemetry-preview.html', import.meta.url), 'utf8');
const context = {};
runInNewContext(html.match(/<script id="telemetry-model">([\s\S]*?)<\/script>/)[1], context);
const { records, aggregate, filterRecords, knownTokens, coverage, sessionGroups } = context.telemetryPreview;
const week = () => filterRecords(records, '2026-10-26', '2026-11-01');

test('known totals include partial measurements once, never cache or reasoning twice', () => {
  const rows = week();
  const usage = aggregate(rows);
  assert.equal(usage.inputTokens, 2816000);
  assert.equal(usage.outputTokens, 142520);
  assert.equal(knownTokens(usage), 2958520);
  assert.equal(usage.peakContext, 105000);
  assert.equal(usage.contextUsed, null);
  assert.equal(usage.contextWindow, null);
  assert.equal(coverage(rows).total, 10);
  assert.equal(coverage(rows).complete, 7);
  assert.equal(coverage(rows).partial, 2);
  assert.equal(coverage(rows).unknown, 1);
  assert.equal(sessionGroups(rows).length, 6);
  const sources = ['conversation', 'naming', 'plugin-maker'].map(source => knownTokens(aggregate(rows.filter(row => row.source === source))));
  assert.equal(sources.reduce((sum, value) => sum + value, 0), knownTokens(usage));
});

test('inclusive calendar filters exclude global calls when a workspace is selected', () => {
  const desktop = filterRecords(records, '2026-10-26', '2026-11-01', 'desktop');
  assert.equal(desktop.length, 5);
  assert.ok(desktop.some(row => row.source === 'naming'));
  assert.ok(desktop.every(row => row.source !== 'plugin-maker'));
  assert.equal(filterRecords(records, '2026-11-01', '2026-11-01').length, 3);
  assert.equal(filterRecords(records, '', '').length, records.length);
  assert.equal(filterRecords(records, '2027-01-01', '').length, 0);
});

test('unavailable cost and token measurements remain unknown, including empty selections', () => {
  const codex = records.filter(row => row.provider === 'codex');
  assert.equal(aggregate(codex).costUsd, null);
  assert.equal(aggregate(codex).cacheWriteTokens, null);
  assert.equal(aggregate(codex).modelCalls, null);
  assert.equal(knownTokens(aggregate([])), null);
  const groups = sessionGroups(week());
  assert.equal(groups.at(-1).id, 'resume');
  assert.equal(knownTokens(groups.at(-1).usage), null);
  assert.equal(groups.at(-1).coverage.unknown, 1);
  const zero = { measurement: { usage: { ...aggregate([]), inputTokens: 0, outputTokens: 0, costUsd: 0 } } };
  assert.equal(knownTokens(aggregate([zero])), 0);
  assert.equal(aggregate([zero]).costUsd, 0);
});

test('sample provider measurements preserve subsets and do not invent model attribution', () => {
  for (const row of records) {
    const usage = row.measurement.usage;
    for (const value of Object.values(usage)) assert.ok(value === null || (Number.isFinite(value) && value >= 0));
    if (usage.cacheReadTokens !== null) assert.ok(usage.cacheReadTokens <= usage.inputTokens);
    if (usage.cacheWriteTokens !== null) assert.ok(usage.cacheReadTokens + usage.cacheWriteTokens <= usage.inputTokens);
    if (usage.reasoningTokens !== null) assert.ok(usage.reasoningTokens <= usage.outputTokens);
    if (row.provider !== 'claude') {
      assert.equal(usage.costUsd, null);
      assert.equal(row.measurement.usageByModel, null);
    }
    if (row.provider === 'claude') assert.equal(usage.reasoningTokens, null);
  }
});
