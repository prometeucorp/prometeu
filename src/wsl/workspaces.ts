import type { Board } from "../types";
import type { Session } from "./session";

export type Catalog = { v: 1; active: string; board: Board };
export type WorktreeRequest = { title: string; path: string; branch: string; base: string };
export interface WorkspacePort {
  createWorktree(request: WorktreeRequest): Promise<Catalog>;
  listWorkspaces(): Promise<Catalog>;
  createWorkspace(title: string, path: string): Promise<Catalog>;
  selectWorkspace(id: string): Promise<Catalog>;
  setWorkspaceStage(id: string, stage: string): Promise<Catalog>;
}

/** Catalog mutations and selection use the host; drafts remain local presentation state. */
export class Workspaces {
  catalog: Catalog | null = null;
  pending = false;
  error = "";
  private drafts = new Map<string, string>();
  private epoch = 0;
  private scope: string | null = null;
  constructor(private port: WorkspacePort, private session: Session, private changed: () => void) {}
  attach(scope: string) {
    const changed = this.scope !== null && this.scope !== scope;
    this.epoch++; this.pending = false; this.scope = scope;
    if (changed) { this.catalog = null; this.drafts.clear(); }
    return changed;
  }
  detach(draft: string) {
    if (this.catalog) this.drafts.set(this.catalog.active, draft);
    this.epoch++; this.pending = false;
  }
  draft() { return this.catalog ? this.drafts.get(this.catalog.active) ?? "" : ""; }
  async load() { return this.run(() => this.port.listWorkspaces()); }
  async create(title: string, path: string) {
    return this.run(() => this.port.createWorkspace(title, path));
  }
  async createWorktree(request: WorktreeRequest) {
    return this.run(() => this.port.createWorktree(request));
  }
  async stage(id: string, stage: string) {
    await this.run(() => this.port.setWorkspaceStage(id, stage));
  }
  async select(id: string, draft: string): Promise<string | null> {
    if (!this.catalog || this.pending || this.session.pending || id === this.catalog.active) return null;
    this.drafts.set(this.catalog.active, draft);
    let selected: Catalog | undefined;
    const success = await this.run(async () => {
      if (!await this.session.select(async () => { selected = await this.port.selectWorkspace(id); })) throw new Error(this.session.error);
      return selected!;
    });
    return success ? this.drafts.get(id) ?? "" : null;
  }
  private async run(run: () => Promise<Catalog>) {
    if (this.pending) return false;
    const epoch = this.epoch;
    this.pending = true; this.error = ""; this.changed();
    try {
      const catalog = await run();
      if (epoch !== this.epoch) return false;
      this.catalog = catalog; return true;
    }
    catch (error) { if (epoch === this.epoch) this.error = String(error instanceof Error ? error.message : error); return false; }
    finally { if (epoch === this.epoch) { this.pending = false; this.changed(); } }
  }
}
