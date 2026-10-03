//! Layered tool selection: compose the global, project and workspace layers into the hub IDs a
//! session injects, one axis at a time. Pure (no Tauri, filesystem or network), so the chain is
//! testable without a board or a repository. See ADR 0045 and docs/contracts/plugin-marketplace.md.

use serde::{Deserialize, Serialize};

/// What a layer does with the set inherited from the layers above it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Base {
    /// Replace the inherited set with `add`; an empty `add` selects nothing from the hub.
    None,
    /// Apply `add` and `remove` over the inherited set. A hand-written layer that omits `base`
    /// means this, so declaring only `add` never erases what came from above.
    #[default]
    Inherit,
}

/// One axis of one layer. The layer itself is `Option<Selection>`: `None` inherits everything from
/// the layers above and contributes nothing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Selection {
    #[serde(default)]
    pub base: Base,
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

impl Selection {
    /// Replace the inherited set.
    pub fn only(add: Vec<String>) -> Selection {
        Selection {
            base: Base::None,
            add,
            remove: Vec::new(),
        }
    }
}

/// The three independent axes of one layer. Shared by the board's global layer, the `[tools]` table
/// of the project settings and the workspace triple, so all three deserialize identically. An absent
/// axis inherits from the layer above it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Tools {
    #[serde(default)]
    pub mcp: Option<Selection>,
    #[serde(default)]
    pub plugins: Option<Selection>,
    #[serde(default)]
    pub skills: Option<Selection>,
}

/// Resolve one axis across the three layers, in order, and keep only IDs the universe has, so a
/// layer written before a package was removed still resolves. Within a layer `add` comes first and
/// `remove` has the last word; the order of the inherited set is preserved and additions follow it.
pub fn resolve(
    global: &Option<Selection>,
    project: &Option<Selection>,
    workspace: &Option<Selection>,
    hub: &[String],
) -> Vec<String> {
    compose(&[global, project, workspace], hub)
}

/// Resolve one axis over an inherited CLI base (ADR 0046). The base ids act as an implicit lowest
/// layer under global: every layer sees them as the inherited set, so `base: "none"` replaces them
/// too and a lower layer may remove one without relisting the rest. The universe — hub plus base —
/// decides which ids survive.
pub fn resolve_with_base(
    base: &[String],
    global: &Option<Selection>,
    project: &Option<Selection>,
    workspace: &Option<Selection>,
    universe: &[String],
) -> Vec<String> {
    let seed = (!base.is_empty()).then(|| Selection {
        base: Base::Inherit,
        add: base.to_vec(),
        remove: Vec::new(),
    });
    compose(&[&seed, global, project, workspace], universe)
}

fn compose(layers: &[&Option<Selection>], universe: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for layer in layers.iter().flat_map(|layer| layer.as_ref()) {
        if layer.base == Base::None {
            ids.clear();
        }
        for id in &layer.add {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids.retain(|id| !layer.remove.contains(id));
    }
    ids.retain(|id| universe.contains(id));
    ids
}

#[cfg(test)]
mod tests {
    use super::{resolve, Base, Selection, Tools};

    fn names(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn replace(add: &[&str]) -> Selection {
        Selection::only(names(add))
    }

    fn delta(add: &[&str], remove: &[&str]) -> Selection {
        Selection {
            base: Base::Inherit,
            add: names(add),
            remove: names(remove),
        }
    }

    #[test]
    fn undeclared_layers_inject_nothing() {
        assert_eq!(resolve(&None, &None, &None, &names(&["a"])), names(&[]));
    }

    #[test]
    fn global_is_the_base_while_project_and_workspace_inherit() {
        let got = resolve(
            &Some(replace(&["a", "b"])),
            &None,
            &None,
            &names(&["a", "b", "c"]),
        );
        assert_eq!(got, names(&["a", "b"]));
    }

    #[test]
    fn base_none_replaces_parent_selection() {
        let got = resolve(
            &Some(replace(&["a", "b"])),
            &Some(replace(&["c"])),
            &None,
            &names(&["a", "b", "c"]),
        );
        assert_eq!(got, names(&["c"]));
    }

    #[test]
    fn base_none_without_add_clears_inherited_selection() {
        let got = resolve(
            &Some(replace(&["a", "b"])),
            &None,
            &Some(replace(&[])),
            &names(&["a", "b"]),
        );
        assert_eq!(got, names(&[]));
    }

    #[test]
    fn base_inherit_appends_additions_and_removes_existing_items() {
        let got = resolve(
            &Some(replace(&["a", "b"])),
            &Some(delta(&["c"], &["a"])),
            &Some(delta(&["d"], &[])),
            &names(&["a", "b", "c", "d"]),
        );
        assert_eq!(got, names(&["b", "c", "d"]));
    }

    #[test]
    fn removal_in_the_last_layer_overrides_addition_in_the_first() {
        let got = resolve(
            &Some(replace(&["a", "b"])),
            &None,
            &Some(delta(&[], &["a"])),
            &names(&["a", "b"]),
        );
        assert_eq!(got, names(&["b"]));
    }

    #[test]
    fn remove_overrides_add_in_the_same_layer() {
        let got = resolve(
            &None,
            &Some(delta(&["a", "b"], &["a"])),
            &None,
            &names(&["a", "b"]),
        );
        assert_eq!(got, names(&["b"]));
    }

    #[test]
    fn ignores_ids_outside_the_hub_and_resolves_the_rest() {
        let got = resolve(
            &Some(replace(&["gone"])),
            &Some(delta(&["a", "also-gone"], &[])),
            &None,
            &names(&["a", "b"]),
        );
        assert_eq!(got, names(&["a"]));
    }

    #[test]
    fn repeated_additions_do_not_duplicate_items() {
        let got = resolve(
            &Some(replace(&["a"])),
            &Some(delta(&["a", "b"], &[])),
            &Some(delta(&["b"], &[])),
            &names(&["a", "b"]),
        );
        assert_eq!(got, names(&["a", "b"]));
    }

    #[test]
    fn layer_accepts_null_and_the_contract_object() {
        let inherit: Option<Selection> = serde_json::from_str("null").unwrap();
        assert_eq!(inherit, None);

        let none: Option<Selection> =
            serde_json::from_str(r#"{"base":"none","add":["a"],"remove":[]}"#).unwrap();
        assert_eq!(none, Some(replace(&["a"])));

        // A layer may declare only one of the deltas.
        let partial: Selection =
            serde_json::from_str(r#"{"base":"inherit","remove":["a"]}"#).unwrap();
        assert_eq!(partial, delta(&[], &["a"]));

        // An omitted `base` inherits instead of erasing the layers above.
        let implicit: Selection = serde_json::from_str(r#"{"add":["a"]}"#).unwrap();
        assert_eq!(implicit.base, Base::Inherit);
    }

    #[test]
    fn serialized_shape_matches_the_contract() {
        let json = serde_json::to_string(&Some(replace(&[]))).unwrap();
        assert_eq!(json, r#"{"base":"none","add":[],"remove":[]}"#);
        assert_eq!(serde_json::to_string(&None::<Selection>).unwrap(), "null");
    }

    /// The CLI-inherited base is the implicit lowest layer: it flows through inheriting layers,
    /// a workspace removal drops one id, and `base: "none"` replaces it (ADR 0046).
    #[test]
    fn cli_base_participates_in_the_chain() {
        use super::resolve_with_base;
        let base = names(&["cli-a", "cli-b"]);
        let universe = names(&["cli-a", "cli-b", "hub-a"]);

        // With no declared layer the base itself resolves; deciding that nothing is injected when
        // no layer declares the axis stays with the caller (session::resolve_axis).
        assert_eq!(
            resolve_with_base(&base, &None, &None, &None, &universe),
            names(&["cli-a", "cli-b"])
        );

        // An inheriting workspace addition keeps the base and appends.
        let got = resolve_with_base(
            &base,
            &None,
            &None,
            &Some(delta(&["hub-a"], &[])),
            &universe,
        );
        assert_eq!(got, names(&["cli-a", "cli-b", "hub-a"]));

        // Removing one inherited CLI id does not relist the rest.
        let got = resolve_with_base(
            &base,
            &None,
            &None,
            &Some(delta(&[], &["cli-a"])),
            &universe,
        );
        assert_eq!(got, names(&["cli-b"]));

        // A global replacement also replaces the CLI base.
        let got = resolve_with_base(&base, &Some(replace(&["hub-a"])), &None, &None, &universe);
        assert_eq!(got, names(&["hub-a"]));

        // Base ids outside the universe are dropped, so a server deleted from the CLI config
        // resolves away even while a stale removal still references it.
        let got = resolve_with_base(&names(&["gone"]), &None, &None, &None, &universe);
        assert_eq!(got, names(&[]));

        // An empty base behaves exactly like resolve.
        assert_eq!(
            resolve_with_base(&[], &Some(replace(&["hub-a"])), &None, &None, &universe),
            resolve(&Some(replace(&["hub-a"])), &None, &None, &universe)
        );
    }

    #[test]
    fn toml_tools_table_becomes_a_layer() {
        #[derive(serde::Deserialize)]
        struct File {
            #[serde(default)]
            tools: Tools,
        }

        let parsed: File = toml::from_str(
            r#"
[scripts]
setup = "npm install"

[tools]
mcp = { base = "inherit", add = ["notion"] }
plugins = { base = "none", add = ["revisor"] }
"#,
        )
        .unwrap();
        assert_eq!(parsed.tools.mcp, Some(delta(&["notion"], &[])));
        assert_eq!(parsed.tools.plugins, Some(replace(&["revisor"])));
        // An absent axis inherits the layers above it.
        assert_eq!(parsed.tools.skills, None);
    }
}
