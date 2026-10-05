//! Persisted action definitions and frozen execution profiles.
use crate::board::Choice;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Catalog {
    #[serde(default)]
    pub defaults_initialized: bool,
    pub profiles: Vec<Profile>,
    pub commands: Vec<Action>,
    /// Project overrides replace the whole profile; absence inherits the global profile.
    #[serde(default)]
    pub overrides: BTreeMap<String, BTreeMap<String, Profile>>,
    #[serde(default)]
    pub pr_action: Option<String>,
}

impl Catalog {
    /// Initialize once so removing or customizing the profile survives subsequent startup.
    pub fn initialize_defaults(&mut self) {
        if self.defaults_initialized {
            return;
        }
        #[derive(Deserialize)]
        struct Seed {
            profile: Profile,
            command: Action,
        }
        let seed: Seed = serde_json::from_str(include_str!("../../../../src/action-defaults.json"))
            .expect("valid bundled action defaults");
        if !self.profiles.iter().any(|p| p.id == seed.profile.id)
            && !self.commands.iter().any(|c| c.name == seed.command.name)
        {
            self.profiles.push(seed.profile);
            self.commands.push(seed.command);
        }
        self.defaults_initialized = true;
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    #[default]
    Ask,
    Auto,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub choice: Choice,
    pub mcp: Option<Vec<String>>,
    pub plugins: Option<Vec<String>>,
    pub skills: Vec<String>,
    pub permission: Permission,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Prompt,
    Agent,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Action {
    pub name: String,
    pub description: String,
    pub kind: Kind,
    pub prompt: String,
    pub profile: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Run {
    pub command: String,
    pub profile: Profile,
    pub paused: bool,
    pub done: bool,
    /// Unused since PR monitoring was removed; still written so earlier versions can read the board.
    #[serde(default)]
    pub turns: u32,
    #[serde(default)]
    pub checked_at: u64,
    pub error: Option<String>,
}
