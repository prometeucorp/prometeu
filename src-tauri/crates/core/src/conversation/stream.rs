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

/// Result of a host-owned durable append under the conversation's capture and stream locks.
/// A recovered occurrence is already in the seeded transcript and must not be published again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Appended,
    AlreadyCommitted,
}

/// Per-conversation memory ceiling. Persistent history is never trimmed by this buffer.
const KEEP: usize = 4 * 1024 * 1024;

/// The host holds one mutex across command acceptance, recording, delivery and snapshot capture.
#[derive(Default)]
pub struct Lines {
    pub text: String,
    pub seq: u64,
    /// Transcript lines trimmed from the head of `text`, so line positions stay absolute.
    pub first: u64,
}

impl Lines {
    pub fn seeded(store: &dyn TranscriptStore) -> Result<Self, String> {
        let mut text = store.load()?;
        let first = trim(&mut text);
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(Self {
            text,
            seq: 0,
            first,
        })
    }

    /// Transcript lines recorded so far, counted from the start of the transcript.
    pub fn end(&self) -> u64 {
        self.first + lines_in(&self.text)
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
        self.publish(text, persist, storage_error, events)
    }

    /// The caller has already committed the sanitized public line while holding this buffer.
    /// This preserves ordinary sequence/retention rules without appending that line twice.
    pub(crate) fn record_committed(
        &mut self,
        text: &str,
        frame: &Value,
        events: &dyn ConversationEvents,
    ) -> Option<Delivery> {
        if frame["type"] == "telemetry.usage" {
            return None;
        }
        Some(self.publish(&public_text(text, frame), keep(frame), None, events))
    }

    fn publish(
        &mut self,
        text: &str,
        persist: bool,
        storage_error: Option<String>,
        events: &dyn ConversationEvents,
    ) -> Delivery {
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
        self.first += trim(&mut self.text);
        self.skip()
    }

    pub fn skip(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Synthetic runtime state participates in replay, but does not consume a live sequence.
    /// `cuts` are transcript positions where the app ended a turn; replay stops there too, so a
    /// later continuation does not turn the cut reply into a finished one.
    pub fn snapshot(&self, presence: Presence, cuts: &[u64], clock: &dyn Clock) -> Snapshot {
        let state = match presence {
            Presence::Busy => "busy",
            Presence::Ready => "ready",
            Presence::Stopped => "stopped",
        };
        let mut text = match cuts.is_empty() {
            true => self.text.clone(),
            false => {
                let stopped = event("session.state", clock.now(), json!({ "state": "stopped" }));
                let mut text = String::with_capacity(self.text.len());
                for (at, line) in (self.first + 1..).zip(self.text.split_inclusive('\n')) {
                    text.push_str(line);
                    if cuts.contains(&at) {
                        text.push_str(&stopped.to_string());
                        text.push('\n');
                    }
                }
                text
            }
        };
        text.push_str(&event("session.state", clock.now(), json!({ "state": state })).to_string());
        text.push('\n');
        Snapshot {
            text,
            seq: self.seq,
        }
    }
}

/// The process behind a snapshot, which replay presents as the conversation's last state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Presence {
    /// A turn is running.
    Busy,
    /// The process waits for the next message.
    Ready,
    /// No process: a turn the transcript left unfinished was cut off with it.
    Stopped,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Snapshot {
    pub text: String,
    pub seq: u64,
}

/// Drop whole lines from the head beyond the memory ceiling and return how many were dropped.
fn trim(text: &mut String) -> u64 {
    if text.len() <= KEEP {
        return 0;
    }
    let cut = text.len() - KEEP;
    let at = text.as_bytes()[cut..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(text.len(), |i| cut + i + 1);
    let dropped = lines_in(&text[..at]);
    text.drain(..at);
    dropped
}

fn lines_in(text: &str) -> u64 {
    text.bytes().filter(|&b| b == b'\n').count() as u64
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

/// Whether this canonical event belongs in retained public history. Native source receipts may
/// still be required for events excluded here, without retaining their private payloads.
pub fn persistent(frame: &Value) -> bool {
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
                | "execution.observed"
                | "session.identity"
                | "commands.updated"
                | "usage.updated"
                | "telemetry.usage"
        )
    )
}

pub(crate) use persistent as keep;

#[cfg(test)]
mod tests;
