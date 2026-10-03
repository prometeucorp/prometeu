// Opt-in acceptance of the shipped WebView2 composition, native IPC and real WSL.
// Run with Windows Node; no browser mock, dev server, provider account or registry changes.
import { chromium, expect } from "@playwright/test";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, mkdir, mkdtemp } from "node:fs/promises";
import { resolve, join } from "node:path";
import { tmpdir } from "node:os";
import { randomUUID } from "node:crypto";
import { createServer } from "node:net";

if (process.platform !== "win32") throw new Error("Run this native acceptance check with Windows Node.");
const configPath = process.argv[2];
if (!configPath) throw new Error("Usage: node scripts/test-wsl-native.mjs CONFIG.json (executable, distribution, runtime, artifacts)");
const config = JSON.parse(await readFile(configPath, "utf8"));
for (const key of ["executable", "distribution", "runtime", "artifacts"]) {
  if (typeof config[key] !== "string" || !config[key]) throw new Error(`Missing ${key}`);
}
const artifacts = resolve(config.artifacts);
await mkdir(artifacts, { recursive: true });
const execute = promisify(execFile);
const base = `/tmp/pn-${randomUUID()}`;
const target = { distribution: config.distribution, executable: config.runtime, root: `${base}/root`, workdir: `${base}/project ' ação`, codex: `${base}/codex ' ação` };
const failures = [], diagnostics = [], steps = [];
let application, browser, page, attempt = 0, completed = false;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const linux = async (code, ...args) => {
  const result = await execute("wsl.exe", ["--distribution", config.distribution, "--exec", "python3", "-c", code, ...args], { timeout: 20_000, maxBuffer: 1024 * 1024 });
  return result.stdout.trim();
};
function record(step) { steps.push(step); console.log(step); }
async function freePort() {
  const server = createServer(); await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port; await new Promise(resolve => server.close(resolve)); return port;
}
async function launch() {
  const port = await freePort();
  const profile = await mkdtemp(join(tmpdir(), "prometeu-webview-"));
  application = spawn(resolve(config.executable), [], {
    cwd: artifacts,
    env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: profile, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let launchError;
  application.once("error", error => { launchError = error; });
  application.stdout.on("data", bytes => diagnostics.push(bytes.toString()));
  application.stderr.on("data", bytes => diagnostics.push(bytes.toString()));
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (application.exitCode !== null) throw new Error(`Native app exited with ${application.exitCode}`);
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; }
    catch { await wait(200); }
  }
  if (!browser) throw new Error("Native WebView2 did not expose its test debugging endpoint");
  const context = browser.contexts()[0];
  page = context.pages()[0] ?? await context.waitForEvent("page", { timeout: 10_000 });
  page.on("pageerror", error => failures.push(error.message));
  page.on("console", message => { if (message.type() === "error") diagnostics.push(`${message.text()} ${JSON.stringify(message.location())}`); });
  page.on("response", response => { if (response.status() >= 400) diagnostics.push(`${response.status()} ${response.url()}`); });
  await page.waitForURL(/tauri\.localhost\//, { timeout: 15_000 });
  await page.goto("http://tauri.localhost/wsl.html");
  await page.evaluate(() => localStorage.setItem("prometeu:idioma", "en"));
  await page.reload();
  await expect(page.getByRole("button", { name: "Connect", exact: true })).toBeVisible();
  if (!await page.evaluate(() => typeof window.__TAURI_INTERNALS__?.invoke === "function")) throw new Error("Native IPC is unavailable");
  record(`Native window ${++attempt} loaded embedded assets and IPC`);
}
async function connect() {
  for (const [name, value] of [
    ["WSL distribution", target.distribution], ["Runtime executable (Linux path)", target.executable],
    ["Private preview directory (Linux path)", target.root], ["Project directory (Linux path)", target.workdir],
    ["Codex executable (Linux path)", target.codex],
  ]) await page.getByRole("textbox", { name, exact: true }).fill(value);
  await page.getByRole("button", { name: "Connect", exact: true }).click();
  await expect(page.locator(".wsl-setup")).toBeHidden({ timeout: 15_000 });
}
const draft = () => page.getByRole("textbox", { name: "Message", exact: true });
async function send(text) {
  await expect(page.locator(".wsl-status")).toHaveText("Ready for your message");
  await draft().fill(text); await draft().press("Enter");
}
const output = () => page.locator(".wsl-terminal .xterm-rows");
async function shell(command) {
  await expect(page.getByRole("button", { name: "Close terminal", exact: true })).toBeEnabled();
  const input = page.locator(".wsl-terminal .xterm-helper-textarea");
  await input.pressSequentially(command); await input.press("Enter");
}
async function closeWindow() {
  const child = application;
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `$p = Get-Process -Id ${child.pid} -ErrorAction Stop; if (-not $p.CloseMainWindow()) { throw 'Could not close native preview window' }`], { timeout: 15_000 });
  await expect.poll(() => child.exitCode, { timeout: 15_000 }).not.toBeNull();
  await browser?.close().catch(() => {}); browser = undefined; page = undefined; application = undefined;
}
try {
  const fixture = Buffer.from(await readFile(new URL("./fixtures/wsl-native-codex.py", import.meta.url))).toString("base64");
  await linux("import sys,pathlib,base64; p=pathlib.Path(sys.argv[1]); p.mkdir(parents=True); f=pathlib.Path(sys.argv[2]); f.write_bytes(base64.b64decode(sys.argv[3])); f.chmod(0o700)", target.workdir, target.codex, fixture);
  await linux(`import pathlib,subprocess,sys
repo=pathlib.Path(sys.argv[1])
def git(*args): subprocess.run(["git","-C",str(repo),*args],check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
git("init","-b","main")
git("config","commit.gpgsign","false")
git("config","user.name","Native test")
git("config","user.email","native@example.invalid")
(repo/"tracked.txt").write_text("committed\\n")
git("add","tracked.txt")
git("commit","-m","initial")
(repo/"tracked.txt").write_text("keep local changes\\n")
`, target.workdir);
  await launch(); await connect();
  await page.getByRole("button", { name: "Start / resume", exact: true }).click();
  await expect(draft()).toBeEnabled({ timeout: 15_000 });
  await send("Native first message");
  await expect(page.locator(".wsl-transcript .bot")).toContainText("Native first message");
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeDisabled();
  record("Conversation traversed native IPC, wsl.exe and the Linux provider adapter");
  await send("Native approval");
  await page.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(page.locator(".wsl-transcript")).toContainText("Native approval received");
  record("Permission card returned an approval to the same native provider request");
  await page.getByRole("button", { name: "Open terminal", exact: true }).click();
  await shell("stty -echo; export NATIVE_TOKEN=kept; printf 'native-%s\\n' shell");
  await expect(output()).toContainText("native-shell");
  await draft().fill("Unsent native draft");
  await page.getByRole("button", { name: "Disconnect", exact: true }).click();
  await connect(); await expect(draft()).toBeEnabled();
  await expect(draft()).toHaveValue("Unsent native draft");
  await expect(page.getByRole("button", { name: "Start / resume", exact: true })).toBeDisabled();
  await expect(output()).toContainText("native-shell");
  await shell("printf 'reattached=%s\\n' \"$NATIVE_TOKEN\"");
  await expect(output()).toContainText("reattached=kept");
  record("Reconnect preserved the live conversation, terminal environment and unsent draft");
  await send("Continue while window is closed");
  await expect(page.locator(".wsl-transcript")).toContainText("Waiting for native window closure");
  await page.getByRole("button", { name: "Create worktree", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Create worktree" });
  await dialog.getByRole("textbox", { name: "Workspace name" }).fill("Native second workspace");
  await expect(dialog.getByRole("textbox", { name: "Git repository (Linux path)" })).toHaveValue(target.workdir);
  await dialog.getByRole("textbox", { name: "New branch", exact: true }).fill("feature/native-isolation");
  await dialog.getByRole("button", { name: "Create workspace", exact: true }).click();
  await expect(dialog).toBeHidden({ timeout: 25_000 });
  const secondDirectory = await linux("import json,pathlib,sys; c=json.loads(pathlib.Path(sys.argv[1],'workspaces.json').read_text()); print(c['board']['workspaces'][1]['worktree'])", target.root);
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1],'tracked.txt').read_text().strip())", secondDirectory)).toBe("committed");
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1],'tracked.txt').read_text().strip())", target.workdir)).toBe("keep local changes");
  expect(await linux("import subprocess,sys; print(subprocess.check_output(['git','-C',sys.argv[1],'branch','--show-current'],text=True).strip())", target.workdir)).toBe("main");
  expect(await linux("import subprocess,sys; print(subprocess.check_output(['git','-C',sys.argv[1],'branch','--show-current'],text=True).strip())", secondDirectory)).toBe("feature/native-isolation");
  await page.getByRole("button", { name: "Native second workspace", exact: true }).click();
  await expect(page.locator(".wsl-transcript .me")).toHaveCount(0);
  await page.getByRole("button", { name: "Start / resume", exact: true }).click();
  await send("Independent second workspace");
  await expect(page.locator(".wsl-transcript .bot")).toContainText("Independent second workspace");
  await page.getByRole("button", { name: "Open terminal", exact: true }).click();
  await shell("stty -echo; export NATIVE_SECOND=independent; printf 'second-%s\\n' shell");
  await expect(output()).toContainText("second-shell");
  record("Created and selected a Git worktree through native UI while preserving the dirty original checkout and running provider");
  await page.screenshot({ path: join(artifacts, "before-window-close.png") });
  await closeWindow();
  await linux("import sys,pathlib; pathlib.Path(sys.argv[1]).touch()", `${target.workdir}/release-turn`);
  await launch(); await connect();
  await expect(page.getByRole("button", { name: "Native second workspace", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(".wsl-transcript .bot")).toContainText("Independent second workspace");
  await shell("printf 'second-reopened=%s\\n' \"$NATIVE_SECOND\"");
  await expect(output()).toContainText("second-reopened=independent");
  expect(await linux("import pathlib,sys; print(len(pathlib.Path(sys.argv[1]).read_text().splitlines()))", `${secondDirectory}/provider-launches`)).toBe("1");
  await page.locator('[data-workspace="primary"]').click();
  await expect(draft()).toBeEnabled();
  await expect(page.getByRole("button", { name: "Start / resume", exact: true })).toBeDisabled();
  await expect(page.locator(".wsl-transcript")).toContainText("Completed after native window closure");
  await shell("printf 'reopened=%s\\n' \"$NATIVE_TOKEN\"");
  await expect(output()).toContainText("reopened=kept");
  const launches = await linux("import pathlib,sys; print(len(pathlib.Path(sys.argv[1]).read_text().splitlines()))", `${target.workdir}/provider-launches`);
  expect(launches).toBe("1");
  await expect(page.locator(".wsl-transcript .me")).toHaveCount(3);
  await page.screenshot({ path: join(artifacts, "reopened-native-window.png") });
  record("Reopening restored the selected workspace; both providers and shells remained isolated and alive");
  await page.getByRole("button", { name: "Close terminal", exact: true }).click();
  await page.getByRole("button", { name: "Stop", exact: true }).click();
  await page.getByRole("button", { name: "Start / resume", exact: true }).click();
  await expect(draft()).toBeEnabled();
  await send("Native resumed message");
  await expect(page.locator(".wsl-transcript .bot").last()).toContainText("Native resumed message");
  await expect(page.locator(".wsl-status")).toHaveText("Ready for your message");
  expect(await linux("import pathlib,sys; print(len(pathlib.Path(sys.argv[1]).read_text().splitlines()))", `${target.workdir}/provider-launches`)).toBe("2");
  await page.getByRole("button", { name: "Shut down runtime", exact: true }).click();
  await expect(page.getByRole("button", { name: "Connect", exact: true })).toBeVisible();
  await expect.poll(() => linux("import pathlib,sys; print(pathlib.Path(sys.argv[1]).exists())", `${target.root}/resident.sock`)).toBe("False");
  expect(failures).toEqual([]);
  record("Stop/resume and explicit shutdown completed without webview exceptions");
  completed = true;
} catch (error) {
  failures.push(error.stack ?? String(error));
  await page?.screenshot({ path: join(artifacts, "failure.png") }).catch(() => {});
  await writeFile(join(artifacts, "failure.html"), await page?.content().catch(() => "") ?? "");
  process.exitCode = 1;
} finally {
  // Keep failed fixture roots for diagnosis; shutdown only the runtime created by this test.
  if (page) {
    try { await page.getByRole("button", { name: "Shut down runtime", exact: true }).click({ timeout: 1500 }); } catch {}
  }
  if (application) {
    try { await closeWindow(); }
    catch { await execute("taskkill.exe", ["/pid", String(application.pid), "/T", "/F"]).catch(() => {}); }
  }
  // A failed assertion can leave the window detached; reconnect only to this test's root.
  try {
    await linux(`import json,pathlib,subprocess,sys
runtime,root,workdir,codex=sys.argv[1:]
if pathlib.Path(root, "resident.sock").exists():
    p=subprocess.Popen([runtime,"--transport","resident","--root",root,"--workdir",workdir,"--codex",codex],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    output,errors=p.communicate(json.dumps({"v":1,"id":1,"action":{"method":"shutdown"}})+"\\n",timeout=15)
    assert p.returncode == 0, errors
    assert not pathlib.Path(root,"resident.sock").exists(), output
`, target.executable, target.root, target.workdir, target.codex);
  } catch (error) { failures.push(`Cleanup: ${error.message}`); completed = false; process.exitCode = 1; }
  await writeFile(join(artifacts, "result.json"), JSON.stringify({ completed, steps, failures, diagnostics, target, executable: config.executable, date: new Date().toISOString() }, null, 2));
  if (completed) await linux("import shutil,sys; shutil.rmtree(sys.argv[1])", base);
  if (!completed) console.error(failures.join("\n"));
}
