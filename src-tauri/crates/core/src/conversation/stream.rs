//! Ordered conversation input, replay and publication, independent of its execution host.

use super::{event, Clock};
use serde_json::{json, Value};

/// A store scoped to one conversation. Paths and provider-owned storage stay at the edge.
pub trait TranscriptStore: Send + Sync {
    fn load(&self) -> Result<String, String>;
    fn append(&self, line: &str) -> Result<(), String>;
}

/// Live delivery scoped to one conversation. Do not reenter its ordering lock from this port.
pub trait ConversationEvents: Send + Sync {
    fn emit(&self, text: &str, seq: u64) -> Result<(), String>;
}

impl<F> ConversationEvents for F
where
    F: Fn(&str, u64) -> Result<(), String> + Send + Sync,
{
    fn emit(&self, text: &str, seq: u64) -> Result<(), String> {
        self(text, seq)
    }
}

/// Accepted canonical commands may produce local echoes before the provider's next output.
/// An error means no local acceptance events are recorded; this port never retries a command.
pub trait ConversationInput {
    fn send(&mut self, command: &Value, transcript: &str) -> Result<Vec<Value>, String>;
}

impl<F> ConversationInput for F
where
    F: FnMut(&Value, &str) -> Result<Vec<Value>, String>,
{
    fn send(&mut self, command: &Value, transcript: &str) -> Result<Vec<Value>, String> {
        self(command, transcript)
    }
}

/// Both effects are attempted once. Failed persistence must not stop live conversation output.
#[derive(Debug, PartialEq)]
pub struct Delivery {
    pub seq: u64,
    pub storage_error: Option<String>,
    pub event_error: Option<String>,
}

/// Per-conversation memory ceiling. Persistent history is never trimmed by this buffer.
const KEEP: usize = 4 * 1024 * 1024;

/// The host holds one mutex across command acceptance, recording, delivery and snapshot capture.
#[derive(Default)]
pub struct Lines {
    pub text: String,
    pub seq: u64,
}

impl Lines {
    pub fn seeded(store: &dyn TranscriptStore) -> Result<Self, String> {
        let mut text = store.load()?;
        trim(&mut text);
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(Self { text, seq: 0 })
    }

    /// Internal capture fields never enter replay or public delivery; canonical turn usage does.
    /// The caller retains the original frame for private capture and reactions after releasing the stream lock.
    pub fn record(
        &mut self,
        text: &str,
        frame: &Value,
        store: &dyn TranscriptStore,
        events: &dyn ConversationEvents,
    ) -> Option<Delivery> {
        if frame["type"] == "telemetry.usage" {
            return None;
        }
        Some(self.deliver(&public_text(text, frame), keep(frame), store, events))
    }

    pub fn deliver(
        &mut self,
        text: &str,
        persist: bool,
        store: &dyn TranscriptStore,
        events: &dyn ConversationEvents,
    ) -> Delivery {
        let storage_error = persist.then(|| store.append(text).err()).flatten();
        let seq = match persist {
            true => self.absorb(text),
            false => self.skip(),
        };
        let event_error = events.emit(text, seq).err();
        Delivery {
            seq,
            storage_error,
            event_error,
        }
    }

    /// The record hook lets the host project ordered execution observations before delivery.
    /// It must use this same buffer and must not acquire the host's command lock again.
    pub fn command(
        &mut self,
        command: &Value,
        clock: &dyn Clock,
        input: &mut dyn ConversationInput,
        mut record: impl FnMut(&mut Self, &Value),
    ) -> Result<Vec<Value>, String> {
        let echo = input.send(command, &self.text)?;
        let started = (command["type"] == "message.send")
            .then(|| event("session.state", clock.now(), json!({ "state": "busy" })));
        let accepted = match command["type"].as_str() {
            Some("message.send") => command["text"].as_str().map(|text| {
                event(
                    "user.message",
                    clock.now(),
                    json!({ "content": [{ "kind": "text", "text": text }] }),
                )
            }),
            _ => closed_request(command, clock),
        };
        let events: Vec<Value> = started.into_iter().chain(accepted).chain(echo).collect();
        for event in &events {
            record(self, event);
        }
        Ok(events)
    }

    pub fn absorb(&mut self, line: &str) -> u64 {
        self.text.push_str(line);
        self.text.push('\n');
        trim(&mut self.text);
        self.skip()
    }

    pub fn skip(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Synthetic runtime state participates in replay, but does not consume a live sequence.
    pub fn snapshot(&self, busy: bool, clock: &dyn Clock) -> Snapshot {
        let mut text = self.text.clone();
        text.push_str(
            &event(
                "session.state",
                clock.now(),
                json!({
                    "state": if busy { "busy" } else { "ready" }
                }),
            )
            .to_string(),
        );
        text.push('\n');
        Snapshot {
            text,
            seq: self.seq,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Snapshot {
    pub text: String,
    pub seq: u64,
}

fn trim(text: &mut String) {
    if text.len() > KEEP {
        let cut = text.len() - KEEP;
        let at = text.as_bytes()[cut..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(text.len(), |i| cut + i + 1);
        text.drain(..at);
    }
}

fn closed_request(command: &Value, clock: &dyn Clock) -> Option<Value> {
    (command["v"] == 1 && command["type"] == "request.respond").then(|| {
        let outcome = match command["response"]["outcome"].as_str() {
            Some("allow") => "allowed",
            Some("deny") => "denied",
            Some("answer") => "answered",
            _ => "cancelled",
        };
        event(
            "request.closed",
            clock.now(),
            json!({ "requestId": command["requestId"], "outcome": outcome }),
        )
    })
}

pub fn public_text<'a>(text: &'a str, frame: &Value) -> std::borrow::Cow<'a, str> {
    if frame.get("telemetry").is_none() && frame.get("providerDurationMs").is_none() {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut public = frame.clone();
    if let Some(fields) = public.as_object_mut() {
        fields.remove("telemetry");
        fields.remove("providerDurationMs");
    }
    std::borrow::Cow::Owned(public.to_string())
}

fn keep(frame: &Value) -> bool {
    !matches!(
        frame["type"].as_str(),
        Some(
            "assistant.started"
                | "assistant.block.started"
                | "assistant.delta"
                | "tool.input.delta"
                | "context.compaction"
                | "context.updated"
                | "session.state"
                | "session.identity"
                | "commands.updated"
                | "usage.updated"
                | "telemetry.usage"
        )
    )
}

#[cfg(test)]
mod tests;
