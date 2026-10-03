// Opt-in acceptance of the shipped WebView2 composition, native IPC and real WSL.
// Run with Windows Node. The default journey uses a synthetic provider; optional
// config.codex runs two live inference turns with an already authenticated WSL CLI.
import { chromium, expect } from "@playwright/test";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { randomUUID } from "node:crypto";
import { createServer } from "node:net";

if (process.platform !== "win32") throw new Error("Run this native acceptance check with Windows Node.");
const configPath = process.argv[2];
if (!configPath) throw new Error("Usage: node scripts/test-windows-application.mjs CONFIG.json (executable, distribution, runtime, artifacts)");
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
let oauthServer;
let application, browser, page, windowsProject, sharedProfile, attempt = 0, completed = false;
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
  const profile = config.bootstrap ? (sharedProfile ??= await mkdtemp(join(tmpdir(), "prometeu-bootstrap-"))) : await mkdtemp(join(tmpdir(), "prometeu-webview-"));
  application = spawn(resolve(config.executable), [], {
    cwd: artifacts,
    env: { ...process.env, PROMETEU_WINDOWS_RUNTIME_ROOT: config.bootstrap ? target.root : `${base}/bootstrap`, WEBVIEW2_USER_DATA_FOLDER: profile, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let launchError;
  application.once("error", error => { launchError = error; });
  application.stdout.on("data", bytes => diagnostics.push(bytes.toString()));
  application.stderr.on("data", bytes => diagnostics.push(bytes.toString()));
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (application.exitCode !== null) throw new Error(`Native app exited with ${application.exitCode}`);
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 1000 }); break; }
    catch { await wait(200); }
  }
  if (!browser) throw new Error("Native WebView2 did not expose its test debugging endpoint");
  const context = browser.contexts()[0];
  page = context.pages()[0] ?? await context.waitForEvent("page", { timeout: 10_000 });
  await page.addLocatorHandler(page.locator("#veil .news"), async news => {
    await news.getByRole("button", { name: "Close", exact: true }).click();
  });
  page.on("pageerror", error => failures.push(error.message));
  page.on("console", message => { if (message.type() === "error") diagnostics.push(`${message.text()} ${JSON.stringify(message.location())}`); });
  page.on("response", response => { if (response.status() >= 400) diagnostics.push(`${response.status()} ${response.url()}`); });
  await page.waitForURL(/tauri\.localhost\/(?:index\.html)?$/, { timeout: 15_000 });
  const reload = await page.evaluate(() => { const changed = localStorage.getItem("prometeu:idioma") !== "en"; localStorage.setItem("prometeu:idioma", "en"); return changed; });
  if (reload) await page.reload();
  await expect(page.locator("#deskView")).toBeVisible({ timeout: 60000 });
  await expect(page.getByRole("dialog", { name: /Set up WSL|Connect/ })).toHaveCount(0);
  if (!await page.evaluate(() => typeof window.__TAURI_INTERNALS__?.invoke === "function")) throw new Error("Native IPC is unavailable");
  record(`Native window ${++attempt} loaded embedded assets and IPC`);
}
async function connect() {
  if (config.bootstrap) {
    if (attempt > 1) return;
    const prepared = await page.evaluate(() => JSON.parse(localStorage.getItem("prometeu:windows-target")));
    const actualDefault = await execute("wsl.exe", ["--exec", "/bin/sh", "-c", 'printf "%s" "$WSL_DISTRO_NAME"']);
    expect(prepared.distribution).toBe(actualDefault.stdout);
    expect(prepared.root).toBe(target.root);
    expect(prepared.codex).toBe(config.codex);
    if (config.existingWorkdir) expect(prepared.workdir).toBe(config.existingWorkdir);
    expect(prepared.executable).toContain("/.local/share/prometeu-windows/runtime/");
    const board = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }));
    expect(board.projects).toHaveLength(0);
    expect(board.workspaces).toHaveLength(0);
    const project = target.workdir;
    Object.assign(target, prepared);
    record("Default WSL opened the original empty desktop without a connection, distribution or project setup screen");
    await page.screenshot({ path: join(artifacts, "default-empty-desktop.png") });
    await page.getByRole("button", { name: "Register a repository", exact: true }).click();
    await page.getByRole("button", { name: "Add local folder", exact: true }).click();
    await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./fixtures/windows-select-directory.ps1", import.meta.url)), "-AppId", String(application.pid), "-Directory", join(String.raw`\\wsl.localhost`, target.distribution, project)], { timeout: 30000 });
    await expect(page.getByRole("dialog", { name: "Projects", exact: true })).toBeHidden({ timeout: 15000 });
    await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
    await expect(page.locator("#veil .sheet")).toBeVisible();
    await expect(page.locator("#d-wt")).toBeDisabled();
    await expect(page.locator("#d-wt")).not.toBeChecked();
    await page.locator("#d-go").click();
    await expect(page.locator("#veil")).toBeHidden();
    await expect(page.locator("#chatwrap")).toBeVisible({ timeout: 30000 });
    await page.locator("#railbody").getByRole("button", { name: "Desk", exact: true }).click();
    await expect(page.locator("#deskView textarea").first()).toBeVisible();
    await expect.poll(() => page.evaluate(async () => {
      const board = await window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null });
      return !board.workspaces[0].preparing && board.workspaces[0].tabs[0].status === "pronta";
    }), { timeout: 30000 }).toBe(true);
    record("Original project picker and launcher created the first conversation in WSL");
    return;
  }
  // Fixtures select the existing diagnostic transport through IPC; no product setup UI.
  await page.evaluate(async target => {
    // Busy shutdown is rejected before effects. Wait only for that explicit refusal;
    // a transport failure or uncertain outcome must not repeat the shutdown request.
    const deadline = Date.now() + 30000;
    while (true) {
      try { await window.__TAURI_INTERNALS__.invoke("wsl_shutdown"); break; }
      catch (error) {
        if (!String(error).includes('"code":"err.windows.operationBusy"') || Date.now() >= deadline) throw error;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
    }
    await window.__TAURI_INTERNALS__.invoke("wsl_disconnect");
    await window.__TAURI_INTERNALS__.invoke("wsl_connect", { target, connection: "native-fixture" });
  }, target);
  await page.reload();
  await expect(page.locator("#deskView textarea").first()).toBeVisible({ timeout: 30000 });
}
async function closeWindow() {
  const child = application;
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `$p = Get-Process -Id ${child.pid} -ErrorAction Stop; if (-not $p.CloseMainWindow()) { throw 'Could not close native preview window' }`], { timeout: 15_000 });
  await expect.poll(() => child.exitCode, { timeout: 15_000 }).not.toBeNull();
  await browser?.close().catch(() => {}); browser = undefined; page = undefined; application = undefined;
}
try {
  if (config.codex) {
    if (typeof config.codex !== "string" || !config.codex.startsWith("/")) throw new Error("Live Codex must be an absolute Linux executable path");
    target.codex = config.codex;
    await linux("import pathlib,sys; pathlib.Path(sys.argv[1]).mkdir(parents=True)", target.workdir);
    if (config.existingWorkdir) {
      if (!config.bootstrap || !config.existingWorkdir.startsWith("/")) throw new Error("existingWorkdir requires bootstrap and an absolute Linux directory");
      await linux("import pathlib,json,sys; p=pathlib.Path(sys.argv[1]); p.mkdir(parents=True,mode=0o700); (p/'runtime.json').write_text(json.dumps(dict(v=1,workdir=sys.argv[2],provider='codex',provider_session=None)))", target.root, config.existingWorkdir);
      record("Seeded an existing runtime identity before the first Windows profile was created");
    }
    await launch(); await connect();
    await page.evaluate(() => {
      window.nativeNotices = [];
      new MutationObserver(() => window.nativeNotices.push(document.querySelector("#msg").textContent))
        .observe(document.querySelector("#msg"), { childList: true, subtree: true, characterData: true });
    });
    const marker = `NATIVE_LIVE_${randomUUID().replaceAll("-", "")}`;
    const composer = page.locator("#deskView textarea").first();
    await composer.fill(`Reply with exactly ${marker}. Do not use tools or access files.`);
    await expect(composer).toHaveValue(`Reply with exactly ${marker}. Do not use tools or access files.`);
    await page.locator("#deskView .send").first().click();
    await expect(page.locator("#deskView .bot").last()).toContainText(marker, { timeout: 120_000 });
    await expect.poll(() => page.evaluate(async () => {
      const board = await window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null });
      return board.workspaces[0].tabs[0].status;
    }), { timeout: 20_000 }).toBe("pronta");
    record("Installed authenticated Codex answered in the original native Windows ChatView through WSL");
    await closeWindow();
    if (config.installer) {
      await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./fixtures/windows-install.ps1", import.meta.url)), "-Installer", config.installer, "-Executable", config.executable], { timeout: 150_000 });
      record("Reinstalled the Windows package while retaining the WSL conversation");
    }
    await launch(); await connect();
    await expect(page.locator("#deskView .bot").last()).toContainText(marker);
    const restored = page.locator("#deskView textarea").first();
    await restored.fill("What exact marker did you reply with in your previous message? Reply only with that marker. Do not use tools or access files.");
    await restored.press("Enter");
    await expect(page.locator("#deskView .bot").filter({ hasText: marker })).toHaveCount(2, { timeout: 120_000 });
    await expect.poll(() => page.evaluate(async () => {
      const board = await window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null });
      return board.workspaces[0].tabs[0].status;
    }), { timeout: 20_000 }).toBe("pronta");
    await page.screenshot({ path: join(artifacts, "shared-live-conversation.png") });
    record("Reopening the native window preserved the real conversation and its next reply recalled the previous marker");
    expect(failures).toEqual([]);
    completed = true;
  } else {
  const fixture = Buffer.from(await readFile(new URL("./fixtures/wsl-native-codex.py", import.meta.url))).toString("base64");
  await linux("import sys,pathlib,base64; p=pathlib.Path(sys.argv[1]); p.mkdir(parents=True); f=pathlib.Path(sys.argv[2]); f.write_bytes(base64.b64decode(sys.argv[3])); f.chmod(0o700)", target.workdir, target.codex, fixture);
  await linux("import pathlib,sys; pathlib.Path(sys.argv[1], 'notes.txt').write_text('Original file')", target.workdir);
  const png = (await readFile(new URL("../src-tauri/icons/32x32.png", import.meta.url))).toString("base64");
  await linux(String.raw`import base64,pathlib,sys
p=pathlib.Path(sys.argv[1]); image=p/'image.png'; image.write_bytes(base64.b64decode(sys.argv[2]))
with image.open('ab') as f: f.truncate(9*1024*1024)
(p/'table.csv').write_text('name,value\nNative,42\n')
content=b'BT /F1 16 Tf 20 100 Td (Native PDF) Tj ET'
objects=[b'<</Type/Catalog/Pages 2 0 R>>', b'<</Type/Pages/Kids[3 0 R]/Count 1>>', b'<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Resources<</Font<</F1 4 0 R>>>>/Contents 5 0 R>>', b'<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>', b'<</Length '+str(len(content)).encode()+b'>>\nstream\n'+content+b'\nendstream']
pdf=b'%PDF-1.4\n'; offsets=[0]
for i,obj in enumerate(objects,1):
    offsets.append(len(pdf)); pdf+=str(i).encode()+b' 0 obj\n'+obj+b'\nendobj\n'
xref=len(pdf); pdf+=b'xref\n0 6\n0000000000 65535 f \n'
for offset in offsets[1:]: pdf+=f'{offset:010d} 00000 n \n'.encode()
pdf+=f'trailer\n<</Size 6/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n'.encode()
(p/'document.pdf').write_bytes(pdf)
`, target.workdir, png);
  await linux(String.raw`import pathlib,sys
p=pathlib.Path(sys.argv[1]); (p/'.prometeu').mkdir()
(p/'.env').write_text('NATIVE_COPY=kept')
(p/'.gitignore').write_text('.env\nsetup-release\nsetup-done\nprovider-launches\n')
(p/'.prometeu/settings.toml').write_text('''
[scripts]
setup = 'while [ ! -f "$PROMETEU_ROOT_PATH/setup-release" ]; do sleep 0.05; done; test -f .env && touch setup-done && echo NATIVE_SETUP_DONE'
run = 'printf "NATIVE_RUN port=%s path=%s\\n" "$PROMETEU_PORT" "$PROMETEU_WORKSPACE_PATH"; sleep 60'
''')
`, target.workdir);
  await linux(`import subprocess,sys
root=sys.argv[1]
def git(*args):
    subprocess.run(["git","-c","commit.gpgsign=false","-C",root,*args],check=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
git("init","-b","main")
git("config","user.name","Native Test")
git("config","user.email","native@example.test")
git("config","commit.gpgsign","false")
git("config","core.hooksPath","no-hooks")
git("add","notes.txt",".gitignore",".prometeu/settings.toml")
git("commit","-m","initial")
`, target.workdir);
  await launch(); await connect();
  const imported = `${base}/selected ' project`;
  await linux("import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.mkdir(); (p/'imported.txt').write_text('Selected through Windows')", imported);
  await page.getByRole("button", { name: "Register a repository", exact: true }).click();
  await page.getByRole("button", { name: "Add local folder", exact: true }).click();
  const unc = join(String.raw`\\wsl.localhost`, target.distribution, imported);
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./fixtures/windows-select-directory.ps1", import.meta.url)), "-AppId", String(application.pid), "-Directory", unc], { timeout: 30_000 });
  await expect(page.getByRole("dialog", { name: "Projects", exact: true })).toBeHidden({ timeout: 15000 });
  const registered = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }));
  expect(registered.projects.map(project => project.path)).toContain(imported);
  expect(registered.workspaces).toHaveLength(1);
  record("Existing Add local folder used the real Windows directory picker and registered its WSL selection without creating a conversation");
  const oauthRoot = `${base}/oauth`;
  const oauthCode = await readFile(new URL("./fixtures/mcp-oauth.py", import.meta.url), "utf8");
  oauthServer = spawn("wsl.exe", ["--distribution", config.distribution, "--exec", "python3", "-u", "-c", oauthCode, oauthRoot], { stdio: ["ignore", "pipe", "pipe"] });
  oauthServer.stderr.on("data", bytes => diagnostics.push(bytes.toString()));
  const oauthUrl = await new Promise((resolve, reject) => {
    let output = "";
    const timeout = setTimeout(() => reject(new Error("OAuth fixture did not start")), 15000);
    oauthServer.once("error", reject);
    oauthServer.stdout.on("data", bytes => { output += bytes; if (output.includes("\n")) { clearTimeout(timeout); resolve(output.split("\n")[0].trim()); } });
  });
  await page.evaluate(url => window.__TAURI_INTERNALS__.invoke("application_request", { command: "mcp_save", args: { server: { id: "native-oauth", note: "Isolated OAuth fixture", config: { url: `${url}/mcp` } } } }), oauthUrl);
  await page.reload();
  await expect(page.locator("#deskView")).toBeVisible({ timeout: 30000 });
  const librarySource = `${base}/native-library`;
  const writeLibrary = async description => linux(String.raw`import pathlib,json,subprocess,sys
p=pathlib.Path(sys.argv[1]); (p/'.claude-plugin').mkdir(parents=True,exist_ok=True)
(p/'.claude-plugin/plugin.json').write_text(json.dumps({'name':'native-library','version':'1.0.0','description':sys.argv[2]}))
def git(*args): subprocess.run(['git','-c','user.name=Test','-c','user.email=test@example.test','-c','commit.gpgsign=false','-c','core.hooksPath=/dev/null',*args],cwd=p,check=True,capture_output=True)
if not (p/'.git').exists(): git('init','-q')
git('add','.'); git('commit','-qm',sys.argv[2])`, librarySource, description);
  await writeLibrary("Native imported package");
  await page.locator("#settings").click();
  await page.locator(".setnavitem").getByText("Resources", { exact: true }).click();
  const addResource = async name => {
    await page.getByRole("button", { name: "Add resource", exact: true }).click();
    await page.getByRole("menuitem", { name, exact: true }).click();
  };
  await addResource("Install plugin");
  await page.locator("#veil").getByLabel("Repository address", { exact: true }).fill(`file://${librarySource}`);
  await page.locator("#veil").getByLabel("Repository address", { exact: true }).press("Tab");
  await expect(page.locator("#veil")).toBeHidden({ timeout: 25000 });
  const libraryRow = page.locator(".resource-row", { has: page.locator("b", { hasText: /^native-library$/ }) });
  await expect(libraryRow).toBeVisible();
  const libraryAction = async name => {
    await libraryRow.getByRole("button", { name: "More options · native-library", exact: true }).click();
    await page.getByRole("menuitem", { name, exact: true }).click();
  };
  await writeLibrary("Native updated package");
  await libraryAction("Update");
  await expect.poll(() => page.evaluate(async () => (await window.__TAURI_INTERNALS__.invoke("application_request", { command: "plugin_hub", args: null }))[0]?.note)).toBe("Native updated package");
  await libraryAction("Remove");
  await page.getByRole("dialog").getByRole("button", { name: "Remove and delete the folder", exact: true }).click();
  await expect(libraryRow).toHaveCount(0);
  await addResource("Point to a folder");
  await page.locator("#veil").getByLabel("Where it is", { exact: true }).fill(join(String.raw`\\wsl.localhost`, target.distribution, librarySource));
  await page.locator("#veil").getByLabel("Where it is", { exact: true }).press("Tab");
  await expect(page.locator("#veil").getByLabel("Name", { exact: true })).toHaveValue("native-library");
  await page.locator("#veil").getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator("#veil")).toBeHidden();
  await expect(libraryRow).toBeVisible();
  await libraryAction("Remove");
  await expect(libraryRow).toHaveCount(0);
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1],'.claude-plugin/plugin.json').exists())", librarySource)).toBe("True");
  record("Original Resources dialogs imported and updated a Git plugin, removed its owned clone, translated a Windows folder source and preserved that user-owned folder on removal");
  const oauthRow = page.locator(".mcp-server", { has: page.locator("b", { hasText: /^native-oauth$/ }) });
  const oauthAction = async name => {
    await oauthRow.getByRole("button", { name: "More options · native-oauth", exact: true }).click();
    await page.getByRole("menuitem", { name, exact: true }).click();
  };
  await oauthAction("Test connection");
  await expect(oauthRow.getByRole("status")).toHaveText("Authentication required on this device", { timeout: 15000 });
  await linux("import pathlib,sys; pathlib.Path(sys.argv[1],'hold-consent').touch()", oauthRoot);
  await oauthAction("Authenticate");
  await expect(oauthRow).toHaveAttribute("aria-busy", "true");
  const duringConsent = await page.evaluate(() => Promise.race([
    window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }),
    new Promise((_, reject) => setTimeout(() => reject(new Error("Browser consent blocked application commands")), 3000)),
  ]));
  expect(duringConsent.workspaces).toHaveLength(1);
  await linux("import pathlib,sys; pathlib.Path(sys.argv[1],'hold-consent').unlink()", oauthRoot);
  await expect(oauthRow.getByRole("status")).toHaveText("signed in", { timeout: 60000 });
  const oauthStats = JSON.parse(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1],'stats.json').read_text())", oauthRoot));
  expect(oauthStats.registrations).toBe(1);
  expect(oauthStats.exchanges).toBe(1);
  expect(oauthStats.refreshes).toBe(1);
  expect(oauthStats.probes).toBeGreaterThan(0);
  await oauthAction("Sign out");
  await expect(oauthRow.getByRole("status")).toHaveText("Authentication required on this device", { timeout: 15000 });
  expect(await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "mcp_logins", args: null }))).toEqual([]);
  await oauthAction("Remove");
  await expect(oauthRow).toHaveCount(0);
  record("Original MCP actions opened native Windows browser consent, received its loopback callback, refreshed private WSL credentials and signed out while other application commands remained responsive");
  await page.locator("#railbody").getByRole("button", { name: "Desk", exact: true }).click();
  await expect(page.locator("#railbody")).toContainText("project");
  await page.evaluate(async () => {
    const source = document.querySelector('.cloud-account image')?.getAttribute('href');
    if (!source) throw new Error("Missing brand asset");
    const image = new Image(); image.src = source;
    await image.decode();
  });
  const composer = page.locator("#deskView textarea").first();
  await expect(composer).toBeVisible();
  await page.evaluate(async () => {
    await window.__TAURI_INTERNALS__.invoke("application_request", { command: "mcp_save", args: { server: {
      id: "native-check", note: "Native configuration probe",
      config: { command: "/usr/bin/python3", args: ["-c", "import os; print(os.environ['NATIVE_TOOL_RESULT'])"], env: { NATIVE_TOOL_RESULT: "Native tool configuration received" } },
    } } });
  });
  // The fixture writes through native IPC instead of the hub editor, so reload its cached catalog.
  await page.reload();
  await expect(composer).toBeVisible({ timeout: 30000 });
  await page.locator("#deskView .mcpbtn").first().click();
  const nativeTool = page.locator(".menu .mrow").filter({ hasText: "native-check" }).first();
  await nativeTool.click();
  await expect(nativeTool).toHaveAttribute("aria-checked", "true");
  await page.keyboard.press("Escape");
  await page.evaluate(async () => {
    const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke("application_request", { command, args });
    const workspace = (await invoke("load_board", null)).workspaces[0];
    await invoke("set_tab_choice", { id: workspace.id, tab: workspace.tabs[0].id,
      choice: { agent: "codex", model: "native-fixture", effort: "high" } });
  });
  await composer.fill("Native shared interface message");
  await composer.press("Enter");
  await expect(page.locator("#deskView .bot")).toContainText("Native shared interface message", { timeout: 20000 });
  await page.screenshot({ path: join(artifacts, "shared-desktop.png") });
  record("Existing desktop shell, desk and ChatView sent a message through native Windows IPC and WSL");
  await composer.fill("Check native tools");
  await composer.press("Enter");
  await expect(page.locator("#deskView .bot").last()).toContainText("Native tool configuration received", { timeout: 20000 });
  const launchesBeforeClose = await linux("import pathlib,sys; print(len(pathlib.Path(sys.argv[1], 'provider-launches').read_text().splitlines()))", target.workdir);
  record("The original tool picker selected a local MCP definition; the WSL fixture provider executed its configured command with the private environment");
  const attachment = join(artifacts, "attachment ' ação.txt");
  await writeFile(attachment, "Readable from WSL");
  await page.locator("#deskView").getByRole("button", { name: "Point the agent at a file", exact: true }).first().click();
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./fixtures/windows-select-directory.ps1", import.meta.url)), "-AppId", String(application.pid), "-Directory", attachment, "-Title", "Files to point the agent at", "-File"], { timeout: 30000 });
  await expect(page.locator("#deskView .cfiles").first()).toContainText("attachment ' ação.txt");
  await composer.fill("Read native attachment");
  await composer.press("Enter");
  await expect(page.locator("#deskView .bot").last()).toContainText("Native attachment: Readable from WSL", { timeout: 20000 });
  record("The original attachment picker sent a Windows file with spaces and Unicode to the WSL provider, which read its contents");
  const withClipboard = async (kind, path, check) => {
    const ready = join(artifacts, `clipboard-${randomUUID()}.ready`), done = `${ready}.done`;
    const fixture = execute("powershell.exe", ["-STA", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./fixtures/windows-clipboard.ps1", import.meta.url)), `-${kind}`, path, "-Ready", ready, "-Done", done], { timeout: 40000 });
    void fixture.catch(() => {});
    let failure;
    try {
      await expect.poll(() => readFile(ready, "utf8").catch(() => "")).toBe("ready");
      await check();
    } catch (error) {
      failure = error;
    } finally {
      await writeFile(done, "done");
      try { await fixture; } catch (error) { failure ??= error; }
      await Promise.all([rm(ready, { force: true }), rm(done, { force: true })]);
    }
    if (failure) throw failure;
  };
  await withClipboard("Image", fileURLToPath(new URL("../src-tauri/icons/32x32.png", import.meta.url)), async () => {
    await composer.focus();
    await composer.press("Control+v");
    await expect(page.locator("#deskView .cfiles").first()).toContainText("pasted.png");
    const files = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "paste_files", args: null }));
    expect(await linux("import pathlib,sys; p=pathlib.Path(sys.argv[1]); print(p.read_bytes()[:8].hex())", files[0])).toBe("89504e470d0a1a0a");
    await composer.press("Enter");
    await expect(page.locator("#deskView .bot").last()).toContainText("pasted.png");
    await expect(page.locator("#deskView .cfiles").first()).toBeHidden();
  });
  await withClipboard("File", attachment, async () => {
    await composer.focus();
    await composer.press("Control+v");
    await expect(page.locator("#deskView .cfiles").first()).toContainText("attachment ' ação.txt");
    const files = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "paste_files", args: null }));
    expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1]).read_text())", files[0])).toBe("Readable from WSL");
    await composer.fill("Read native attachment");
    await composer.press("Enter");
    await expect(page.locator("#deskView .bot").last()).toContainText("Native attachment: Readable from WSL");
  });
  record("Native clipboard paste reused attachment chips; PNG data and copied-file paths remained readable inside WSL");
  await page.locator("#deskView .tile .topen").first().click();
  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "notes.txt" }).click();
  await expect(page.locator("#vtext")).toHaveValue("Original file");
  await page.locator("#vtext").fill("Edited from the native Windows interface");
  await page.locator("#vsave").click();
  await expect.poll(() => linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", target.workdir)).toBe("Edited from the native Windows interface");
  record("Existing file tree and editor saved the WSL project file");
  await page.locator("#tree .treerow", { hasText: "image.png" }).click();
  await expect(page.locator("#vfile.image img")).toBeVisible({ timeout: 20000 });
  await expect.poll(() => page.locator("#vfile.image img").evaluate(image => image.naturalWidth)).toBe(32);
  await page.locator("#tree .treerow", { hasText: "table.csv" }).click();
  await expect(page.locator("#vfile.csv table")).toContainText("Native42");
  await page.locator("#tree .treerow", { hasText: "document.pdf" }).click();
  await expect(page.locator("#vfile.pdf iframe")).toHaveAttribute("src", /^blob:/);
  const binary = await page.evaluate(async id => {
    const bytes = await window.__TAURI_INTERNALS__.invoke("application_request", { command: "read_bytes", args: { id, rel: "image.png" } });
    return { buffer: bytes instanceof ArrayBuffer, length: bytes.byteLength, signature: Array.from(new Uint8Array(bytes).slice(0, 8)) };
  }, registered.workspaces[0].id);
  expect(binary).toEqual({ buffer: true, length: 9 * 1024 * 1024, signature: [137, 80, 78, 71, 13, 10, 26, 10] });
  await page.screenshot({ path: join(artifacts, "shared-pdf.png") });
  record("Original image and CSV viewers received WSL binary data; PDF loaded its blob frame and a file larger than the bridge frame arrived as ArrayBuffer");
  await page.locator("#tree .treerow", { hasText: "notes.txt" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Show in file manager", exact: true }).click();
  const explorerDirectory = join(String.raw`\\wsl.localhost`, target.distribution, target.workdir);
  const verifyReveal = fileURLToPath(new URL("./fixtures/windows-verify-reveal.ps1", import.meta.url));
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", verifyReveal, "-Directory", explorerDirectory, "-Selected", "notes.txt"], { timeout: 20000 });
  const revealed = await page.evaluate(id => window.__TAURI_INTERNALS__.invoke("application_request", { command: "reveal_path", args: { id, rel: "" } }), registered.projects.find(project => project.path === imported).id);
  expect(revealed).toBeNull();
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", verifyReveal, "-Directory", unc], { timeout: 20000 });
  record("Existing file menu selected the WSL file in native Explorer; project reveal opened its folder without launching a file association");
  await page.locator("#tabbar .tabadd .caret").click();
  await page.locator(".ui-search-picker-choice", { hasText: "New terminal" }).click();
  await expect(page.locator("#termview")).toBeVisible();
  await expect(page.locator("#termview .xterm-rows")).toContainText("project");
  const terminal = page.locator("#termview .xterm-helper-textarea");
  await terminal.pressSequentially("export SHARED_TOKEN=kept; printf 'WINDOWS_%s\\n' SHARED_TERMINAL");
  await terminal.press("Enter");
  await expect(page.locator("#termview .xterm-rows")).toContainText("WINDOWS_SHARED_TERMINAL");
  await page.screenshot({ path: join(artifacts, "shared-terminal.png") });
  record("Existing terminal tab ran a real WSL shell");
  // Exercise retained DOM state and native event/snapshot ordering, which mocks cannot prove.
  await page.locator("#railbody").getByRole("button", { name: "Desk", exact: true }).click();
  await composer.fill("Continue while connection is lost");
  await composer.press("Enter");
  await expect(page.locator("#deskView .bot").last()).toContainText("Waiting for attachment recovery");
  await composer.fill("Unsent draft must survive attachment recovery");
  await page.evaluate(() => { window.retainedWebview = "same-document"; });
  await page.evaluate(() => {
    const fetch = window.fetch;
    let paused = true;
    window.pendingSnapshots = [];
    window.fetch = async (url, options) => {
      const result = await fetch.call(window, url, options);
      if (paused && String(url).endsWith("/application_request") && JSON.parse(options.body).command === "chat_snapshot") {
        await new Promise(resolve => window.pendingSnapshots.push(resolve));
      }
      return result;
    };
    window.releaseSnapshots = () => {
      paused = false;
      window.fetch = fetch;
      window.pendingSnapshots.forEach(resolve => resolve());
    };
  });
  await page.locator("#deskView .tile .topen").first().click();
  await expect.poll(() => page.evaluate(() => window.pendingSnapshots.length)).toBeGreaterThan(0);
  await page.locator("#tabbar .tab", { hasText: "Terminal" }).click();
  await expect(terminal).toBeFocused();
  await page.evaluate(async () => { window.releaseSnapshots(); await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
  await expect(page.locator("#termview")).toBeVisible();
  await expect(terminal).toBeFocused();
  await terminal.pressSequentially("touch recovery-shell-ready; while [ ! -f release-recovery ]; do sleep 0.02; done; printf 'OUTPUT_%s\\n' AFTER_LOSS");
  await terminal.press("Enter");
  // Wait for all typed bytes to reach the shell before deliberately breaking its attachment.
  await expect.poll(() => linux("import pathlib,sys; print(pathlib.Path(sys.argv[1],'recovery-shell-ready').exists())", target.workdir)).toBe("True");
  await linux(`import os,pathlib,signal,sys
matches=[]
for entry in pathlib.Path('/proc').iterdir():
    if not entry.name.isdigit(): continue
    try: args=(entry/'cmdline').read_bytes().split(b'\\0')
    except (FileNotFoundError,PermissionError,ProcessLookupError): continue
    if b'--root' in args and args[args.index(b'--root')+1].decode()==sys.argv[1] and b'--transport' in args and args[args.index(b'--transport')+1]==b'resident': matches.append(int(entry.name))
assert len(matches)==1,matches
os.kill(matches[0],signal.SIGKILL)
pathlib.Path(sys.argv[2],'release-recovery').touch()`, target.root, target.workdir);
  await expect(page.locator("#termview .xterm-rows")).toContainText("OUTPUT_AFTER_LOSS", { timeout: 30000 });
  await terminal.pressSequentially("printf 'recovered=%s\\n' \"$SHARED_TOKEN\"");
  await terminal.press("Enter");
  await expect(page.locator("#termview .xterm-rows")).toContainText("recovered=kept");
  expect(await page.evaluate(() => window.retainedWebview)).toBe("same-document");
  await page.locator("#railbody").getByRole("button", { name: "Desk", exact: true }).click();
  await expect(composer).toHaveValue("Unsent draft must survive attachment recovery");
  await expect(page.locator("#deskView .bot").filter({ hasText: "Completed during attachment recovery" })).toHaveCount(1);
  expect(await linux("import json,pathlib,sys; print(sum(json.loads(line)=='Continue while connection is lost' for line in pathlib.Path(sys.argv[1],'provider-inputs').read_text().splitlines()))", target.workdir)).toBe("1");
  await composer.fill("");
  record("Killing only the WSL attachment recovered automatically without reloading: draft, one completed reply and the same shell survived without replaying input");
  await closeWindow();
  await launch(); await connect();
  await expect(page.locator("#deskView .bot").first()).toContainText("Native shared interface message");
  await page.locator("#deskView .tile .topen").first().click();
  await page.locator("#tabbar .tab", { hasText: "Terminal" }).click();
  await expect(page.locator("#termview")).toBeVisible();
  await expect(page.locator("#termview .xterm-rows")).toContainText("WINDOWS_SHARED_TERMINAL");
  const restored = page.locator("#termview .xterm-helper-textarea");
  await restored.pressSequentially("printf 'reopened=%s\\n' \"$SHARED_TOKEN\"");
  await restored.press("Enter");
  await expect(page.locator("#termview .xterm-rows")).toContainText("reopened=kept");
  expect(await linux("import pathlib,sys; print(len(pathlib.Path(sys.argv[1], 'provider-launches').read_text().splitlines()))", target.workdir)).toBe(launchesBeforeClose);
  record("Reopening the original interface restored the conversation and the same WSL shell");
  await page.locator("#railbody").getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.locator("#veil .sheet")).toBeVisible();
  await page.locator("#d-model").click();
  await page.locator(".ui-search-picker-choice", { hasText: "Native fixture" }).click();
  await expect(page.locator("#d-model")).toContainText("Native fixture");
  await expect(page.locator("#d-wt")).toBeChecked();
  await page.locator("#d-prompt").fill("Created through the original launcher");
  await page.locator("#d-add").click();
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", fileURLToPath(new URL("./fixtures/windows-select-directory.ps1", import.meta.url)), "-AppId", String(application.pid), "-Directory", attachment, "-Title", "Files to attach to the context", "-File"], { timeout: 30000 });
  await expect(page.locator("#d-inj")).toContainText("attachment ' ação.txt");
  await page.locator("#d-go").click();
  await expect(page.locator("#veil")).toBeHidden();
  // The launcher closes on submission. A responsive transport can return the board
  // before checkout completes; wait for the actual creation rather than queue ordering.
  let created;
  await expect.poll(async () => {
    const board = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }));
    created = board.workspaces.find(workspace => workspace.title === "Created through the original launcher");
    return created?.id;
  }, { timeout: 30000 }).toBeTruthy();
  await expect(page.locator("#chatwrap")).toBeVisible();
  expect(created.model).toBe("native-fixture");
  expect(created.worktree).not.toBe(target.workdir);
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], '.env').read_text())", created.worktree)).toBe("NATIVE_COPY=kept");
  await expect(page.locator("#chatwrap .bot").filter({ hasText: "Created through the original launcher" })).toHaveCount(0);
  await linux("import pathlib,sys; pathlib.Path(sys.argv[1], 'setup-release').touch()", target.workdir);
  await expect(page.locator("#chatwrap .bot")).toContainText("Created through the original launcher", { timeout: 25000 });
  await expect(page.locator("#chatwrap .bot")).toContainText("attachment ' ação.txt");
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'setup-done').exists())", created.worktree)).toBe("True");
  const setupTab = page.locator("#dockstrip .docktab").filter({ hasText: /^Setup$/ });
  if (!(await setupTab.getAttribute("class")).split(" ").includes("on") || await page.locator("#dock").evaluate(el => el.classList.contains("closed"))) await setupTab.click();
  await expect(page.locator("#dock-again")).toBeVisible();
  await linux("import pathlib,sys; p=pathlib.Path(sys.argv[1]); (p/'.env').write_text('NATIVE_COPY=local-edit'); (p/'setup-done').unlink()", created.worktree);
  await page.locator("#dock-again").click();
  await expect.poll(() => linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'setup-done').exists())", created.worktree)).toBe("True");
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], '.env').read_text())", created.worktree)).toBe("NATIVE_COPY=local-edit");
  record("Original Setup rerun used deferred preparation and preserved the locally edited copied file");
  await page.locator("#run-go").click();
  await expect(page.locator("#run-go")).toContainText("Stop");
  await expect(page.locator(".xterm-rows").filter({ hasText: "NATIVE_RUN" })).toContainText(`port=${created.port}`);
  await page.screenshot({ path: join(artifacts, "shared-run.png") });
  await page.locator("#run-go").click();
  await expect(page.locator("#run-go")).toContainText("Run");
  record("Existing Setup copied the ignored file and released initial input after completion; existing Run started and stopped a WSL script with its reserved port");
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", created.worktree)).toBe("Original file");
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", target.workdir)).toBe("Edited from the native Windows interface");
  await page.screenshot({ path: join(artifacts, "shared-launcher-created.png") });
  record("Original launcher discovered the WSL account/model and created an isolated Git worktree with its initial conversation");
  await page.locator("#tab-files").click();
  await page.locator("#tree .treerow", { hasText: "notes.txt" }).click();
  await expect(page.locator("#vtext")).toHaveValue("Original file");
  await page.locator("#vtext").fill("Reviewed through native Git");
  await page.locator("#vsave").click();
  await expect.poll(() => linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", created.worktree)).toBe("Reviewed through native Git");
  await page.locator("#tab-diff").click();
  const changed = page.locator('.git-group[data-scope="changes"] .git-file[data-path="notes.txt"]');
  await expect(changed).toBeVisible();
  await changed.locator(".git-file-name").click();
  await expect(page.locator("#dlist")).toContainText("Reviewed through native Git");
  await changed.getByRole("button", { name: "Stage file: notes.txt", exact: true }).click();
  await page.locator('.git-nav [data-mode="staged"]').click();
  await expect(page.locator('.git-group[data-scope="staged"] .git-file[data-path="notes.txt"]')).toBeVisible();
  await linux("import pathlib,sys; pathlib.Path(sys.argv[1], 'notes.txt').write_text('Later unstaged edit')", created.worktree);
  await page.locator("#git-message").fill("test: native Windows review");
  await page.locator("#git-commit").click();
  await expect(page.locator("#git-message")).toHaveValue("");
  expect(await linux("import subprocess,sys; print(subprocess.check_output(['git','-C',sys.argv[1],'show','HEAD:notes.txt'],text=True))", created.worktree)).toBe("Reviewed through native Git");
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", created.worktree)).toBe("Later unstaged edit");
  await page.locator('.git-nav [data-mode="changes"]').click();
  await expect(changed).toBeVisible();
  await page.locator('.git-nav [data-mode="history"]').click();
  await expect(page.locator(".git-history-row").first()).toContainText("test: native Windows review");
  await expect(page.locator("#dlist")).toContainText("Reviewed through native Git");
  await expect(page.locator("#dlist")).not.toContainText("Later unstaged edit");
  await page.screenshot({ path: join(artifacts, "shared-git.png") });
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", target.workdir)).toBe("Edited from the native Windows interface");
  record("Original Changes panel reviewed and staged a WSL diff, committed only its reviewed index, preserved later edits and displayed real Git history");
  await page.locator("#railbody .group").filter({ hasText: "selected ' project" }).click();
  await page.locator("#tree .treerow", { hasText: "imported.txt" }).click();
  await expect(page.locator("#vtext")).toHaveValue("Selected through Windows");
  await page.locator("#vtext").fill("Edited in the selected project");
  await page.locator("#vsave").click();
  await expect.poll(() => linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'imported.txt').read_text())", imported)).toBe("Edited in the selected project");
  await page.locator("#tabbar .tabadd button").first().click();
  await expect(page.locator("#termview .xterm-rows")).toContainText("selected");
  const projectTerminal = page.locator("#termview .xterm-helper-textarea");
  await projectTerminal.pressSequentially("printf 'SELECTED_%s\\n' PROJECT; pwd");
  await projectTerminal.press("Enter");
  await expect(page.locator("#termview .xterm-rows")).toContainText("SELECTED_PROJECT");
  await expect(page.locator("#termview .xterm-rows")).toContainText(imported);
  await page.screenshot({ path: join(artifacts, "shared-project.png") });
  const duplicate = await page.evaluate(path => window.__TAURI_INTERNALS__.invoke("application_request", { command: "add_project", args: { path } }), unc);
  expect(duplicate.path).toBe(imported);
  const wrong = await page.evaluate(async path => {
    try { await window.__TAURI_INTERNALS__.invoke("application_request", { command: "add_project", args: { path } }); return null; }
    catch (error) { return String(error); }
  }, join(String.raw`\\wsl.localhost`, "Other-Distribution", imported));
  expect(wrong).toContain("err.windows.otherDistribution");
  windowsProject = await mkdtemp(join(tmpdir(), "prometeu-selected ' "));
  await writeFile(join(windowsProject, "drive.txt"), "Windows drive file");
  const drive = await page.evaluate(path => window.__TAURI_INTERNALS__.invoke("application_request", { command: "add_project", args: { path } }), windowsProject);
  expect(drive.path.startsWith("/")).toBe(true);
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'drive.txt').read_text())", drive.path)).toBe("Windows drive file");
  await page.evaluate(id => window.__TAURI_INTERNALS__.invoke("application_request", { command: "reveal_path", args: { id, rel: "drive.txt" } }), drive.id);
  await execute("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", verifyReveal, "-Directory", windowsProject, "-Selected", "drive.txt"], { timeout: 20000 });
  // Exercise the original workspace controls across native IPC and real WSL effects.
  const lifecycleRow = page.locator(`#railbody [data-workspace="${created.id}"] > .navitem`);
  await lifecycleRow.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Rename", exact: true }).click();
  await page.locator("input.rename").fill("Native lifecycle");
  await page.locator("input.rename").press("Enter");
  await expect(lifecycleRow).toContainText("Native lifecycle");
  await lifecycleRow.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Pin to top", exact: true }).click();
  await expect.poll(async () => {
    const board = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }));
    return board.workspaces.find(w => w.id === created.id).pinned;
  }).toBe(true);
  await lifecycleRow.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Finish", exact: true }).click();
  const cleanupDialog = page.getByRole("dialog").filter({ has: page.locator(".cleanlist") });
  await expect(cleanupDialog.locator(".cleanrow")).toContainText("Native lifecycle");
  await expect(cleanupDialog.locator(".cleanrow input")).not.toBeChecked();
  await expect(cleanupDialog.locator("#c-go")).toBeDisabled();
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'notes.txt').read_text())", created.worktree)).toBe("Later unstaged edit");
  await cleanupDialog.locator(".cleanrow input").check();
  await cleanupDialog.locator("#c-go").click();
  await expect(cleanupDialog).toHaveCount(0);
  const cleaned = await page.evaluate(id => window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }).then(board => ({ workspace: board.workspaces.find(w => w.id === id), finalStage: board.stages.at(-1) })), created.id);
  expect(cleaned.workspace.archived).toBe(true);
  expect(cleaned.workspace.cleaned).toBe(true);
  expect(cleaned.workspace.stage).toBe(cleaned.finalStage);
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1]).exists())", created.worktree)).toBe("False");
  const retained = await page.evaluate(session => window.__TAURI_INTERNALS__.invoke("application_request", { command: "chat_snapshot", args: { session } }), created.tabs[0].id);
  expect(retained.text).toContain("turn.completed");
  await page.screenshot({ path: join(artifacts, "shared-lifecycle.png") });
  record("Original workspace menus renamed, pinned and finished the WSL workspace; existing cleanup required explicit dirty-worktree selection and retained its conversation");
  await page.getByRole("button", { name: "Actions for selected ' project", exact: true }).click();
  await page.getByRole("menuitem", { name: "Remove project", exact: true }).click();
  await expect(page.locator("#railbody .group").filter({ hasText: "selected ' project" })).toHaveCount(0);
  await page.getByRole("button", { name: "Actions for project ' ação", exact: true }).click();
  await page.getByRole("menuitem", { name: "Remove project", exact: true }).click();
  const finalBoard = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("application_request", { command: "load_board", args: null }));
  expect(finalBoard.workspaces).toHaveLength(2);
  expect(finalBoard.projects).toHaveLength(1);
  expect(finalBoard.projects[0].path).toBe(drive.path);
  expect(await linux("import pathlib,sys; print(pathlib.Path(sys.argv[1], 'imported.txt').read_text())", imported)).toBe("Edited in the selected project");
  record("Selected projects reused the original editor and terminal; drive paths used the connected WSL mount; duplicate/mismatched-distribution imports and removal preserved workspaces and files");
  expect(failures).toEqual([]);
  completed = true;
  }
} catch (error) {
  if (page) diagnostics.push(JSON.stringify(await page.evaluate(() => window.nativeNotices ?? []).catch(() => [])));
  failures.push(error.stack ?? String(error)); process.exitCode = 1;
  await page?.screenshot({ path: join(artifacts, "shared-failure.png") }).catch(() => {});
  await writeFile(join(artifacts, "shared-failure.html"), await page?.content().catch(() => "") ?? "");
} finally {
  if (oauthServer) {
    await linux("import pathlib,os,signal,sys; p=pathlib.Path(sys.argv[1],'oauth/server.pid'); pid=int(p.read_text()) if p.exists() else None; cmd=pathlib.Path('/proc',str(pid),'cmdline') if pid else None; os.kill(pid,signal.SIGTERM) if cmd and cmd.exists() and sys.argv[1].encode() in cmd.read_bytes() else None", base).catch(() => {});
    oauthServer.kill();
  }
  if (application) {
    try { await closeWindow(); }
    catch { await execute("taskkill.exe", ["/pid", String(application.pid), "/T", "/F"]).catch(() => {}); }
  }
  // A failed assertion can leave the window detached; reconnect only to this test's root.
  try {
    await linux(`import json,pathlib,subprocess,sys,time
runtime,root,workdir,codex=sys.argv[1:5]
if pathlib.Path(root, "resident.sock").exists():
    p=subprocess.Popen([runtime,"--transport","resident","--catalog",sys.argv[5],"--root",root,"--workdir",workdir,"--codex",codex],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    output,errors=p.communicate(json.dumps({"v":1,"id":1,"action":{"method":"shutdown"}})+"\\n",timeout=15)
    assert p.returncode == 0, errors
    assert any(frame.get("id") == 1 and frame.get("result", {}).get("stopped") is True for frame in map(json.loads, output.splitlines())), output
    deadline=time.monotonic()+5
    while pathlib.Path(root,"resident.sock").exists() and time.monotonic()<deadline: time.sleep(0.02)
    assert not pathlib.Path(root,"resident.sock").exists(), "Resident endpoint was not removed after shutdown"
`, target.executable, target.root, target.workdir, target.codex, config.bootstrap ? "application" : "workspace");
  } catch (error) { failures.push(`Cleanup: ${error.message}`); completed = false; process.exitCode = 1; }
  await writeFile(join(artifacts, "shared-result.json"), JSON.stringify({ completed, failures, diagnostics, steps, target }, null, 2));
  if (windowsProject) await rm(windowsProject, { recursive: true, force: true });
  if (completed) await linux("import shutil,sys; shutil.rmtree(sys.argv[1])", base);
  if (!completed) console.error(failures.join("\n"));
}
