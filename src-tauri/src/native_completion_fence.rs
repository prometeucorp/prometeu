//! Provider-specific persistence evidence for successful native terminal turns.
//! No clock, file, rendered text or canonical message id establishes this fence.
use crate::{
    state::ProviderId,
    terminal_agent_hooks::{NativeHookEvent, NativeHookKind},
};
use prometeu_protocols::{
    terminal_status::{ProgramStatusReport, ProgramStatusState},
    terminal_transcript::{TranscriptPhase, TranscriptRecord},
};
use std::collections::HashSet;

const SOURCE_LIMIT: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceWatermark {
    /// The authenticated append stream's revision and EOF captured before native acceptance.
    Claude { revision: String, next_offset: u64 },
    /// The next trajectory index after the accepted native user step. Rewrites retain indices.
    Antigravity { next_step_index: u64 },
}

#[derive(Clone, Copy)]
pub struct PollToken {
    generation: u64,
    sequence: u64,
}

/// Private proof only. The coordinator must still commit its own current completion ticket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedCompletion {
    pub generation: u64,
    pub native_source: String,
}

struct Assistant {
    source: String,
    text: String,
}

struct Turn {
    generation: u64,
    native_session: String,
    watermark: SourceWatermark,
    working: bool,
    done_at: Option<u64>,
    stop_at: Option<u64>,
    stop_text: Option<String>,
    stop_proven: bool,
    invocation_started: bool,
    stops: usize,
    last_stop_execution: Option<u64>,
    sources: HashSet<String>,
    last_position: Option<u64>,
    assistant: Option<Assistant>,
    poll_ready: bool,
}

pub struct NativeCompletionFence {
    provider: ProviderId,
    process_nonce: String,
    sequence: u64,
    turn: Option<Turn>,
}

impl NativeCompletionFence {
    /// Claude's managed invocation must also enable CLAUDE_CODE_EAGER_FLUSH=1. Its installed
    /// engine awaits the local write queue before yielding its final result to the PSP publisher.
    /// Antigravity uses the persisted exact final step instead of assuming a global flush order.
    pub fn verified(provider: ProviderId, version: &str, process_nonce: &str) -> Option<Self> {
        let supported = match provider {
            ProviderId::Claude => prometeu_protocols::cli_version::at_least(version, "2.1.326"),
            ProviderId::Antigravity => prometeu_protocols::cli_version::at_least(version, "1.5.10"),
            _ => false,
        };
        (supported && !process_nonce.is_empty()).then(|| Self {
            provider,
            process_nonce: process_nonce.into(),
            sequence: 0,
            turn: None,
        })
    }

    /// Acceptance is scoped to one native submission, including locally typed submissions.
    /// A guessed source watermark is not valid; replay must finish before this call.
    pub fn begin(
        &mut self,
        generation: u64,
        native_session: &str,
        watermark: SourceWatermark,
    ) -> bool {
        self.turn = None;
        if native_session.is_empty()
            || !matches!(
                (self.provider, &watermark),
                (ProviderId::Claude, SourceWatermark::Claude { revision, .. }) if !revision.is_empty()
            ) && !matches!(
                (self.provider, &watermark),
                (ProviderId::Antigravity, SourceWatermark::Antigravity { .. })
            )
        {
            return false;
        }
        self.turn = Some(Turn {
            generation,
            native_session: native_session.into(),
            watermark,
            working: false,
            done_at: None,
            stop_at: None,
            stop_text: None,
            stop_proven: false,
            invocation_started: false,
            stops: 0,
            last_stop_execution: None,
            sources: HashSet::new(),
            last_position: None,
            assistant: None,
            poll_ready: false,
        });
        true
    }

    /// Cancellation, source replacement/loss and retirement invalidate success permanently for
    /// this generation. A later accepted submission establishes a new fence with begin().
    pub fn invalidate(&mut self) {
        self.turn = None;
    }

    /// A rewritten snapshot must contain the candidate again. Retain its highest source position
    /// so truncation cannot fall back to an earlier identical response.
    pub fn refresh_persisted_source(&mut self) {
        if let Some(turn) = self.turn.as_mut() {
            turn.assistant = None;
            turn.poll_ready = false;
        }
    }

    pub fn observe_psp(&mut self, generation: u64, report: &ProgramStatusReport) {
        if report.id.is_some() {
            return;
        }
        let sequence = self.tick();
        let Some(turn) = self
            .turn
            .as_mut()
            .filter(|turn| turn.generation == generation)
        else {
            return;
        };
        turn.poll_ready = false;
        match report.state {
            ProgramStatusState::Working | ProgramStatusState::Blocked => {
                turn.working = true;
                turn.done_at = None;
            }
            ProgramStatusState::Done if turn.working => {
                turn.done_at = Some(sequence);
            }
            ProgramStatusState::Error | ProgramStatusState::Clear => self.invalidate(),
            ProgramStatusState::Idle if turn.working => self.invalidate(),
            _ => {}
        }
    }

    /// The caller drains authenticated root hooks in their bridge delivery order. A hook remains
    /// observational: only a subsequent native root Done can arbitrate other hooks' decisions.
    pub fn observe_hook(&mut self, generation: u64, event: &NativeHookEvent) {
        if event.provider != self.provider
            || event.process_nonce != self.process_nonce
            || event.agent_id.is_some()
        {
            return;
        }
        let sequence = self.tick();
        let Some(turn) = self.turn.as_mut().filter(|turn| {
            turn.generation == generation && turn.native_session == event.native_session
        }) else {
            return;
        };
        match event.kind {
            NativeHookKind::TurnFailed | NativeHookKind::SessionEnded => self.invalidate(),
            NativeHookKind::ModelInvocationStarted => {
                if let SourceWatermark::Antigravity { next_step_index } = &mut turn.watermark {
                    let Some(initial) = event.initial_num_steps else {
                        self.invalidate();
                        return;
                    };
                    *next_step_index = (*next_step_index).max(initial);
                    turn.invocation_started = true;
                    turn.sources.clear();
                    turn.assistant = None;
                    turn.last_position = None;
                    turn.stops = 0;
                    turn.last_stop_execution = None;
                }
                clear_stop(turn);
            }
            NativeHookKind::PromptSubmitted => {
                turn.stops = 0;
                turn.last_stop_execution = None;
                clear_stop(turn);
            }
            NativeHookKind::ToolStarted => clear_stop(turn),
            NativeHookKind::StopObserved => {
                if self.provider == ProviderId::Antigravity {
                    let Some(execution) = event.execution_number else {
                        self.invalidate();
                        return;
                    };
                    if turn
                        .last_stop_execution
                        .is_some_and(|previous| execution < previous)
                    {
                        return;
                    }
                    if turn.last_stop_execution != Some(execution) {
                        turn.stops += 1;
                        turn.last_stop_execution = Some(execution);
                    }
                } else {
                    turn.stops += 1;
                }
                turn.poll_ready = false;
                turn.stop_at = Some(sequence);
                turn.stop_text = event.text.clone().filter(|text| !text.is_empty());
                turn.stop_proven = if self.provider == ProviderId::Antigravity {
                    turn.invocation_started
                        && event.fully_idle == Some(true)
                        && event.reason.as_deref() == Some("NO_TOOL_CALL")
                        && event
                            .error
                            .as_ref()
                            .is_none_or(|value| value.is_null() || value.as_str() == Some(""))
                } else {
                    turn.stop_text.is_some()
                };
                // stop_hook_active/continuationActive describe earlier continuation. They do
                // not report the result of the other hooks deciding this stop.
            }
            _ => {}
        }
    }

    /// Raw authenticated source metadata must be retained before conversion to canonical frames.
    /// Claude requires live append provenance. An Antigravity snapshot can be classified as replay
    /// after rewriting; only its native step index beyond this submission's watermark is eligible.
    pub fn observe_record(&mut self, generation: u64, revision: &str, record: &TranscriptRecord) {
        let Some(turn) = self
            .turn
            .as_mut()
            .filter(|turn| turn.generation == generation)
        else {
            return;
        };
        turn.poll_ready = false;
        if self.provider == ProviderId::Claude && record.phase != TranscriptPhase::Live {
            return;
        }
        let row = &record.value;
        let (source, response, text) = match &turn.watermark {
            SourceWatermark::Claude {
                revision: expected,
                next_offset,
            } => {
                if revision != expected {
                    self.invalidate();
                    return;
                }
                if record.offset < *next_offset
                    || row["type"] != "assistant"
                    || row["sessionId"].as_str() != Some(turn.native_session.as_str())
                    || row["isSidechain"] == true
                    || row.get("agentId").is_some_and(|value| !value.is_null())
                {
                    return;
                }
                if turn.last_position.is_some_and(|last| record.offset < last) {
                    return;
                }
                turn.last_position = Some(record.offset);
                let Some(uuid) = row["uuid"].as_str().filter(|uuid| !uuid.is_empty()) else {
                    turn.assistant = None;
                    return;
                };
                // One API response can be persisted as several blocks with different UUIDs.
                // A continued Stop must observe another response, not merely another block.
                let Some(response) = row["message"]["id"].as_str().filter(|id| !id.is_empty())
                else {
                    turn.assistant = None;
                    return;
                };
                if row["message"]["stop_reason"] != "end_turn" {
                    turn.assistant = None;
                    return;
                }
                let Some(content) = row["message"]["content"].as_array() else {
                    turn.assistant = None;
                    return;
                };
                let text = content
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .map(|block| block["text"].as_str())
                    .collect::<Option<Vec<_>>>();
                let Some(text) = text else {
                    turn.assistant = None;
                    return;
                };
                (
                    uuid.to_owned(),
                    response.to_owned(),
                    text.join("\n").trim_matches(js_whitespace).to_owned(),
                )
            }
            SourceWatermark::Antigravity { next_step_index } => {
                let Some(index) = row["step_index"]
                    .as_u64()
                    .filter(|index| index == next_step_index && turn.invocation_started)
                else {
                    return;
                };
                if row["source"] != "MODEL" || row["type"] != "PLANNER_RESPONSE" {
                    return;
                }
                if turn.last_position.is_some_and(|last| index < last) {
                    return;
                }
                turn.last_position = Some(index);
                if row["status"] != "DONE"
                    || row["truncated_fields"]
                        .as_array()
                        .is_some_and(|fields| !fields.is_empty())
                    || row["tool_calls"]
                        .as_array()
                        .is_some_and(|calls| !calls.is_empty())
                {
                    turn.assistant = None;
                    return;
                }
                let Some(text) = row["content"].as_str() else {
                    turn.assistant = None;
                    return;
                };
                let source = format!("step:{index}");
                (source.clone(), source, text.to_owned())
            }
        };
        if turn.sources.len() >= SOURCE_LIMIT && !turn.sources.contains(&response) {
            self.invalidate();
            return;
        }
        turn.sources.insert(response);
        turn.assistant = Some(Assistant { source, text });
    }

    pub fn start_poll(&mut self, generation: u64) -> PollToken {
        PollToken {
            generation,
            sequence: self.tick(),
        }
    }

    pub fn finish_poll(&mut self, token: PollToken, caught_up: bool) {
        let Some(turn) = self
            .turn
            .as_mut()
            .filter(|turn| turn.generation == token.generation)
        else {
            return;
        };
        turn.poll_ready = caught_up
            && turn.done_at.is_some_and(|done| token.sequence > done)
            && turn.stop_at.is_some_and(|stop| token.sequence > stop);
    }

    pub fn take_completion(&mut self, generation: u64) -> Option<PersistedCompletion> {
        let turn = self.turn.as_ref()?;
        if turn.generation != generation
            || !turn.poll_ready
            || !turn.stop_proven
            || turn.sources.len() < turn.stops
        {
            return None;
        }
        let assistant = turn.assistant.as_ref()?;
        if turn
            .stop_text
            .as_deref()
            .is_some_and(|text| text != assistant.text)
        {
            return None;
        }
        let proof = PersistedCompletion {
            generation,
            native_source: assistant.source.clone(),
        };
        self.turn = None;
        Some(proof)
    }

    fn tick(&mut self) -> u64 {
        let Some(next) = self.sequence.checked_add(1) else {
            self.invalidate();
            return self.sequence;
        };
        self.sequence = next;
        next
    }
}

fn clear_stop(turn: &mut Turn) {
    turn.stop_at = None;
    turn.stop_text = None;
    turn.stop_proven = false;
    turn.poll_ready = false;
}

fn js_whitespace(ch: char) -> bool {
    matches!(ch, '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

#[cfg(test)]
mod tests;
