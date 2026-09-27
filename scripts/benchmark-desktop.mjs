import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import os from "node:os";
import { dirname } from "node:path";
import { chromium } from "@playwright/test";
import { preview } from "vite";

const samples = Number(process.env.PROMETEU_BENCH_SAMPLES ?? 5);
assert(Number.isInteger(samples) && samples >= 3 && samples <= 30, "Use 3–30 measured samples");
const output = process.env.PROMETEU_BENCH_OUTPUT ?? "benchmark-results/desktop-performance.json";
const version = JSON.parse(await readFile("package.json", "utf8")).version;
async function buildDigest() {
  const hash = createHash("sha256");
  for (const entry of (await readdir("dist", { recursive: true, withFileTypes: true }))
    .filter(entry => entry.isFile()).map(entry => `${entry.parentPath}/${entry.name}`).sort()) {
    hash.update(entry).update("\0").update(await readFile(entry)).update("\0");
  }
  return hash.digest("hex");
}
const buildSha256 = await buildDigest();
const server = await preview({ preview: { host: "127.0.0.1", port: 0, open: false } });
let browser;
const results = [];
try {
  browser = await chromium.launch();
  for (let sample = -1; sample < samples; sample++) {
    const context = await browser.newContext({ viewport: { width: 1600, height: 1000 }, locale: "en-US" });
    try {
      // External services cannot add network noise or receive benchmark data.
      const origin = new URL(server.resolvedUrls.local[0]).origin;
      await context.route("**/*", route => new URL(route.request().url()).origin === origin
        ? route.continue() : route.abort());
      await context.addInitScript(({ version }) => {
        localStorage.setItem("prometeu:idioma", "en");
        localStorage.setItem("prometeu:novidades", version);
        const tabs = ["t1", "t2", "t3", "t9"];
        localStorage.setItem("prometeu:mesa", JSON.stringify({ order: tabs,
          sizes: Object.fromEntries(tabs.map(tab => [tab, [550, 350]])),
          hidden: ["t4", "t5", "t6", "t7", "t8"],
        }));
      }, { version });
      const page = await context.newPage();
      const errors = [];
      page.on("pageerror", error => errors.push(error.stack ?? error.message));
      await page.goto(origin, { waitUntil: "domcontentloaded" });
      await page.locator('#tiles .tile[data-tab="t1"]').waitFor();
      const startupMs = await page.evaluate(async () => {
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        return performance.now();
      });
      const streams = await page.evaluate(async () => {
        const tabs = ["t1", "t2", "t3", "t9"];
        for (const tab of tabs) {
          const tile = document.querySelector(`#tiles .tile[data-tab="${tab}"]`);
          const rect = tile?.getBoundingClientRect();
          if (!tile || tile.hidden || !rect || rect.width === 0 || rect.top < 0 || rect.bottom > innerHeight) {
            throw new Error(`Conversation ${tab} is not fully visible`);
          }
        }
        const emit = (tab, type, fields) => window.mock.line(tab, { v: 1, type, at: 1, ...fields });
        const start = performance.now();
        let last = start;
        let maxFrameGapMs = 0;
        for (const tab of tabs) {
          emit(tab, "assistant.started", { messageId: `bench-${tab}` });
          emit(tab, "assistant.block.started", { messageId: `bench-${tab}`, index: 0, block: { kind: "text", text: "" } });
        }
        for (let batch = 0; batch < 20; batch++) {
          for (let chunk = 0; chunk < 5; chunk++) for (const tab of tabs) {
            emit(tab, "assistant.delta", { messageId: `bench-${tab}`, index: 0, kind: "text", delta: "Measured streaming text. " });
          }
          await new Promise(requestAnimationFrame);
          const now = performance.now();
          maxFrameGapMs = Math.max(maxFrameGapMs, now - last);
          last = now;
        }
        for (const tab of tabs) {
          emit(tab, "assistant.block", { messageId: `bench-${tab}`, index: 0, block: { kind: "text", text: "Benchmark complete" } });
          emit(tab, "turn.completed", { outcome: "ok", message: "", durationMs: 1, costUsd: null });
        }
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        for (const tab of tabs) {
          if (!document.querySelector(`#tiles .tile[data-tab="${tab}"]`).textContent.includes("Benchmark complete")) {
            throw new Error(`Stream did not render for ${tab}`);
          }
        }
        return { durationMs: performance.now() - start, maxFrameGapMs };
      });
      assert.deepEqual(errors, [], "The benchmark must not hide rendering failures");
      if (sample >= 0) results.push({ startupMs, ...streams });
    } finally {
      await context.close();
    }
  }
  const summary = Object.fromEntries(Object.keys(results[0]).map(key => {
    const sorted = results.map(result => result[key]).sort((a, b) => a - b);
    const middle = Math.floor(sorted.length / 2);
    const median = sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
    return [key, { median, min: sorted[0], max: sorted.at(-1) }];
  }));
  assert.equal(await buildDigest(), buildSha256, "The production build changed during measurement");
  const report = {
    schema: 1, measuredAt: new Date().toISOString(),
    buildSha256,
    revision: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    dirty: !!execFileSync("git", ["status", "--porcelain"], { encoding: "utf8" }).trim(),
    environment: { platform: os.platform(), release: os.release(), arch: os.arch(), cpu: os.cpus()[0]?.model,
      node: process.version, browser: browser.version() },
    workload: { backend: "browser mock", build: "production", viewport: [1600, 1000], tileSize: [550, 350], warmups: 1,
      freshContexts: true, streams: 4, deltasPerStream: 100, batches: 20 },
    samples: results, summary,
  };
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ output, summary }, null, 2));
} finally {
  await browser?.close();
  await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
