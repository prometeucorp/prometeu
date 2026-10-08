import { isId, MEMBERS_MAX } from "../relay/src/protocol";
import type { ShareRights } from "./types";

/// Rights a shared workspace grants beyond viewing and commenting (ADR 0090). They name people, never the whole
/// organization; a person's devices act through that person.

export type Right = keyof ShareRights;

export const NO_RIGHTS: ShareRights = { send: [], control: [] };

/// Read rights from a board or an authenticated announcement. Unknown kinds are ignored so a newer owner stays
/// readable, and anything malformed grants nothing.
export function parseRights(value: unknown): ShareRights {
  const source = value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
  const people = (list: unknown) => Array.isArray(list) ? [...new Set(list.filter(isId))].slice(0, MEMBERS_MAX) : [];
  return { send: people(source.send), control: people(source.control) };
}

/// What a viewer may do in a remote conversation. The owner's own devices act through remote control, and an owner
/// from before rights announces none while still letting its audience act, so its controls stay.
export function viewerRights(announced: ShareRights | undefined, person: string, ownDevice: boolean): Record<Right, boolean> {
  if (!announced || ownDevice) return { send: true, control: true };
  return { send: announced.send.includes(person), control: announced.control.includes(person) };
}
