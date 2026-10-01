//! Pick the provider a task starts with. Pure: availability arrives as a function, so builders,
//! installations and sign-ins combine in tests without CLIs or accounts.

use super::{Profile, ProviderRule};
use crate::state::{Choice, ProviderId, Workspace};

/// The provider a task starts with and whether it shares a builder's family.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub choice: Choice,
    pub same_family: bool,
}

/// Providers of the workspace's ordinary tabs: each tab's own choice, or the workspace default it
/// inherits. Task tabs do not build. A workspace without ordinary tabs counts its default provider.
pub fn builders(ws: &Workspace) -> Vec<ProviderId> {
    let mut found = Vec::new();
    for tab in ws.tabs.iter().filter(|tab| tab.task.is_none()) {
        let agent = tab.choice.as_ref().map_or(ws.agent, |choice| choice.agent);
        if !found.contains(&agent) {
            found.push(agent);
        }
    }
    if found.is_empty() {
        found.push(ws.agent);
    }
    found
}

/// A fixed profile keeps its choice. Otherwise take the first usable candidate outside the
/// builders, then the first usable one, then the first candidate, so the spawn reports why it
/// cannot run instead of this rule refusing the review. `usable` runs at most once per candidate
/// and only when needed, because checking a provider may start a process.
pub fn pick(
    profile: &Profile,
    builders: &[ProviderId],
    mut usable: impl FnMut(ProviderId) -> bool,
) -> Pick {
    let candidates = &profile.candidates;
    if profile.provider_rule == ProviderRule::Fixed || candidates.is_empty() {
        return Pick {
            choice: profile.choice.clone(),
            same_family: false,
        };
    }
    let mut known: Vec<Option<bool>> = vec![None; candidates.len()];
    let mut ready =
        |index: usize| *known[index].get_or_insert_with(|| usable(candidates[index].agent));
    let index = (0..candidates.len())
        .find(|&index| !builders.contains(&candidates[index].agent) && ready(index))
        .or_else(|| (0..candidates.len()).find(|&index| ready(index)))
        .unwrap_or(0);
    let choice = candidates[index].clone();
    Pick {
        same_family: builders.contains(&choice.agent),
        choice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Access, Permission};
    use crate::state::Tab;
    use serde_json::json;
    use ProviderId::{Antigravity, Claude, Codex};

    fn choice(agent: ProviderId) -> Choice {
        Choice {
            agent,
            ..Default::default()
        }
    }

    fn review(candidates: &[ProviderId]) -> Profile {
        Profile {
            id: "review".into(),
            name: "Review".into(),
            prompt: "Review".into(),
            choice: choice(candidates[0]),
            provider_rule: ProviderRule::DifferentFromBuilder,
            candidates: candidates.iter().copied().map(choice).collect(),
            access: Access::ReadOnly,
            mcp: None,
            plugins: None,
            skills: vec![],
            permission: Permission::Ask,
            watch: None,
        }
    }

    fn picked(pick: Pick) -> (ProviderId, bool) {
        (pick.choice.agent, pick.same_family)
    }

    #[test]
    fn prefers_a_family_that_did_not_build() {
        let p = review(&[Codex, Claude]);
        assert_eq!(picked(pick(&p, &[Claude], |_| true)), (Codex, false));
        assert_eq!(picked(pick(&p, &[Codex], |_| true)), (Claude, false));
    }

    #[test]
    fn mixed_builders_fall_back_to_the_first_usable_candidate() {
        let p = review(&[Codex, Claude]);
        assert_eq!(picked(pick(&p, &[Claude, Codex], |_| true)), (Codex, true));
        let three = review(&[Codex, Claude, Antigravity]);
        assert_eq!(
            picked(pick(&three, &[Claude, Codex], |_| true)),
            (Antigravity, false)
        );
    }

    #[test]
    fn uninstalled_and_signed_out_candidates_are_skipped() {
        let p = review(&[Codex, Claude]);
        // Codex is not installed: the other family is unavailable, so Claude reviews Claude.
        assert_eq!(
            picked(pick(&p, &[Claude], |agent| agent != Codex)),
            (Claude, true)
        );
        // Claude is signed out: Codex reviews Codex.
        assert_eq!(
            picked(pick(&p, &[Codex], |agent| agent != Claude)),
            (Codex, true)
        );
    }

    #[test]
    fn nothing_usable_keeps_the_first_candidate_for_the_spawn_to_report() {
        let p = review(&[Codex, Claude]);
        assert_eq!(picked(pick(&p, &[Claude], |_| false)), (Codex, false));
        assert_eq!(picked(pick(&p, &[Codex], |_| false)), (Codex, true));
    }

    #[test]
    fn fixed_profiles_never_probe_providers() {
        let mut p = review(&[Codex]);
        p.provider_rule = ProviderRule::Fixed;
        p.candidates.clear();
        p.choice.model = "gpt".into();
        let result = pick(&p, &[Codex], |_| {
            panic!("fixed profiles do not probe providers")
        });
        assert_eq!(
            result,
            Pick {
                choice: p.choice.clone(),
                same_family: false
            }
        );
    }

    #[test]
    fn availability_is_checked_lazily_once_per_candidate() {
        let p = review(&[Codex, Claude, Antigravity]);
        let mut calls = vec![];
        pick(&p, &[Claude], |agent| {
            calls.push(agent);
            true
        });
        assert_eq!(calls, [Codex]);
        calls.clear();
        pick(&p, &[Codex, Claude, Antigravity], |agent| {
            calls.push(agent);
            agent == Antigravity
        });
        assert_eq!(calls, [Codex, Claude, Antigravity]);
    }

    #[test]
    fn builders_are_ordinary_tabs_or_the_workspace_default() {
        let mut ws: Workspace = serde_json::from_value(json!({
            "id": "w", "title": "", "repo": "", "repo_name": "", "branch": "",
            "worktree": "", "stage": "", "agent": "codex"
        }))
        .unwrap();
        let tab = |id: &str, agent: Option<ProviderId>, task: bool| -> Tab {
            let mut tab: Tab = serde_json::from_value(json!({
                "id": id, "title": "", "status": "pronta", "note": null, "pending_prompt": null
            }))
            .unwrap();
            tab.choice = agent.map(choice);
            tab.task = task.then(|| {
                serde_json::from_value(json!({
                    "command": "review", "profile": review(&[Claude]), "paused": false,
                    "done": true, "turns": 0, "checked_at": 0, "error": null
                }))
                .unwrap()
            });
            tab
        };
        assert_eq!(
            builders(&ws),
            [Codex],
            "no tabs: the workspace default built it"
        );
        ws.tabs = vec![tab("review", Some(Claude), true)];
        assert_eq!(builders(&ws), [Codex], "an earlier review does not build");
        ws.tabs.push(tab("inherits", None, false));
        ws.tabs.push(tab("override", Some(Claude), false));
        ws.tabs.push(tab("again", Some(Claude), false));
        assert_eq!(builders(&ws), [Codex, Claude]);
    }
}
