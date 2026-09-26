import * as mcp from "./mcp";
import * as plugins from "./plugins";
import * as skills from "./skills";
import { resourceView } from "./components/resource-view";
import { resourceLabels } from "./resources/labels";
import type { ResourceFilter, ResourceSelection } from "./resources/model";

let selection: ResourceSelection = { filter: "all", query: "" };
export function setResourceFilter(filter: ResourceFilter) { selection.filter = filter; }

/** Bind the presentation to hub-owned actions. No DOM crosses the hub boundary. */
export function resourceSettings(defaults: () => void) {
  return resourceView({
    labels: resourceLabels(),
    snapshot: { items: [...plugins.resourceItems(), ...mcp.resourceItems(), ...skills.resourceItems()] },
    selection, changed: next => { selection = next; }, defaults,
    add: anchor => [...plugins.settingsActions(), "sep", ...mcp.settingsActions(anchor), "sep", ...skills.settingsActions()],
  }).root;
}
