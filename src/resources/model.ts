import { matchesSettings, type ResourceFilter } from "../settings-navigation";
import type { IconName } from "../../packages/design-system/src/icons";
import type { Item } from "../menu";

export type { ResourceFilter };
export type ResourceOrigin = { label: string; here?: boolean; hint?: string };
export type ResourceItem = {
  /** Unique across resource kinds, local entries and pending catalog groups. Also restores focus. */
  key: string;
  id: string;
  kind: Exclude<ResourceFilter, "all">;
  description: string;
  origins: ResourceOrigin[];
  glyph: IconName;
  scope?: string;
  status?: string;
  busy?: boolean;
  actions: Item[];
};
export type ResourceSnapshot = {
  items: ResourceItem[];
  loading?: boolean;
  error?: string;
};
export type ResourceSelection = { filter: ResourceFilter; query: string };

export function matchesResource(item: ResourceItem, selection: ResourceSelection, kindLabel: string): boolean {
  return (selection.filter === "all" || selection.filter === item.kind)
    && matchesSettings(selection.query, `${item.id} ${item.description} ${item.origins.map(origin => origin.label).join(" ")} ${item.status ?? ""} ${kindLabel}`);
}
