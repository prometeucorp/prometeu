//! Bridge validated native hooks, source user boundaries and explicit PSP into the pure fence.
use super::*;
use crate::native_completion_fence::{
    NativeCompletionFence, PersistedCompletion, PollToken, SourceWatermark,
};
use prometeu_protocols::{
    terminal_status::{ProgramStatusReport, ProgramStatusState},
    terminal_transcript::TranscriptRecord,
};

const HOOK_LIMIT: usize = 64;
const HOOK_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct CompletionObservation {
    provider: ProviderId,
    fence: NativeCompletionFence,
    native_session: Option<String>,
    revision: String,
    baseline_ready: bool,
    generation: u64,
    pending_prompt: Option<String>,
    pending_hook_seen: bool,
    current_prompt: Option<String>,
    current_hook_seen: bool,
    seen_user: Option<u64>,
    unbound_user: Option<TranscriptRecord>,
    unbound_assistant: Option<TranscriptRecord>,
    hooks: VecDeque<NativeHookEvent>,
    hook_bytes: usize,
    reports: VecDeque<ProgramStatusReport>,
    report_sequence: u64,
    done_report: Option<u64>,
    failed: bool,
}

impl CompletionObservation {
    pub(super) fn new(config: &ObservationConfig, version: &str, revision: &str) -> Option<Self> {
        Some(Self {
            provider: config.provider,
            fence: NativeCompletionFence::verified(
                config.provider,
                version,
                &config.process_nonce,
            )?,
            native_session: config.expected_native_session.clone(),
            revision: revision.into(),
            baseline_ready: config.start == ObservationStart::Fresh,
            generation: 0,
            pending_prompt: None,
            pending_hook_seen: false,
            current_prompt: None,
            current_hook_seen: false,
            seen_user: None,
            unbound_user: None,
            unbound_assistant: None,
            hooks: VecDeque::new(),
            hook_bytes: 0,
            reports: VecDeque::new(),
            report_sequence: 0,
            done_report: None,
            failed: false,
        })
    }

    pub(super) fn expect_submission(&mut self, text: &str) {
        self.start_pending(text.to_owned());
        self.hooks.clear();
        self.hook_bytes = 0;
        self.reports.clear();
        self.done_report = None;
    }

    pub(super) fn hook(&mut self, event: &NativeHookEvent) {
        if event.agent_id.is_some() || self.failed {
            return;
        }
        self.native_session
            .get_or_insert_with(|| event.native_session.clone());
        if event.kind == NativeHookKind::PromptSubmitted {
            let Some(text) = event.prompt.as_deref() else {
                return;
            };
            if self.pending_prompt.as_deref() == Some(text) {
                if !self.pending_hook_seen {
                    self.hooks.clear();
                    self.hook_bytes = 0;
                }
                self.pending_hook_seen = true;
            } else if self.current_prompt.as_deref() == Some(text) && !self.current_hook_seen {
                self.current_hook_seen = true;
            } else {
                self.start_pending(text.to_owned());
                self.pending_hook_seen = true;
                self.hooks.clear();
                self.hook_bytes = 0;
            }
        }
        self.fence.observe_hook(self.generation, event);
        if !matches!(
            event.kind,
            NativeHookKind::PromptSubmitted
                | NativeHookKind::ModelInvocationStarted
                | NativeHookKind::StopObserved
                | NativeHookKind::ToolStarted
                | NativeHookKind::TurnFailed
                | NativeHookKind::SessionEnded
        ) {
            return;
        }
        let bytes = event.text.as_ref().map_or(0, String::len)
            + event.prompt.as_ref().map_or(0, String::len);
        if self.hooks.len() >= HOOK_LIMIT || bytes > HOOK_BYTES.saturating_sub(self.hook_bytes) {
            self.fail();
            return;
        }
        let mut retained = event.clone();
        retained.tool_input = None;
        retained.tool_response = None;
        self.hook_bytes += bytes;
        self.hooks.push_back(retained);
        if event.kind == NativeHookKind::ModelInvocationStarted {
            if let Some(record) = self.unbound_user.take() {
                let assistant = self.unbound_assistant.take();
                self.record(&record);
                if let Some(record) = assistant {
                    self.record(&record);
                }
            }
        }
    }

    pub(super) fn program_status(&mut self, report: &ProgramStatusReport) -> Option<u64> {
        if report.id.is_some() || self.failed {
            return None;
        }
        let sequence = self.report_sequence.checked_add(1)?;
        self.report_sequence = sequence;
        if report.state != ProgramStatusState::Done {
            self.done_report = None;
        } else if self
            .reports
            .back()
            .is_none_or(|last| last.state != ProgramStatusState::Done)
        {
            self.done_report = Some(sequence);
        }
        self.fence.observe_psp(self.generation, report);
        // Only root state participates in this fence. UI text is neither needed nor retained.
        if self
            .reports
            .back()
            .is_some_and(|last| last.state == report.state)
        {
            return Some(sequence);
        }
        if self.reports.len() >= HOOK_LIMIT {
            self.fail();
            return None;
        }
        self.reports.push_back(ProgramStatusReport {
            id: None,
            state: report.state,
            kind: None,
            message: None,
            app: None,
            title: None,
            progress: None,
        });
        Some(sequence)
    }

    pub(super) fn record(&mut self, record: &TranscriptRecord) {
        if self.failed {
            return;
        }
        if self.provider == ProviderId::Antigravity
            && self.unbound_user.is_some()
            && record.value["source"] == "MODEL"
            && record.value["type"] == "PLANNER_RESPONSE"
        {
            // A filesystem poll can outrun hook-channel delivery. Retain one bounded final
            // source row until the exact pre-invocation index binds its preceding user step.
            self.unbound_assistant = Some(record.clone());
            return;
        }
        if let Some((position, mut text)) = user_boundary(self.provider, record) {
            let new = self.seen_user.is_none_or(|last| position > last);
            if !new
                || !self.baseline_ready
                || self.provider == ProviderId::Claude && record.phase != TranscriptPhase::Live
            {
                self.seen_user = Some(self.seen_user.map_or(position, |last| last.max(position)));
                return;
            }
            if self.provider == ProviderId::Antigravity
                && (self
                    .pending_prompt
                    .as_deref()
                    .is_some_and(|prompt| prompt != text)
                    || self.pending_prompt.is_none() && text.starts_with("<USER_REQUEST>\n"))
            {
                // The JSONL user string is a model-facing rendering with native context tags.
                // GetLastUserInput supplies the original protobuf text. Bind it only when that
                // authenticated invocation's trajectory ends at this exact USER_INPUT step.
                let prompt = self.hooks.iter().rev().find_map(|event| {
                    (event.kind == NativeHookKind::ModelInvocationStarted
                        && event.initial_num_steps == position.checked_add(1))
                    .then_some(event.prompt.as_ref())
                    .flatten()
                });
                let Some(prompt) = prompt else {
                    self.unbound_user = Some(record.clone());
                    return;
                };
                text = prompt.clone();
            }
            self.seen_user = Some(self.seen_user.map_or(position, |last| last.max(position)));
            self.unbound_user = None;
            if self
                .pending_prompt
                .as_deref()
                .is_some_and(|expected| expected != text)
            {
                // A source user boundary that does not match the next accepted submission must
                // not consume its completion evidence, even when a later response is identical.
                self.fail();
                return;
            }
            if self.pending_prompt.is_none() {
                self.start_pending(text.clone());
            }
            let Some(native) = self.native_session.as_deref() else {
                return;
            };
            let Some(next) = position.checked_add(1) else {
                self.fail();
                return;
            };
            let watermark = match self.provider {
                ProviderId::Claude => SourceWatermark::Claude {
                    revision: self.revision.clone(),
                    next_offset: next,
                },
                ProviderId::Antigravity => SourceWatermark::Antigravity {
                    next_step_index: next,
                },
                _ => return,
            };
            if !self.fence.begin(self.generation, native, watermark) {
                self.fail();
                return;
            }
            self.pending_prompt = None;
            self.current_prompt = Some(text);
            self.current_hook_seen = self.pending_hook_seen;
            self.pending_hook_seen = false;
            // AGY may deliver its model/stop hooks before this snapshot is read. Discard any
            // earlier execution's buffered hooks using its actual pre-invocation step count.
            if self.provider == ProviderId::Antigravity {
                let start = self.hooks.iter().position(|event| {
                    event.kind == NativeHookKind::ModelInvocationStarted
                        && event
                            .initial_num_steps
                            .is_some_and(|count| count > position)
                });
                if let Some(start) = start {
                    self.hooks.drain(..start);
                } else {
                    self.hooks.clear();
                }
            }
            for event in self.hooks.drain(..) {
                self.fence.observe_hook(self.generation, &event);
            }
            self.hook_bytes = 0;
            // Trust/auth setup can precede the first user boundary. Its completed/cleared
            // activity is not part of the model turn; only the latest native activity remains.
            let after_reset = self
                .reports
                .iter()
                .rposition(|report| {
                    matches!(
                        report.state,
                        ProgramStatusState::Idle
                            | ProgramStatusState::Clear
                            | ProgramStatusState::Error
                    )
                })
                .map_or(0, |index| index + 1);
            for report in self.reports.iter().skip(after_reset) {
                self.fence.observe_psp(self.generation, report);
            }
        }
        self.fence
            .observe_record(self.generation, &self.revision, record);
    }

    pub(super) fn refresh_snapshot(&mut self) {
        self.fence.refresh_persisted_source();
    }

    pub(super) fn source_replaced(&mut self, revision: &str) {
        if self.provider == ProviderId::Claude {
            self.fence.invalidate();
            self.pending_prompt = None;
            self.current_prompt = None;
            self.seen_user = None;
            self.hooks.clear();
            self.reports.clear();
            self.hook_bytes = 0;
        }
        self.revision = revision.into();
    }

    pub(super) fn start_poll(&mut self) -> PollToken {
        self.fence.start_poll(self.generation)
    }
    pub(super) fn finish_poll(&mut self, token: PollToken, caught_up: bool, baseline_ready: bool) {
        self.baseline_ready = baseline_ready;
        self.fence.finish_poll(token, caught_up);
    }
    pub(super) fn take(&mut self) -> Option<(u64, PersistedCompletion)> {
        let report = self.done_report?;
        (!self.failed)
            .then(|| self.fence.take_completion(self.generation))
            .flatten()
            .map(|proof| (report, proof))
    }
    pub(super) fn fail(&mut self) {
        self.failed = true;
        self.fence.invalidate();
        self.hooks.clear();
        self.reports.clear();
        self.hook_bytes = 0;
        self.done_report = None;
        self.unbound_user = None;
        self.unbound_assistant = None;
    }
    fn start_pending(&mut self, text: String) {
        let Some(generation) = self.generation.checked_add(1) else {
            self.fail();
            return;
        };
        self.generation = generation;
        self.fence.invalidate();
        self.pending_prompt = Some(text);
        self.pending_hook_seen = false;
        self.current_prompt = None;
        self.current_hook_seen = false;
        self.unbound_user = None;
        self.unbound_assistant = None;
    }
}

fn user_boundary(provider: ProviderId, record: &TranscriptRecord) -> Option<(u64, String)> {
    let row = &record.value;
    match provider {
        ProviderId::Claude
            if row["type"] == "user"
                && row["isSidechain"] != true
                && row["isMeta"] != true
                && row["isSynthetic"] != true
                && row["isCompactSummary"] != true =>
        {
            row["uuid"].as_str().filter(|id| !id.is_empty())?;
            let content = &row["message"]["content"];
            let text = if let Some(text) = content.as_str() {
                text.to_owned()
            } else {
                let blocks = content.as_array()?;
                if blocks.iter().any(|block| block["type"] == "tool_result") {
                    return None;
                }
                blocks
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .map(|block| block["text"].as_str())
                    .collect::<Option<Vec<_>>>()?
                    .join("\n")
            };
            (!text.is_empty()).then_some((record.offset, text))
        }
        ProviderId::Antigravity
            if row["source"] == "USER_EXPLICIT"
                && row["type"] == "USER_INPUT"
                && row["truncated_fields"].as_array().is_none_or(Vec::is_empty) =>
        {
            Some((
                row["step_index"].as_u64()?,
                row["content"].as_str()?.to_owned(),
            ))
        }
        _ => None,
    }
}
