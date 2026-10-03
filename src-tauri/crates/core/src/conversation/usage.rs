//! Canonical usage measurements shared by provider adapters and telemetry storage.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub context_used: Option<u64>,
    pub context_window: Option<u64>,
    pub peak_context: Option<u64>,
    pub model_calls: Option<u64>,
    pub compactions: Option<u64>,
    pub cache_rebuilds: Option<u64>,
    pub cost_usd: Option<f64>,
}
impl Usage {
    pub fn valid(&self) -> bool {
        self.cost_usd.is_none_or(|n| n.is_finite() && n >= 0.)
            && [
                (self.cache_read_tokens, self.input_tokens),
                (self.cache_write_tokens, self.input_tokens),
                (self.reasoning_tokens, self.output_tokens),
            ]
            .iter()
            .all(|(part, total)| match (part, total) {
                (Some(p), Some(t)) => p <= t,
                _ => true,
            })
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Measurement {
    /// Usage covers the observed main agent, excluding independently running children.
    pub usage_scope: UsageScope,
    /// Both turn totals are complete; known partial sums remain useful when false.
    #[serde(default)]
    pub complete: bool,
    pub selected_model: Option<String>,
    pub observed_models: Option<Vec<String>>,
    pub usage: Usage,
    pub usage_by_model: Option<Vec<ModelUsage>>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum UsageScope {
    #[default]
    MainAgent,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsage {
    pub model: String,
    pub usage: Usage,
}
