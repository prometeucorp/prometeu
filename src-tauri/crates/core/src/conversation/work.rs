//! Settlement of a turn and the background tasks it still owns.

use crate::conversation;
use serde_json::Value;

/// Background tasks observed for one conversation, and the completion they hold back. A provider's
/// native subagents outlive the turn that started them, so a conversation only settles when the
/// turn has ended *and* those tasks have drained. Runtime only: tasks die with the process.
#[derive(Default)]
pub struct Work {
    running: usize,
    /// A turn ended while tasks were still running.
    held: bool,
}

impl Work {
    /// Apply a canonical event. `true` when the conversation settles on it: the turn has ended and
    /// no background task is still running. Both adapters feed this through the same two events.
    pub fn observe(&mut self, event: &Value) -> bool {
        // The main agent answering again invalidates the terminal its children were holding, or a
        // later drain would settle the conversation in the middle of the resumed turn.
        if conversation::agent_activity(event) {
            self.held = false;
            return false;
        }
        match event["type"].as_str() {
            // An interruption ends the turn and the children it started, whether or not the
            // provider reports the drain. Nothing may hold the conversation open afterwards.
            Some("turn.completed") if event["outcome"] == "interrupted" => {
                *self = Self::default();
                true
            }
            Some("turn.completed") => self.ended(),
            Some("background.changed") => {
                self.reported(event["tasks"].as_array().map_or(0, Vec::len))
            }
            _ => false,
        }
    }

    /// The turn ended. `true` when the conversation settles now.
    fn ended(&mut self) -> bool {
        self.held = self.running > 0;
        !self.held
    }

    /// An adapter reported the current tasks. `true` when a held completion settles now.
    fn reported(&mut self, running: usize) -> bool {
        self.running = running;
        let settled = running == 0 && self.held;
        self.held = self.held && !settled;
        settled
    }
}

impl Work {
    /// Background tasks this conversation still owes, independently of its main turn.
    pub fn busy(&self) -> bool {
        self.running > 0
    }
}

#[cfg(test)]
mod work_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_turn_settles_only_after_its_background_tasks_drain() {
        let mut work = Work::default();
        assert!(!work.reported(1));
        assert!(!work.ended());
        assert!(!work.reported(2));
        assert!(!work.reported(1));
        assert!(work.reported(0));
        // The drain is consumed: a repeated empty report does not settle a second time.
        assert!(!work.reported(0));
    }

    #[test]
    fn a_turn_without_background_tasks_settles_immediately() {
        let mut work = Work::default();
        assert!(work.ended());
        assert!(!work.reported(0));
        // A task started by a later continuation settles with that continuation's own terminal.
        assert!(!work.reported(1));
        assert!(!work.reported(0));
        assert!(work.ended());
    }

    #[test]
    fn an_interruption_settles_without_waiting_for_a_reported_drain() {
        let mut work = Work::default();
        assert!(!work.observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]})));
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"interrupted"})));
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
    }

    #[test]
    fn a_resumed_turn_drops_the_completion_its_tasks_were_holding() {
        let mut work = Work::default();
        assert!(!work.observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]})));
        assert!(!work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
        // The main agent answers again before the child finishes.
        assert!(!work.observe(&json!({"type":"assistant.started","messageId":"m"})));
        assert!(
            !work.observe(&json!({"type":"background.changed","tasks":[]})),
            "a drain must not settle a resumed turn"
        );
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
    }

    #[test]
    fn a_conversation_owes_work_while_its_tasks_run() {
        let mut work = Work::default();
        assert!(!work.busy());
        work.observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]}));
        assert!(work.busy());
        work.observe(&json!({"type":"turn.completed","outcome":"ok"}));
        assert!(work.busy(), "a held completion still owes its children");
        work.observe(&json!({"type":"background.changed","tasks":[]}));
        assert!(!work.busy());
    }

    #[test]
    fn only_the_turn_and_its_tasks_move_the_conversation() {
        let mut work = Work::default();
        for event in [
            json!({"type":"assistant.block","block":{"kind":"text"}}),
            json!({"type":"request.opened","requestId":"q"}),
            json!({"type":"context.updated","used":10}),
        ] {
            assert!(!work.observe(&event));
        }
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
    }

    #[test]
    fn draining_without_a_finished_turn_does_not_settle() {
        let mut work = Work::default();
        assert!(!work.reported(1));
        assert!(!work.reported(0));
        assert!(work.ended());
    }
}
