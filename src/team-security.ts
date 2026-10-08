import { isId, MEMBERS_MAX } from "../relay/src/protocol";
import { generateIdentity, validateIdentity, validatePublicKey, type Identity } from "./team-crypto";

type Scope = { identity: Identity; peers: Record<string, string>; receipts?: Record<string, number>; clock?: number;
  sequence?: number; shares?: Record<string, { owner: string; key: string; revision: number; message: string }>;
  /** Member → key whose remote input the owner approved (ADR 0090). */
  approved: Record<string, string>;
  /** Member → workspaces where input from an unapproved key was discarded, until the owner answers. */
  paused?: Record<string, string[]> };
type State = { version: 1; scopes: Record<string, unknown> };
type Write = (state: unknown) => Promise<void>;

const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const dictionary = <T>(source: Record<string, T> = {}): Record<string, T> => Object.assign(Object.create(null), source);

/// Workspaces remembered per paused member; each one names where the owner sees the notice.
const PAUSED_MAX = 64;

/** Identity pins survive tickets, reconnects and organization switches; a member's new key replaces the pin.
 *  Remote input additionally requires the owner's approval of that member and key (ADR 0090). */
export class TeamSecurity {
  readonly identity: Identity;
  private observed = new Map<string, string | undefined>();
  private self: string | undefined;
  private pending = Promise.resolve();

  private constructor(private scope: string, private state: State, private current: Scope, private write: Write) {
    this.identity = current.identity;
  }

  static async load(scope: string, read: () => Promise<unknown>, write: Write): Promise<TeamSecurity> {
    if (!scope || typeof scope !== "string") throw new Error("Invalid identity scope");
    const value = await read();
    if (value != null && (!record(value) || value.version !== 1 || !record(value.scopes))) {
      throw new Error("Invalid identity storage");
    }
    const state: State = { version: 1, scopes: dictionary(value == null ? {} : (value as State).scopes) };
    let current: Scope;
    if (Object.prototype.hasOwnProperty.call(state.scopes, scope)) {
      const saved = state.scopes[scope];
      if (!record(saved) || !record(saved.peers) || Object.keys(saved.peers).length > MEMBERS_MAX) {
        throw new Error("Invalid identity storage");
      }
      const identity = await validateIdentity(saved.identity);
      const peers = dictionary<string>();
      for (const [member, key] of Object.entries(saved.peers)) {
        if (!isId(member) || typeof key !== "string") throw new Error("Invalid identity peer");
        await validatePublicKey(key);
        peers[member] = key;
      }
      if (saved.receipts !== undefined && (!record(saved.receipts) || Object.keys(saved.receipts).length > 4096 ||
        Object.entries(saved.receipts).some(([id, at]) => !isId(id) || !Number.isSafeInteger(at)))) throw new Error("Invalid replay storage");
      if (saved.clock !== undefined && (!Number.isSafeInteger(saved.clock) || Number(saved.clock) < 0)) throw new Error("Invalid replay clock");
      if (saved.sequence !== undefined && (!Number.isSafeInteger(saved.sequence) || Number(saved.sequence) < 0)) throw new Error("Invalid share sequence");
      if (saved.shares !== undefined && (!record(saved.shares) || Object.keys(saved.shares).length > 4096 ||
        Object.entries(saved.shares).some(([ws, v]) => !isId(ws) || !record(v) || !isId(v.owner) || !isId(v.message) ||
          typeof v.key !== "string" || !Number.isSafeInteger(v.revision) || Number(v.revision) < 1))) throw new Error("Invalid share history");
      if (saved.approved !== undefined && (!record(saved.approved) || Object.keys(saved.approved).length > MEMBERS_MAX)) {
        throw new Error("Invalid input approvals");
      }
      // Files from before approvals existed trust the keys already pinned, so upgrading does not stop teams.
      const approved = dictionary<string>(saved.approved === undefined ? peers : {});
      for (const [member, key] of Object.entries(saved.approved ?? {})) {
        if (!isId(member) || typeof key !== "string") throw new Error("Invalid input approvals");
        await validatePublicKey(key);
        approved[member] = key;
      }
      if (saved.paused !== undefined && (!record(saved.paused) || Object.keys(saved.paused).length > MEMBERS_MAX ||
        Object.entries(saved.paused).some(([member, list]) => !isId(member) || !Array.isArray(list) ||
          list.length > PAUSED_MAX || !list.every(isId)))) throw new Error("Invalid paused input");
      current = { identity, peers, receipts: saved.receipts as Scope["receipts"], clock: saved.clock as number | undefined,
        sequence: saved.sequence as number | undefined, shares: saved.shares as Scope["shares"], approved,
        paused: saved.paused === undefined ? undefined : dictionary(saved.paused as Record<string, string[]>) };
    } else {
      current = { identity: await generateIdentity(), peers: dictionary(), approved: dictionary() };
      state.scopes[scope] = current;
      await write(state);
    }
    return new TeamSecurity(scope, state, current, write);
  }

  private serialize(operation: () => Promise<void>): Promise<void> {
    const next = this.pending.then(operation);
    this.pending = next.catch(() => {});
    return next;
  }

  private async save(peers: Record<string, string>): Promise<void> {
    await this.commit({ ...this.current, identity: this.identity, peers });
  }

  private async commit(current: Scope): Promise<void> {
    const state: State = { version: 1, scopes: dictionary(this.state.scopes) };
    state.scopes[this.scope] = current;
    await this.write(state);
    this.state = state;
    this.current = current;
  }

  async nextRevision(): Promise<number> {
    let revision = 0;
    await this.serialize(async () => {
      revision = (this.current.sequence ?? 0) + 1;
      if (!Number.isSafeInteger(revision)) throw new Error("Share sequence exhausted");
      await this.commit({ ...this.current, sequence: revision });
    });
    return revision;
  }

  observeShare(ws: string, owner: string, key: string, revision: number, message: string): Promise<void> {
    return this.serialize(async () => {
      if (!isId(ws) || !isId(owner) || !isId(message) || !Number.isSafeInteger(revision) || revision < 1) throw new Error("Invalid share revision");
      const previous = this.current.shares?.[ws];
      if (previous && (previous.owner !== owner || (previous.key === key &&
        (revision < previous.revision || (revision === previous.revision && message !== previous.message))))) throw new Error("Repeated or replaced share");
      if (previous?.key === key && previous.revision === revision) return;
      const shares = dictionary(this.current.shares);
      shares[ws] = { owner, key, revision, message };
      if (Object.keys(shares).length > 4096) throw new Error("Share history full");
      await this.commit({ ...this.current, shares });
    });
  }

  observe(members: Array<{ id: string; key?: string }>, self: string): Promise<void> {
    return this.serialize(async () => {
      // Clear availability before async validation so a bad directory cannot leave
      // previously available keys usable while this observation is being checked.
      this.observed.clear();
      if (!isId(self) || !Array.isArray(members) || members.length > MEMBERS_MAX
        || (this.self !== undefined && this.self !== self)) throw new Error("Invalid identity directory");
      this.self = self;
      const observed = new Map<string, string | undefined>();
      for (const member of members) {
        if (!member || !isId(member.id) || observed.has(member.id)
          || (member.key !== undefined && typeof member.key !== "string")) throw new Error("Invalid identity directory");
        if (member.key !== undefined) await validatePublicKey(member.key);
        observed.set(member.id, member.key);
      }
      this.observed = observed;
      const peers = dictionary(this.current.peers);
      let added = false;
      for (const [member, key] of observed) {
        if (key === undefined) continue;
        if (member === self) {
          if (key !== this.identity.publicKey) throw new Error("Own identity key changed");
          continue;
        }
        // A reinstall or a new pairing arrives as a different key. Adopt it silently instead of blocking
        // content until someone compares codes by hand; the trade-off is in ADR 0042.
        if (peers[member] !== key) {
          peers[member] = key;
          added = true;
        }
      }
      if (Object.keys(peers).length > MEMBERS_MAX) throw new Error("Too many identity peers");
      if (added) await this.save(peers);
    });
  }

  key(member: string): string | undefined {
    const pinned = member === this.self ? this.identity.publicKey : this.current.peers[member];
    return pinned !== undefined && this.observed.get(member) === pinned ? pinned : undefined;
  }

  /** Content follows any adopted key (ADR 0042); input runs only from a member and key the owner approved. */
  approved(member: string): boolean {
    const key = this.key(member);
    return key !== undefined && (member === this.self || this.current.approved[member] === key);
  }

  /** Approve the current keys of these members and forget their paused input. Resolves how many became approved. */
  approve(members: string[]): Promise<number> {
    let count = 0;
    return this.serialize(async () => {
      const approved = dictionary(this.current.approved);
      const paused = dictionary(this.current.paused);
      let answered = false;
      for (const member of new Set(members)) {
        const key = this.key(member);
        if (key === undefined || member === this.self) continue;
        if (approved[member] !== key) { approved[member] = key; count++; }
        if (paused[member]) { delete paused[member]; answered = true; }
      }
      if (!count && !answered) return;
      if (Object.keys(approved).length > MEMBERS_MAX) throw new Error("Too many input approvals");
      await this.commit({ ...this.current, approved, paused });
    }).then(() => count);
  }

  /** Remember that input from an unapproved member was discarded in a workspace. Resolves whether that is news. */
  pause(member: string, ws: string): Promise<boolean> {
    let added = false;
    return this.serialize(async () => {
      if (!isId(member) || !isId(ws) || this.approved(member) || this.current.paused?.[member]?.includes(ws)) return;
      const paused = dictionary(this.current.paused);
      paused[member] = [...(paused[member] ?? []), ws].slice(-PAUSED_MAX);
      if (Object.keys(paused).length > MEMBERS_MAX) throw new Error("Too many paused members");
      await this.commit({ ...this.current, paused });
      added = true;
    }).then(() => added);
  }

  /** Forget paused input without approving it; the next discarded input brings the notice back. */
  dismiss(members: string[]): Promise<void> {
    return this.serialize(async () => {
      if (!members.some(member => this.current.paused?.[member])) return;
      const paused = dictionary(this.current.paused);
      for (const member of members) delete paused[member];
      await this.commit({ ...this.current, paused });
    });
  }

  /** Workspaces with discarded input for each member whose current key is still unapproved. */
  paused(): Record<string, string[]> {
    return Object.fromEntries(Object.entries(this.current.paused ?? {})
      .filter(([member]) => this.key(member) !== undefined && !this.approved(member)));
  }

  /** Persist before executing remote input; clock rollback fails closed. */
  consume(id: string, expires: number): Promise<void> {
    return this.serialize(async () => {
      const now = Date.now();
      if (!isId(id) || !Number.isSafeInteger(expires) || expires < now || expires > now + 120_000 ||
        now < (this.current.clock ?? 0) || this.current.receipts?.[id] !== undefined) throw new Error("Expired or repeated input");
      const receipts = Object.fromEntries(Object.entries(this.current.receipts ?? {}).filter(([, at]) => at >= now));
      if (Object.keys(receipts).length >= 4096) throw new Error("Replay storage full");
      receipts[id] = expires;
      const current = { ...this.current, receipts, clock: now };
      await this.commit(current);
    });
  }
}
