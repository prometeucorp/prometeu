import type { ConnectionRecovery } from "../connection";

/** Only attachment and read-only view restoration are retried. User operations never enter here. */
export class Recovery implements ConnectionRecovery {
  private listeners = new Set<() => Promise<void>>();
  private pending: Promise<void> | null = null;
  private requested = false;

  constructor(private connect: () => Promise<boolean>, private status: (recovering: boolean) => void) {}

  subscribe(restore: () => Promise<void>) {
    this.listeners.add(restore);
    return () => { this.listeners.delete(restore); };
  }

  recover(): Promise<void> {
    this.requested = true;
    if (this.pending) return this.pending;
    this.pending = this.restore().finally(() => { this.pending = null; });
    return this.pending;
  }

  private async restore() {
    let restoring = false;
    let delay = 250;
    for (;;) {
      this.requested = false;
      try {
        restoring = await this.connect() || restoring;
        if (restoring) {
          const results = await Promise.allSettled([...this.listeners].map(restore => restore()));
          const failure = results.find(result => result.status === "rejected");
          if (failure) throw failure.reason;
        }
        if (this.requested) continue;
        this.status(false);
        return;
      } catch {
        this.status(true);
        await new Promise(resolve => setTimeout(resolve, delay));
        delay = Math.min(delay * 2, 5000);
      }
    }
  }
}
