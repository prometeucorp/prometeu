//! Provider preparation and canonical protocol connections. Native configuration stays at the edge.
use super::launch::LaunchRequest;
use super::workers::Translate;
use crate::conversation::stream::ConversationInput;

/// Preparation may materialize native account/tool files but must not launch the conversation.
/// The prepared type is local to the execution adapter and is never a bridge payload.
pub trait ProviderPreparation<Prepared>: Send + Sync {
    fn prepare(&self, request: &LaunchRequest) -> Result<Prepared, String>;
}

pub trait AgentInput: ConversationInput + Send {
    /// Reader-owned protocol state may remain alive after input closes.
    fn close(&mut self);
    /// Whether a new message must wait for the current turn and its background work to settle.
    fn requires_idle(&self) -> bool;
}

pub struct AgentProtocol {
    pub input: Box<dyn AgentInput>,
    pub translate: Translate,
}
