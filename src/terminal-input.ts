/** xterm 5.5.0 marks keyboard, IME, paste and SGR mouse data at this synchronous boundary.
 * Keeping the origin on a stack also handles nested input without disabling user events while
 * asynchronous output parses. Focus reports are view events although xterm does not mark them.
 */
type CoreInput = { triggerDataEvent(data: string, wasUserInput?: boolean): void };
type XtermCoreInput = { _core?: { coreService?: CoreInput } };

export function terminalInputOrigin(terminal: unknown) {
  const core = (terminal as XtermCoreInput)._core?.coreService;
  let user = false;
  const original = core?.triggerDataEvent;
  const wrapped: CoreInput["triggerDataEvent"] = function(data, wasUserInput = false) {
    const previous = user;
    user = wasUserInput;
    try { original!.call(core, data, wasUserInput); }
    finally { user = previous; }
  };
  if (core && original) core.triggerDataEvent = wrapped;
  return {
    supported: !!original,
    isUser: (data: string) => user || data === "\x1b[I" || data === "\x1b[O",
    dispose: () => { if (core?.triggerDataEvent === wrapped) core.triggerDataEvent = original!; },
  };
}
