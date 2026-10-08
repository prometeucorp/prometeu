import type { Down, Member, Shared, Up } from "../relay/src/protocol";
import type { TeamChannel } from "./team-channel";
import type { TeamSecurity } from "./team-security";
import type { ShareRights } from "./types";

/// Contracts between the portable collaboration core (team-member.ts and its features) and the shell that hosts it.
/// The desktop shell is team.ts; a browser shell supplies the same ports without Tauri. See ADR 0026.

export type Phase = "off" | "connecting" | "online";

/// A relay membership resolved by the shell: desktop team.json or a Cloud session.
export type Membership = {
  member: string;
  /// JSON scope bound into every encryption context; see docs/contracts/relay-v4.md.
  scope: string;
  /// Storage scope for the private identity, TOFU pins and receipts.
  privateScope: string;
  /// Consent scope persisted on shared workspaces: organization:<id>:<member> or team:<id>.
  shareScope: string;
  /// Shared-secret teams accept legacy boards without share_team and skip Cloud-only reconciliation.
  legacy: boolean;
  /// A fresh socket URL per connection or lease renewal: a one-time ticket for organizations, a credential for legacy teams.
  /// Null gives up silently; a rejection is reported and retried with backoff.
  url(): Promise<string | null>;
};

/// Private identity storage; the desktop keeps it in team-security.json through IPC.
export type SecurityStore = {
  read(): Promise<unknown>;
  write(state: unknown): Promise<void>;
};

/// A check captured when a frame is queued and re-evaluated before it leaves the socket.
export type Gate = () => boolean;

/// Feature hooks called by the member in registration order.
export type Feature = {
  /// A new encrypted channel exists for the next connection; declare owned shares before the welcome.
  connecting?(channel: TeamChannel): void;
  /// Capture a validity check for an outgoing frame, or null when the feature has no say.
  outgoing?(frame: Up): Gate | null;
  /// An authenticated plain frame, after the member updated directory state.
  frame?(frame: Down): void;
  /// An authenticated binary frame addressed to this member.
  binary?(data: ArrayBuffer): void;
  /// The socket is gone; drop per-connection state.
  closed?(): void;
  /// The membership is gone; drop everything.
  reset?(): void;
};

/// What a feature may use from the member.
export type Context = {
  send(frame: Up, completion?: (sent: boolean) => void): boolean;
  sendConfirmed(frame: Up): Promise<boolean>;
  /// Encrypt one binary frame for the recipients computed at send time.
  sendBinary(data: Uint8Array, recipients: () => string[], valid: Gate): boolean;
  changed(): void;
  fail(text: string): void;
  /// Increments on every connect and disconnect; compare to drop stale asynchronous work.
  generation(): number;
  phase(): Phase;
  you(): string | null;
  members(): Member[];
  shares(): Map<string, Shared>;
  membership(): Membership | null;
  channel(): TeamChannel | null;
  /// Write security state on the connection's queue, after the frames already queued and before a reconnect
  /// reloads the store. Resolves null instead of writing once that connection is gone.
  secure<T>(work: (security: TeamSecurity) => Promise<T>): Promise<T | null>;
  nameOf(member: string): string;
};

/// Destination for lines of the displayed remote conversation.
export type GuestSink = {
  live: (tab: string, bytes: Uint8Array) => void;
  /// Replace the view when a fresh snapshot arrives after owner or connection recovery.
  reset: (tab: string, bytes: Uint8Array) => void;
};

/// Local agent actions the owner feature needs from the shell.
export type OwnerHost = {
  setShared(id: string, shared: boolean, audience: string[] | null, remoteControl: boolean, team: string | null,
    rights: ShareRights | null): Promise<void>;
  snapshot(tab: string): Promise<{ text: string; seq: number }>;
  control(tab: string, frame: unknown): Promise<void>;
  prompt(tab: string, text: string): Promise<void>;
};
