import type { BackgroundContext } from "./background";

/** The general PR sweep is advisory; explicit task monitoring has its own clock. */
export class PrScanPolicy {
  private lastRequested: number | null = null;

  mark(now: number) { this.lastRequested = now; }

  remaining(context: BackgroundContext, now: number): number {
    const interval = !context.visible || !context.focused
      ? 900_000
      : context.power === "ac" ? 180_000 : 300_000;
    return this.lastRequested === null ? 0 : Math.max(0, interval - (now - this.lastRequested));
  }

  returnedToForeground(before: BackgroundContext, after: BackgroundContext): boolean {
    return !(before.visible && before.focused) && after.visible && after.focused;
  }
}
