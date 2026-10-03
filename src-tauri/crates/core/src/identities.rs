use serde::{Deserialize, Serialize};

/// Map legacy path-based domain identities in board state, never in the telemetry database.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Identities(pub std::collections::BTreeMap<String, String>);
impl Identities {
    pub fn ensure(&mut self, kind: &str, source: &str) -> String {
        self.0
            .entry(format!("{kind}:{source}"))
            .or_insert_with(|| {
                if matches!(kind, "workspace" | "conversation")
                    && uuid::Uuid::parse_str(source).is_ok()
                {
                    source.into()
                } else {
                    uuid::Uuid::new_v4().to_string()
                }
            })
            .clone()
    }
    pub fn get(&self, kind: &str, source: &str) -> Option<String> {
        self.0.get(&format!("{kind}:{source}")).cloned()
    }
}
