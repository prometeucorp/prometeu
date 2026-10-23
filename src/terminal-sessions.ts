/// Terminal sessions (ADR 0092) run on the macOS and Linux backend. The Windows shell turns them
/// off before the interface loads, because its WSL runtime does not implement them yet.
let available = true;

export const terminalSessions = () => available;

export function withoutTerminalSessions() {
  available = false;
}
