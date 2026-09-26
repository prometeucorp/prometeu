import { batteryBudget, foreground, type BackgroundContext } from "./background";

/** Switching apps often must not turn each return into a networked sweep of every clone. */
const RETURN_SPACING = 60_000;

/** The general PR sweep is advisory; explicit task monitoring has its own clock. */
export class PrScanPolicy {
  private lastRequested: number | null = null;

  mark(now: number) { this.lastRequested = now; }

  remaining(context: BackgroundContext, now: number): number {
    const interval = !foreground(context) ? 900_000 : batteryBudget(context) ? 300_000 : 180_000;
    return this.lastRequested === null ? 0 : Math.max(0, interval - (now - this.lastRequested));
  }

  /** A return to the foreground scans at once unless the last request is under a minute old. */
  returnedToForeground(before: BackgroundContext, after: BackgroundContext, now: number): boolean {
    if (foreground(before) || !foreground(after)) return false;
    return this.lastRequested === null || now - this.lastRequested >= RETURN_SPACING;
  }
}
