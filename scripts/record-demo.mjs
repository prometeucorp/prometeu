import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, writeFile, stat } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { createServer } from "vite";

// Marketing capture only. The real UI consumes synthetic data through its existing mock.
const root = fileURLToPath(new URL("../", import.meta.url));
const output = resolve(root, "test-results/demo");
const ffmpeg = process.env.FFMPEG ?? "ffmpeg";
const fps = 15;
const seconds = 20;
const run = (args) => {
  const result = spawnSync(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", ...args], { encoding: "utf8" });
  if (result.error) throw new Error("Install FFmpeg or set FFMPEG to its executable path.", { cause: result.error });
  assert.equal(result.status, 0, result.stderr);
};
run(["-version"]);
await mkdir(output, { recursive: true });

const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, open: false } });
let browser;
try {
  await server.listen();
  const origin = server.resolvedUrls.local[0];
  browser = await chromium.launch();
  const page = await browser.newPage({
    viewport: { width: 1280, height: 820 }, deviceScaleFactor: 1, locale: "en-US",
    userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36",
  });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    localStorage.setItem("mock:analyticsEnabled", "false");
    localStorage.setItem("mock:githubFeed", "none");
    localStorage.setItem("prometeu:idioma", "en");
    localStorage.setItem("prometeu:mesa", JSON.stringify({
      order: ["demo-claude", "demo-codex"], hidden: [],
      sizes: { "demo-claude": [450, 450], "demo-codex": [450, 450] },
    }));
    // Seed before the first board render; no unrelated sample conversations mount.
    Object.defineProperty(window, "mock", { configurable: true, set(mock) {
      Object.defineProperty(window, "mock", { value: mock, configurable: true });
      const native = window.__TAURI_INTERNALS__;
      const invoke = native.invoke;
      let seeded = false;
      let menuId = 100000;
      native.invoke = async (command, args, options) => {
        // Native menus have no surface in the browser, but Tauri expects resource identities.
        if (command === "plugin:menu|new") return [++menuId, args.options?.id ?? `demo-menu-${menuId}`];
        if (command === "chat_send") {
          mock.line(args.session, { v: 1, at: Date.now(), type: "user.message", content: [{ kind: "text", text: args.text }] });
          return;
        }
        const result = await invoke(command, args, options);
        if (command === "chat_snapshot") {
          result.text = result.text.split("\n").filter(line => line && JSON.parse(line).v === 1).join("\n") + "\n";
        }
        if (command === "load_board" && !seeded) {
          seeded = true;
          result.projects = [{ id: "p2", name: "orbit", path: "/demo/orbit" }];
          result.workspaces = result.workspaces.slice(0, 2);
          result.workspaces.forEach((workspace, i) => {
            const task = i ? "csv-export" : "onboarding";
            Object.assign(workspace, {
              title: i ? "Add CSV export" : "Polish onboarding", project: "p2", repo: "/demo/orbit", repo_name: "orbit",
              branch: `prometeu/${task}`, worktree: `/demo/worktrees/${task}`,
              repos: [{ path: "/demo/orbit", name: "orbit", worktree: `/demo/worktrees/${task}`, base: "main", pr: null }],
              agent: i ? "codex" : "claude", model: i ? "gpt-5.6-sol" : "opus[1m]", effort: "high",
              tabs: [{ id: i ? "demo-codex" : "demo-claude", title: "", status: "rodando", note: null, tokens: 0 }],
            });
            workspace.active = workspace.tabs[0].id;
          });
        }
        return result;
      };
    } });
  });
  await page.route("**/__demo", route => route.fulfill({ contentType: "text/html", body: `<!doctype html>
    <html lang="en"><meta charset="utf-8"><title>Prometeu demo</title>
    <style>
      * { box-sizing: border-box; }
      body { margin: 0; overflow: hidden; background: #eee9e2; color: #29231e;
        font-family: -apple-system, BlinkMacSystemFont, sans-serif; }
      .brand { position: absolute; top: 28px; left: 40px; display: flex; gap: 10px;
        align-items: center; font-size: 18px; font-weight: 650; letter-spacing: -.5px; }
      .brand img { width: 28px; height: 28px; }
      .edition { position: absolute; top: 36px; right: 40px; font-size: 12px;
        letter-spacing: 2px; color: #85786c; }
      h1 { position: absolute; top: 66px; left: 40px; margin: 0;
        font-size: 43px; font-weight: 650; letter-spacing: -1.6px; }
      .subtitle { position: absolute; top: 123px; left: 42px; margin: 0;
        font-size: 19px; color: #786c61; }
      .window { position: absolute; left: 32px; top: 171px; width: 1216px; height: 584px;
        border-radius: 12px; overflow: hidden; border: 1px solid #302923;
        box-shadow: 0 16px 36px #49332132, 0 3px 8px #49332120; background: #141110; }
      iframe { display: block; border: 0; width: 1214px; height: 582px; }
      footer { position: absolute; left: 40px; right: 40px; top: 781px;
        display: flex; align-items: center; justify-content: space-between; }
      .chapters { display: flex; gap: 26px; font-size: 13px; color: #9b8d80; }
      .chapters span { display: flex; gap: 9px; align-items: center; }
      .chapters b { font-size: 11px; font-weight: 500; }
      .chapters .active { color: #392e25; }
      .chapters .active b { color: #e45728; }
      .disclosure { color: #85786c; font-size: 14px; }
      #cursor { position: absolute; z-index: 3; pointer-events: none; width: 24px; height: 30px;
        filter: drop-shadow(0 2px 2px #0008); opacity: 0; }
      #ring { position: absolute; z-index: 2; pointer-events: none; border: 2px solid #ff7a42;
        width: 34px; height: 34px; border-radius: 50%; opacity: 0; }
      #progress { position: absolute; bottom: 0; left: 0; height: 3px; background: #e96936; }
      #fade { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; opacity: 0; }
    </style>
    <div class="brand"><img src="/docs/icon.png" alt="">prometeu</div>
    <div class="edition">A LITTLE LESS JUGGLING.</div>
    <h1>Your agents, side by side.</h1>
    <p class="subtitle">Claude Code and Codex. One desk.</p>
    <div class="window"><iframe name="app" src="/" title="Prometeu"></iframe></div>
    <footer><div class="chapters"><span class="active"><b>01</b> Work in parallel</span>
      <span><b>02</b> Focus in</span><span><b>03</b> Keep moving</span></div>
      <span class="disclosure">Real interface · Simulated agents</span></footer>
    <svg id="cursor" viewBox="0 0 24 30"><path d="M2 2v23l6-6 5 10 5-3-5-9 9-1Z" fill="white" stroke="#24201c" stroke-width="1.5" stroke-linejoin="round"/></svg>
    <div id="ring"></div><div id="progress"></div><img id="fade" alt=""></html>` }));
  await page.goto(`${origin}__demo`);
  const app = page.frame({ name: "app" });
  await expect(app.locator("#deskView")).toBeVisible();
  const tile = id => app.locator(`#tiles .tile[data-tab="demo-${id}"]`);
  await expect(tile("claude")).toBeVisible();
  await expect(tile("codex")).toBeVisible();
  await expect(tile("claude")).toBeInViewport({ ratio: 1 });
  await expect(tile("codex")).toBeInViewport({ ratio: 1 });
  const emit = (tab, events) => app.evaluate(({ tab, events }) => {
    for (const event of events) window.mock.line(`demo-${tab}`, { v: 1, at: Date.now(), ...event });
  }, { tab, events });
  const textBlock = (messageId, index, text) => ({ type: "assistant.block", messageId, index, block: { kind: "text", text } });
  const tool = (messageId, index, id, name, input) => ({ type: "assistant.block", messageId, index, block: { kind: "tool", id, name, input } });
  const done = toolId => ({ type: "tool.completed", toolId, output: "Done.", error: false, background: false });
  const completed = { type: "turn.completed", outcome: "ok", message: "", durationMs: null, costUsd: null };
  const status = async (tab, busy) => {
    await emit(tab, [{ type: "session.state", state: busy ? "busy" : "ready" }]);
    await app.evaluate(async ({ tab, busy }) => {
      const { invoke } = window.__TAURI_INTERNALS__;
      const board = await invoke("load_board");
      const workspace = board.workspaces.find(w => w.active === `demo-${tab}`);
      workspace.tabs[0].status = busy ? "rodando" : "pronta";
      await invoke("set_unread", { id: workspace.id, unread: false });
    }, { tab, busy });
  };
  const leftStream = "- One clear next step\n- Keyboard-friendly forms\n- Helpful empty states";
  const rightStream = "- Respect active filters\n- Escape commas and quotes\n- Keep large exports responsive";
  for (const [tab, prompt, intro, stream] of [
    ["claude", "Make onboarding feel effortless.", "I’m simplifying the first-run experience.\n\n", leftStream],
    ["codex", "Add CSV export for transactions.", "I’m adding export to the toolbar.\n\n", rightStream],
  ]) {
    await emit(tab, [
      { type: "user.message", content: [{ kind: "text", text: prompt }] },
      { type: "assistant.started", messageId: tab },
      { type: "assistant.block.started", messageId: tab, index: 0, block: { kind: "text", text: intro + stream.slice(0, stream.lastIndexOf("\n")) } },
    ]);
    await status(tab, true);
  }
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  await page.clock.install();

  const capture = index => page.screenshot({ path: resolve(output, `frame-${String(index).padStart(4, "0")}.png`) });
  const chapter = async (index, title, subtitle) => page.evaluate(({ index, title, subtitle }) => {
    document.querySelector("h1").textContent = title;
    document.querySelector(".subtitle").textContent = subtitle;
    document.querySelectorAll(".chapters span").forEach((span, i) => span.classList.toggle("active", i === index));
  }, { index, title, subtitle });
  let pointer = { x: 1165, y: 700 };
  let target = pointer;
  let ringAt = -100;
  const aim = async locator => {
    const box = await locator.boundingBox();
    assert(box, "The recorded control must be visible");
    target = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  };
  const click = async (locator, frame) => {
    await locator.click();
    ringAt = frame;
  };
  const followup = "Also add a skip button.";
  for (let frame = 0; frame < fps * seconds; frame++) {
    if (frame >= 8 && frame < 45) {
      for (const [tab, stream] of [["claude", leftStream], ["codex", rightStream]]) {
        const offset = stream.lastIndexOf("\n") + (frame - 8) * 2;
        const delta = stream.slice(offset, offset + 2);
        if (delta) await emit(tab, [{ type: "assistant.delta", messageId: tab, index: 0, kind: "text", delta }]);
      }
    }
    if (frame === 45) {
      await emit("claude", [textBlock("claude", 0, "I’m simplifying the first-run experience.\n\n" + leftStream),
        tool("claude", 1, "onboarding", "Edit", { file_path: "src/onboarding.ts", old_string: "showWelcome()", new_string: "showNextStep()" })]);
      await emit("codex", [textBlock("codex", 0, "I’m adding export to the toolbar.\n\n" + rightStream),
        tool("codex", 1, "export", "Bash", { command: "npm test -- export", description: "Test CSV formatting and filters" })]);
    }
    if (frame === 70) {
      await emit("claude", [done("onboarding"), textBlock("claude", 2, "The new flow is ready. Three steps, with keyboard navigation throughout."), completed]);
      await status("claude", false);
      await aim(tile("claude").locator(".topen"));
    }
    if (frame === 95) {
      await click(tile("claude").locator(".topen"), frame);
      await expect(app.locator("#wsView")).toBeVisible();
      await app.locator("#sidetoggle").click();
      await chapter(1, "Every task gets its own space.", "Separate worktrees. Conversations that stay with the work.");
      await expect(app.locator("#crumb")).toContainText("Polish onboarding");
    }
    if (frame === 115) await aim(app.locator("#chatwrap .composer textarea"));
    if (frame === 130) await click(app.locator("#chatwrap .composer textarea"), frame);
    if (frame >= 135 && frame < 135 + followup.length * 2 && frame % 2 === 1) {
      await app.locator("#chatwrap .composer textarea").fill(followup.slice(0, Math.floor((frame - 135) / 2) + 1));
    }
    if (frame === 180) await aim(app.locator("#chatwrap .composer .send"));
    if (frame === 192) {
      await click(app.locator("#chatwrap .composer .send"), frame);
      await expect(app.locator("#chatwrap .turn.user").last()).toContainText(followup);
      await emit("claude", [{ type: "assistant.started", messageId: "followup" }, textBlock("followup", 0, "I’ll add “Skip for now” and keep setup available from Settings.")]);
      await status("claude", true);
      await chapter(2, "Focus in. Step back.", "Continue any conversation. Keep every task in view.");
      await aim(app.locator("#railbody .navitem", { hasText: "Desk" }));
    }
    if (frame === 218) {
      await click(app.locator("#railbody .navitem", { hasText: "Desk" }), frame);
      await expect(app.locator("#deskView")).toBeVisible();
      await status("claude", true);
      await status("codex", true);
      target = { x: 1190, y: 720 };
    }
    if (frame === 235) {
      await emit("codex", [done("export"), textBlock("codex", 2, "CSV export is ready. Active filters and quoted values are covered."), completed]);
      await status("codex", false);
    }
    if (frame === 255) {
      await emit("claude", [tool("followup", 1, "skip", "Edit", { file_path: "src/onboarding.ts", old_string: "Continue", new_string: "Skip for now" })]);
    }
    pointer = { x: pointer.x + (target.x - pointer.x) * .2, y: pointer.y + (target.y - pointer.y) * .2 };
    await page.evaluate(({ frame, total, pointer, ringAt }) => {
      const cursor = document.querySelector("#cursor");
      cursor.style.left = `${pointer.x - 3}px`;
      cursor.style.top = `${pointer.y - 3}px`;
      cursor.style.opacity = frame >= 70 && frame < 240 ? "1" : "0";
      const ring = document.querySelector("#ring");
      const age = (frame - ringAt) / 9;
      ring.style.left = `${pointer.x - 17}px`;
      ring.style.top = `${pointer.y - 17}px`;
      ring.style.transform = `scale(${.4 + age})`;
      ring.style.opacity = age >= 0 && age < 1 ? String(1 - age) : "0";
      document.querySelector("#progress").style.width = `${100 * frame / (total - 1)}%`;
      document.querySelector("#fade").style.opacity = String(Math.max(0, (frame - total + 7) / 6));
    }, { frame, total: fps * seconds, pointer, ringAt });
    await page.clock.runFor(1000 / fps);
    const png = await capture(frame);
    if (frame === 0) await page.locator("#fade").evaluate((img, src) => { img.src = src; }, `data:image/png;base64,${png.toString("base64")}`);
    if ([0, 90, 110, 175, 240, 280].includes(frame)) console.log(`Captured ${(frame / fps).toFixed(1)}s`);
  }
  assert.deepEqual(errors, [], "The captured interface must run without uncaught errors");
  await expect(tile("claude").locator(".feed")).toContainText(followup);
  await expect(tile("codex").locator(".feed")).toContainText("CSV export is ready.");

  // One shared palette avoids color flicker. Ordered dithering preserves small UI text.
  const input = ["-framerate", String(fps), "-i", resolve(output, "frame-%04d.png"), "-frames:v", String(fps * seconds)];
  run([...input, "-vf", "palettegen=stats_mode=diff", "-frames:v", "1", resolve(output, "palette.png")]);
  const gif = resolve(root, "docs/demo.gif");
  run(["-framerate", String(fps), "-i", resolve(output, "frame-%04d.png"), "-i", resolve(output, "palette.png"),
    "-lavfi", "paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle", "-frames:v", String(fps * seconds), "-loop", "0", gif]);
  run([...input, "-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p", "-movflags", "+faststart", resolve(output, "demo.mp4")]);
  assert((await stat(gif)).size < 10_000_000, "The README GIF must stay under 10 MB");
  await writeFile(resolve(output, "preview.html"), '<!doctype html><html lang="en"><title>Prometeu demo preview</title><body style="margin:40px;background:#eee9e2;text-align:center"><video src="demo.mp4" controls autoplay loop muted style="max-width:100%;width:1000px"></video></body></html>');
  console.log(`Demo: ${gif}\nVideo and frames: ${output}`);
} finally {
  await browser?.close();
  await server.close();
}
