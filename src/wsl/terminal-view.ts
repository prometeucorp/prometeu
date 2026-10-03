import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { button } from "../ui";
import { h } from "../util";
import { t } from "../i18n";
import { SupportingTerminal, type TerminalPort } from "./terminal";
import "@xterm/xterm/css/xterm.css";

export function terminalView(port: TerminalPort) {
  const root = h("section", "wsl-terminal");
  root.setAttribute("aria-label", t("wsl.terminal"));
  const canvas = h("div", "wsl-terminal-canvas");
  const status = h("span", "wsl-terminal-status"); status.setAttribute("role", "status");
  const error = h("div", "wsl-error"); error.setAttribute("role", "alert");
  const tokens = getComputedStyle(document.documentElement);
  const terminal = new Terminal({ cursorBlink: true, fontSize: 12, scrollback: 4000, fontFamily: tokens.getPropertyValue("--mono").trim(), theme: { background: tokens.getPropertyValue("--bg").trim(), foreground: tokens.getPropertyValue("--fg").trim() } });
  const fit = new FitAddon(); terminal.loadAddon(fit);
  const controller = new SupportingTerminal(port, {
    reset: (data, consumed) => { canvas.hidden = false; ensureMounted(); fit.fit(); terminal.reset(); terminal.write(data, consumed); },
    write: (data, consumed) => terminal.write(data, consumed),
  }, paint);
  let connected = false;
  let mounted = false;
  const open = button(t("wsl.terminalOpen"), () => {
    canvas.hidden = false;
    ensureMounted(); fit.fit();
    void controller.open(terminal.cols, terminal.rows).then(() => terminal.focus());
  });
  const close = button(t("wsl.terminalClose"), () => void controller.close(), "ghost");
  const controls = h("div", "row"); controls.append(open, close, status);
  root.append(controls, error, canvas); canvas.hidden = true;
  terminal.onData(data => controller.input(new TextEncoder().encode(data)));
  terminal.onBinary(data => controller.input(Uint8Array.from(data, char => char.charCodeAt(0))));
  terminal.onResize(({ cols, rows }) => void controller.resize(cols, rows));
  let resizeFrame = 0;
  const observer = new ResizeObserver(() => {
    cancelAnimationFrame(resizeFrame);
    resizeFrame = requestAnimationFrame(() => { if (mounted && !canvas.hidden) fit.fit(); });
  });
  observer.observe(canvas);
  function ensureMounted() { if (!mounted) { terminal.open(canvas); mounted = true; } }
  function paint() {
    open.disabled = !connected || !controller.supported || controller.pending || controller.id !== null;
    close.disabled = !connected || controller.pending || controller.id === null;
    terminal.options.disableStdin = !controller.running || controller.pending;
    status.textContent = t(!connected ? "wsl.terminalOffline" : !controller.supported ? "wsl.terminalUnsupported" : controller.pending ? "wsl.pending" : controller.running ? "wsl.terminalRunning" : controller.id ? "wsl.terminalExited" : "wsl.terminalIdle");
    if (connected && controller.id && !controller.pending && !controller.running && controller.code !== null) status.textContent = t("wsl.terminalExitedCode", { code: controller.code });
    const errors: Record<string, string> = { terminal_input_limit: t("wsl.terminalInputLimit"), terminal_stream_gap: t("wsl.terminalStreamGap") };
    error.textContent = errors[controller.error] ?? controller.error;
  }
  paint();
  return {
    root,
    update(value: boolean) { const attached = !connected && value; const detached = connected && !value; connected = value; if (detached) controller.disconnected(); if (attached) void controller.attach(); paint(); },
    destroy() { observer.disconnect(); cancelAnimationFrame(resizeFrame); controller.dispose(); terminal.dispose(); },
  };
}
