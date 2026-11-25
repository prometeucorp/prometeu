//! One canonical writer for native conversations. Recovery and replay are storage-only;
//! live delivery is composed by the host through the pump's durable commit ports.
use crate::{
    lock::lock,
    terminal_agent_journal::{
        AppendResult, CanonicalPosition, Capture, CaptureJournal, JournalBinding, PreparedInput,
        Receipt,
    },
    terminal_agent_observation::{NativeCapture, NativeIngestionBatch},
};
use prometeu_core::{
    conversation::stream::{persistent, public_text, CommitOutcome, Lines, TranscriptStore},
    session::pump::CanonicalCommit,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const SOURCE: &str = "native:source-watermark";
const SOURCE_TRAJECTORY: &str = "native:source-trajectory";
const ERROR: &str = "terminal_ingestion_failed";

pub(crate) fn journal_path(root: &Path, session: &str) -> Result<std::path::PathBuf, String> {
    if session.is_empty()
        || session.len() > 256
        || !session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(ERROR.into());
    }
    Ok(root
        .join("terminal-ingestion")
        .join(format!("{session}.sqlite3")))
}

/// Resume may recover a native identity committed just before a crash lost the board update.
/// Fresh never adopts an old identity. The preparer still captures the account and reauthorizes
/// the exact native path before reading or launching that recovered session.
pub(crate) fn resume_request(
    root: &Path,
    request: &prometeu_core::session::launch::LaunchRequest,
) -> Result<prometeu_core::session::launch::LaunchRequest, String> {
    use prometeu_core::session::launch::{LaunchRequest, StartMode};
    let mode = match &request.mode {
        StartMode::Fresh => StartMode::Fresh,
        StartMode::Resume { provider_session } => {
            let provider = serde_json::to_value(request.settings.agent).map_err(|_| ERROR)?;
            let provider = provider.as_str().ok_or(ERROR)?;
            let journal = CaptureJournal::peek_binding(
                &journal_path(root, &request.session)?,
                &request.session,
                provider,
            )
            .map_err(|_| ERROR)?
            .and_then(|binding| binding.native_session);
            let path = super::binding::Binding::bound_native(
                root,
                &request.session,
                request.settings.agent,
            )
            .map_err(|_| ERROR)?;
            if journal
                .as_ref()
                .zip(path.as_ref())
                .is_some_and(|(a, b)| a != b)
            {
                return Err(ERROR.into());
            }
            let recovered = journal.or(path);
            if provider_session
                .as_ref()
                .zip(recovered.as_ref())
                .is_some_and(|(a, b)| a != b)
            {
                return Err(ERROR.into());
            }
            StartMode::Resume {
                // Resume cannot become a fresh native conversation merely because a crash
                // preceded every authoritative identity acknowledgment.
                provider_session: Some(provider_session.clone().or(recovered).ok_or(ERROR)?),
            }
        }
    };
    Ok(LaunchRequest {
        session: request.session.clone(),
        workspace: request.workspace.clone(),
        worktree: request.worktree.clone(),
        settings: request.settings.clone(),
        mode,
    })
}

pub(crate) struct JournalStore {
    inner: Mutex<Writer>,
    failed: AtomicBool,
}
struct Writer {
    log: File,
    journal: CaptureJournal,
    position: CanonicalPosition,
    fresh: bool,
    source_ingestion: bool,
    source_revision: Option<String>,
    observed_user: Option<String>,
    batch: Option<(String, Option<String>)>,
    replay: bool,
}

pub(crate) type Submission = PreparedInput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeDisposition {
    Duplicate,
    User,
    Output,
    Revision,
    Checkpoint,
}

impl JournalStore {
    /// The caller passes the existing canonical path and a private journal path for this exact
    /// tab. Complete pending bytes before any Lines seed, native replay or process launch.
    pub(crate) fn open(
        log_path: &Path,
        journal_path: &Path,
        binding: JournalBinding,
        fresh: bool,
    ) -> Result<Arc<Self>, String> {
        if fresh
            && CaptureJournal::peek_binding(journal_path, &binding.app_session, &binding.provider)
                .map_err(|_| ERROR)?
                .is_some_and(|saved| saved.native_session.is_some())
        {
            return Err(ERROR.into());
        }
        let parent = log_path.parent().ok_or(ERROR)?;
        crate::paths::ensure_private_dir(parent).map_err(|_| ERROR)?;
        let log = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(log_path)
            .map_err(|_| ERROR)?;
        let metadata = log.metadata().map_err(|_| ERROR)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(ERROR.into());
        }
        log.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|_| ERROR)?;
        let mut writer = Writer {
            log,
            journal: CaptureJournal::open(journal_path, &binding).map_err(|_| ERROR)?,
            position: CanonicalPosition { bytes: 0, lines: 0 },
            fresh,
            source_ingestion: matches!(
                binding.provider.as_str(),
                "cursor" | "codex" | "claude" | "antigravity"
            ),
            source_revision: None,
            observed_user: None,
            batch: None,
            replay: false,
        };
        writer.journal.recover(&mut writer.log).map_err(|_| ERROR)?;
        writer.journal.recover_inputs().map_err(|_| ERROR)?;
        writer.position = count(&mut writer.log)?;
        Ok(Arc::new(Self {
            inner: Mutex::new(writer),
            failed: AtomicBool::new(false),
        }))
    }

    pub(crate) fn seeded(&self) -> Result<Lines, String> {
        Lines::seeded(self)
    }

    /// Reattach an existing process that has not created its transcript yet. Keep its journal
    /// and identity, but allow the first prompt only when neither store records conversation work.
    pub(crate) fn resume_observation(
        &self,
        adopts_process: bool,
        transcript: Option<&Path>,
    ) -> Result<bool, String> {
        if !adopts_process
            || transcript.is_none()
            || transcript
                .map(Path::try_exists)
                .transpose()
                .map_err(|_| ERROR)?
                .unwrap_or(false)
        {
            return Ok(true);
        }
        let text = self.load()?;
        let mut writer = lock(&self.inner);
        if writer
            .journal
            .has_conversation_activity()
            .map_err(|_| ERROR)?
            || !text.lines().all(|line| {
                serde_json::from_str::<Value>(line)
                    .is_ok_and(|frame| !persistent(&frame) || frame["type"] == "background.changed")
            })
        {
            return Ok(true);
        }
        writer.fresh = true;
        Ok(false)
    }
    pub(crate) fn bind_native(&self, native: &str) -> Result<(), String> {
        lock(&self.inner)
            .journal
            .bind_native(native)
            .map_err(|_| ERROR.into())
    }
    pub(crate) fn caught_up(&self) -> bool {
        if !self.healthy() {
            return false;
        }
        let writer = lock(&self.inner);
        writer
            .journal
            .prepared_inputs()
            .is_ok_and(|pending| pending.is_empty())
            && writer
                .journal
                .pending_submissions()
                .is_ok_and(|pending| pending.is_empty())
    }
    pub(crate) fn healthy(&self) -> bool {
        !self.failed.load(Ordering::Acquire)
    }

    /// Capture the causal boundary before reserving input. The canonical echo retains original
    /// text; the normalized hash is only for the provider's proven submission transformation.
    pub(crate) fn submission(
        &self,
        id: &str,
        original: &str,
        normalized: &str,
    ) -> Result<Submission, String> {
        if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
            return Err(ERROR.into());
        }
        let mut writer = lock(&self.inner);
        if writer
            .journal
            .pending_submissions()
            .map_err(|_| ERROR)?
            .len()
            >= 64
        {
            return Err(ERROR.into());
        }
        let watermark = writer.journal.latest(SOURCE).map_err(|_| ERROR)?;
        let after_native_user = watermark
            .as_ref()
            .and_then(|record| record.checkpoint["lastUser"].as_str())
            .map(str::to_owned);
        let empty_baseline = watermark
            .as_ref()
            .is_some_and(|record| record.checkpoint["emptyBaseline"] == true)
            || (watermark.is_none() && writer.fresh);
        if writer.source_ingestion && after_native_user.is_none() && !empty_baseline {
            return Err(ERROR.into());
        }
        let submission = Submission {
            id: id.into(),
            original: original.into(),
            original_hash: input_hash(original),
            normalized_hash: input_hash(normalized),
            after_native_user,
            empty_baseline,
            at: crate::conversation::now(),
        };
        writer
            .journal
            .prepare_input(&submission)
            .map_err(|_| ERROR)?;
        Ok(submission)
    }

    pub(crate) fn accepted_submission(&self, id: &str, original: &str) -> Result<bool, String> {
        let writer = lock(&self.inner);
        match writer
            .journal
            .lookup(&format!("app:submission:{id}"), "accepted")
            .map_err(|_| ERROR)?
        {
            Some(receipt)
                if receipt.checkpoint["originalHash"].as_str()
                    == Some(input_hash(original).as_str()) =>
            {
                Ok(true)
            }
            Some(_) => Err(ERROR.into()),
            None => Ok(false),
        }
    }
    pub(crate) fn cancel_unwritten_submission(&self, id: &str) -> Result<(), String> {
        lock(&self.inner)
            .journal
            .cancel_unwritten_input(id)
            .map_err(|_| ERROR.into())
    }

    pub(crate) fn commit_submission(
        &self,
        submission: &Submission,
        lines: &[String],
        offset: u64,
    ) -> Result<CommitOutcome, String> {
        let users = lines
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|frame| frame["type"] == "user.message")
            .collect::<Vec<_>>();
        if lines.len() != 1
            || users.len() != 1
            || content_hash(&users[0]) != submission.original_hash
        {
            return Err(ERROR.into());
        }
        let mut writer = lock(&self.inner);
        let kind = if writer.source_ingestion {
            "submission"
        } else {
            "app"
        };
        let result = writer.append(&format!("app:submission:{}",submission.id),"accepted",json!({"kind":kind,"id":submission.id,"originalHash":submission.original_hash,"normalizedHash":submission.normalized_hash,"afterNativeUser":submission.after_native_user,"emptyBaseline":submission.empty_baseline}),lines.to_vec(),offset)?;
        writer
            .journal
            .finish_input(&submission.id)
            .map_err(|_| ERROR)?;
        Ok(result)
    }

    pub(crate) fn begin_batch(&self, batch: &NativeIngestionBatch) -> Result<(), String> {
        let mut writer = lock(&self.inner);
        writer.replay =
            batch.phase == prometeu_protocols::terminal_transcript::TranscriptPhase::Replay;
        if writer.journal.native_session().is_none() {
            return Err(ERROR.into());
        }
        if writer.source_revision.as_deref() != Some(&batch.source_revision) {
            writer.source_revision = Some(batch.source_revision.clone());
            writer.observed_user = None;
            writer.batch = None;
        }
        if let Some((id, initial)) = &writer.batch {
            if id == &batch.id {
                writer.observed_user = initial.clone();
                return Ok(());
            }
        }
        writer.batch = Some((batch.id.clone(), writer.observed_user.clone()));
        Ok(())
    }

    pub(crate) fn disposition(&self, record: &NativeCapture) -> Result<NativeDisposition, String> {
        let writer = lock(&self.inner);
        validate_source(record)?;
        writer.verify_trajectory(record)?;
        if writer
            .journal
            .lookup(&record.occurrence, &record.version)
            .map_err(|_| ERROR)?
            .is_some()
        {
            return Ok(NativeDisposition::Duplicate);
        }
        if record.frame.is_none() {
            return Ok(NativeDisposition::Checkpoint);
        }
        if record.checkpoint["kind"] == "user" {
            return Ok(NativeDisposition::User);
        }
        if record.checkpoint["kind"] == "completion" {
            return Ok(NativeDisposition::Output);
        }
        let previous = writer
            .journal
            .latest(&record.occurrence)
            .map_err(|_| ERROR)?;
        if let Some(previous) = &previous {
            if previous.checkpoint["messageId"] != record.checkpoint["messageId"]
                || previous.checkpoint["index"] != record.checkpoint["index"]
                || previous.checkpoint["kind"] != record.checkpoint["kind"]
                || previous.checkpoint["toolId"] != record.checkpoint["toolId"]
                || previous.checkpoint["frameType"] != record.checkpoint["frameType"]
                || (previous.checkpoint["sourceUser"].is_string()
                    && previous.checkpoint["sourceUser"] != record.checkpoint["sourceUser"])
            {
                return Err(ERROR.into());
            }
        }
        let message = record.checkpoint["messageId"].as_str().ok_or(ERROR)?;
        let turn = writer
            .journal
            .latest(&format!("{message}:checkpoint"))
            .map_err(|_| ERROR)?;
        let record_scope = record.checkpoint["sourceScope"] == "record"
            || turn
                .as_ref()
                .is_some_and(|turn| turn.checkpoint["sourceScope"] == "record");
        let mut superseded = false;
        if record_scope {
            let Some(user) = record.checkpoint["sourceUser"].as_str() else {
                // Recovery may contain a compacted ancestry or session-wide metadata. It can
                // update known history quietly, but cannot act on the currently running turn.
                return if previous.is_some() || record.checkpoint["kind"] == "metadata" {
                    Ok(NativeDisposition::Revision)
                } else {
                    Err(ERROR.into())
                };
            };
            let user = writer
                .journal
                .latest(user)
                .map_err(|_| ERROR)?
                .filter(|receipt| receipt.checkpoint["kind"] == "user")
                .ok_or(ERROR)?;
            superseded = writer
                .journal
                .source_input_after(user.position)
                .map_err(|_| ERROR)?;
            if previous
                .as_ref()
                .is_some_and(|receipt| !receipt.checkpoint["sourceUser"].is_string())
            {
                // Pre-v4 receipts have no causal completion index. Their existing occurrences
                // can acquire a source identity without replaying old work into a resumed turn.
                return Ok(NativeDisposition::Revision);
            }
            if record.checkpoint["kind"] == "result" {
                let tool = record.occurrence.strip_suffix(":result").ok_or(ERROR)?;
                let tool = writer
                    .journal
                    .latest(&format!("{tool}:tool"))
                    .map_err(|_| ERROR)?
                    .ok_or(ERROR)?;
                if tool.checkpoint["kind"] != "tool"
                    || tool.checkpoint["messageId"] != record.checkpoint["messageId"]
                    || tool.checkpoint["index"] != record.checkpoint["index"]
                    || tool.checkpoint["toolId"] != record.checkpoint["toolId"]
                {
                    return Err(ERROR.into());
                }
                if !tool.checkpoint["sourceUser"].is_string() {
                    return Ok(NativeDisposition::Revision);
                }
                if tool.checkpoint["sourceUser"] != record.checkpoint["sourceUser"] {
                    return Err(ERROR.into());
                }
            }
        }
        let completed = if record_scope {
            record.checkpoint["sourceUser"]
                .as_str()
                .map(|user| {
                    writer
                        .journal
                        .completed_source_user(user)
                        .map_err(|_| ERROR)
                })
                .transpose()?
                .unwrap_or(false)
        } else {
            turn.is_some_and(|turn| {
                matches!(
                    turn.checkpoint["status"].as_str(),
                    Some("completed" | "failed" | "interrupted")
                )
            })
        };
        if completed || superseded {
            let known_tool = if record.checkpoint["kind"] == "result" {
                let tool = record.occurrence.strip_suffix(":result").ok_or(ERROR)?;
                writer
                    .journal
                    .latest(&format!("{tool}:tool"))
                    .map_err(|_| ERROR)?
                    .is_some_and(|tool| {
                        tool.checkpoint["kind"] == "tool"
                            && tool.checkpoint["messageId"] == record.checkpoint["messageId"]
                            && tool.checkpoint["index"] == record.checkpoint["index"]
                            && tool.checkpoint["toolId"] == record.checkpoint["toolId"]
                            && (!tool.checkpoint["sourceUser"].is_string()
                                || tool.checkpoint["sourceUser"] == record.checkpoint["sourceUser"])
                    })
            } else {
                false
            };
            if previous.is_some() || known_tool || record.checkpoint["kind"] == "metadata" {
                return Ok(NativeDisposition::Revision);
            }
            // A first result can revise its already captured tool. Other newly discovered
            // historical occurrences need a source-specific proof, never current-turn effects.
            return Err(ERROR.into());
        }
        Ok(NativeDisposition::Output)
    }

    /// RPC source batches extend the current native user cursor instead of replaying a snapshot
    /// from its first row. Bootstrap still uses `begin_batch` and storage-only replay.
    pub(crate) fn begin_stream_batch(&self, batch: &NativeIngestionBatch) -> Result<(), String> {
        if batch.phase != prometeu_protocols::terminal_transcript::TranscriptPhase::Live {
            return Err(ERROR.into());
        }
        let mut writer = lock(&self.inner);
        if writer.journal.native_session().is_none() {
            return Err(ERROR.into());
        }
        if let Some((id, initial)) = &writer.batch {
            if id == &batch.id {
                writer.observed_user = initial.clone();
                return Ok(());
            }
        }
        if writer.observed_user.is_none() {
            writer.observed_user = writer
                .journal
                .latest(SOURCE)
                .map_err(|_| ERROR)?
                .and_then(|source| source.checkpoint["lastUser"].as_str().map(str::to_owned));
        }
        writer.replay = false;
        writer.source_revision = Some(batch.source_revision.clone());
        writer.batch = Some((batch.id.clone(), writer.observed_user.clone()));
        Ok(())
    }

    pub(crate) fn commit_record(
        &self,
        record: &NativeCapture,
        line: Option<&str>,
        offset: u64,
    ) -> Result<CommitOutcome, String> {
        lock(&self.inner).record(record, line, offset)
    }

    pub(crate) fn commit_completion(
        &self,
        native_source: &str,
        line: &str,
        offset: u64,
    ) -> Result<CommitOutcome, String> {
        let frame: Value = serde_json::from_str(line).map_err(|_| ERROR)?;
        if !self.healthy()
            || native_source.is_empty()
            || native_source.len() > 1024
            || native_source.chars().any(char::is_control)
            || frame["v"] != 1
            || frame["type"] != "turn.completed"
        {
            return Err(ERROR.into());
        }
        let mut writer = lock(&self.inner);
        if writer.journal.native_session().is_none() {
            return Err(ERROR.into());
        }
        let key = format!(
            "native:completion:{:x}",
            Sha256::digest(native_source.as_bytes())
        );
        if writer
            .journal
            .lookup(&key, "completed")
            .map_err(|_| ERROR)?
            .is_some()
        {
            return Ok(CommitOutcome::AlreadyCommitted);
        }
        let source_user = match &writer.observed_user {
            Some(user) => Some(user.clone()),
            None => writer
                .journal
                .latest(SOURCE)
                .map_err(|_| ERROR)?
                .and_then(|receipt| receipt.checkpoint["lastUser"].as_str().map(str::to_owned)),
        };
        writer.append(
            &key,
            "completed",
            json!({"kind":"completion","source":native_source,"sourceUser":source_user}),
            vec![line.into()],
            offset,
        )
    }

    /// Duplicate user records still advance the current snapshot's causal watermark.
    pub(crate) fn commit_checkpoint(
        &self,
        record: &NativeCapture,
    ) -> Result<CommitOutcome, String> {
        let mut writer = lock(&self.inner);
        let offset = writer.position.lines;
        writer.record(record, None, offset)
    }

    /// Replay can discover a crash tail but never enters Pump or public delivery. The caller
    /// seeds Lines only after this bootstrap finishes and acknowledges the observer batch.
    pub(crate) fn replay(&self, batch: &NativeIngestionBatch) -> Result<(), String> {
        if batch.phase != prometeu_protocols::terminal_transcript::TranscriptPhase::Replay {
            return Err(ERROR.into());
        }
        self.begin_batch(batch)?;
        for record in &batch.records {
            let line = record
                .frame
                .as_ref()
                .filter(|frame| persistent(frame))
                .map(|frame| public_text(&frame.to_string(), frame).into_owned());
            let mut writer = lock(&self.inner);
            let offset = writer.position.lines;
            writer.record(record, line.as_deref(), offset)?;
        }
        self.finish_batch(batch)
    }

    /// Call after all records committed, before observer acknowledgment. This private watermark
    /// restores the causal app-input queue after a crash; it adds no public line or fake event.
    pub(crate) fn finish_batch(&self, batch: &NativeIngestionBatch) -> Result<(), String> {
        if !batch.complete {
            return Err(ERROR.into());
        }
        let mut writer = lock(&self.inner);
        if writer.source_revision.as_deref() != Some(&batch.source_revision)
            || writer.batch.as_ref().map(|(id, _)| id.as_str()) != Some(&batch.id)
        {
            return Err(ERROR.into());
        }
        if batch.finished {
            let last = writer.observed_user.clone();
            let offset = writer.position.lines;
            writer.append(
                SOURCE,
                &batch.source_revision,
                json!({"kind":"source","lastUser":last,"emptyBaseline":last.is_none()}),
                vec![],
                offset,
            )?;
        }
        Ok(())
    }
}

impl CanonicalCommit for JournalStore {
    fn commit(&self, lines: &[String], offset: u64) -> Result<CommitOutcome, String> {
        if !self.healthy() {
            return Err(ERROR.into());
        }
        let result = lock(&self.inner).append(
            &format!("app:event:{}", uuid::Uuid::new_v4()),
            "v1",
            json!({"kind":"app"}),
            lines.to_vec(),
            offset,
        );
        // The portable feed API reports errors without owning process lifecycle. Once generic
        // output loses its durable receipt, the terminal host must stop before more observations.
        if result.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        result
    }
}
impl TranscriptStore for JournalStore {
    fn load(&self) -> Result<String, String> {
        let mut writer = lock(&self.inner);
        writer.log.seek(SeekFrom::Start(0)).map_err(|_| ERROR)?;
        let mut text = String::new();
        writer.log.read_to_string(&mut text).map_err(|_| ERROR)?;
        Ok(text)
    }
    fn append(&self, _: &str) -> Result<(), String> {
        Err("native_transcript_requires_commit".into())
    }
}

impl Writer {
    fn append(
        &mut self,
        occurrence: &str,
        version: &str,
        checkpoint: Value,
        lines: Vec<String>,
        offset: u64,
    ) -> Result<CommitOutcome, String> {
        if self.position.lines != offset {
            return Err(ERROR.into());
        }
        let result = self
            .journal
            .append(
                &mut self.log,
                Capture {
                    occurrence: occurrence.into(),
                    version: version.into(),
                    checkpoint,
                    position: self.position,
                    canonical_lines: lines,
                },
            )
            .map_err(|_| ERROR)?;
        match result {
            AppendResult::Appended(receipt) => {
                self.position = receipt.end();
                Ok(CommitOutcome::Appended)
            }
            AppendResult::AlreadyCommitted(_) => Ok(CommitOutcome::AlreadyCommitted),
        }
    }

    fn record(
        &mut self,
        record: &NativeCapture,
        line: Option<&str>,
        offset: u64,
    ) -> Result<CommitOutcome, String> {
        validate_source(record)?;
        if self.journal.native_session().is_none() {
            return Err(ERROR.into());
        }
        self.verify_trajectory(record)?;
        if let Some(trajectory) = record.checkpoint["trajectoryId"].as_str() {
            if self
                .journal
                .latest(SOURCE_TRAJECTORY)
                .map_err(|_| ERROR)?
                .is_none()
            {
                self.append(
                    SOURCE_TRAJECTORY,
                    "bound",
                    json!({"kind":"source_identity","trajectoryId":trajectory}),
                    vec![],
                    self.position.lines,
                )?;
            }
        }
        let user = record.checkpoint["kind"] == "user";
        if self
            .journal
            .lookup(&record.occurrence, &record.version)
            .map_err(|_| ERROR)?
            .is_some()
        {
            if user {
                self.observed_user = Some(record.occurrence.clone());
            }
            return Ok(CommitOutcome::AlreadyCommitted);
        }
        if user {
            let target = self.user_target(record)?;
            if let Some(target) = target {
                self.journal
                    .bind_alias(
                        &record.occurrence,
                        &record.version,
                        record.checkpoint.clone(),
                        &target,
                    )
                    .map_err(|_| ERROR)?;
                self.observed_user = Some(record.occurrence.clone());
                return Ok(CommitOutcome::AlreadyCommitted);
            }
        }
        let lines = match (&record.frame, line) {
            (Some(frame), Some(line)) => {
                let public = public_text(&frame.to_string(), frame).into_owned();
                if serde_json::from_str::<Value>(line).map_err(|_| ERROR)?
                    != serde_json::from_str::<Value>(&public).map_err(|_| ERROR)?
                {
                    return Err(ERROR.into());
                }
                vec![line.into()]
            }
            (None, None) => vec![],
            (Some(frame), None)
                if record.checkpoint["kind"] == "metadata" && !persistent(frame) =>
            {
                vec![]
            }
            _ => return Err(ERROR.into()),
        };
        let result = self.append(
            &record.occurrence,
            &record.version,
            record.checkpoint.clone(),
            lines,
            offset,
        )?;
        if user {
            self.observed_user = Some(record.occurrence.clone());
        }
        Ok(result)
    }

    fn verify_trajectory(&self, record: &NativeCapture) -> Result<(), String> {
        if self
            .journal
            .latest(SOURCE_TRAJECTORY)
            .map_err(|_| ERROR)?
            .is_some_and(|saved| {
                saved.checkpoint["trajectoryId"].as_str()
                    != record.checkpoint["trajectoryId"].as_str()
            })
        {
            return Err(ERROR.into());
        }
        Ok(())
    }

    fn user_target(&mut self, record: &NativeCapture) -> Result<Option<Receipt>, String> {
        let hash = record.checkpoint["contentHash"].as_str().ok_or(ERROR)?;
        if let Some(previous) = self.journal.latest(&record.occurrence).map_err(|_| ERROR)? {
            if previous.checkpoint["contentHash"].as_str() != Some(hash) {
                return Err(ERROR.into());
            }
            return Ok(Some(previous));
        }
        let pending = self.journal.pending_submissions().map_err(|_| ERROR)?;
        if pending.is_empty() && record.input_binding.is_none() {
            if let Some(input) = self
                .journal
                .prepared_inputs()
                .map_err(|_| ERROR)?
                .into_iter()
                .next()
                .filter(|input| {
                    input.after_native_user == self.observed_user
                        && (self.observed_user.is_some() || input.empty_baseline)
                })
            {
                if !self.replay {
                    return Err(ERROR.into());
                }
                self.note_echo_mismatch(&input.normalized_hash, hash);
                let frame = json!({"v":1,"type":"user.message","at":input.at,"content":[{"kind":"text","text":input.original}]});
                let occurrence = format!("app:submission:{}", input.id);
                self.append(&occurrence,"accepted",json!({"kind":"submission","id":input.id,"originalHash":input.original_hash,"normalizedHash":input.normalized_hash,"afterNativeUser":input.after_native_user,"emptyBaseline":input.empty_baseline}),vec![frame.to_string()],self.position.lines)?;
                self.journal.finish_input(&input.id).map_err(|_| ERROR)?;
                return self
                    .journal
                    .lookup(&occurrence, "accepted")
                    .map_err(|_| ERROR.into());
            }
        }
        let explicit = record
            .input_binding
            .as_ref()
            .map(|id| format!("app:submission:{id}"));
        let target = if let Some(explicit) = explicit {
            Some(
                self.journal
                    .lookup(&explicit, "accepted")
                    .map_err(|_| ERROR)?
                    .ok_or(ERROR)?,
            )
        } else {
            pending.into_iter().next().filter(|candidate| {
                candidate.checkpoint["afterNativeUser"].as_str() == self.observed_user.as_deref()
                    && (self.observed_user.is_some()
                        || candidate.checkpoint["emptyBaseline"] == true)
            })
        };
        if let Some(target) = &target {
            self.note_echo_mismatch(
                target.checkpoint["normalizedHash"]
                    .as_str()
                    .unwrap_or_default(),
                hash,
            );
        }
        Ok(target)
    }

    /// Causal order already proves which app submission this native user echoes; the hash only
    /// confirms the provider kept the text unchanged. CLIs reformat prompts across versions
    /// (NFC, paste wrappers), so a mismatch is logged and the echo is bound anyway. Rejecting it
    /// would kill the agent and poison every later resume on the same record.
    fn note_echo_mismatch(&self, expected: &str, observed: &str) {
        if expected != observed {
            super::diagnose(
                self.journal.native_session().unwrap_or("unbound"),
                "history.echo_mismatch",
            );
        }
    }
}

fn validate_source(record: &NativeCapture) -> Result<(), String> {
    if let Some(trajectory) = record
        .checkpoint
        .get("trajectoryId")
        .filter(|value| !value.is_null())
    {
        if !trajectory.as_str().is_some_and(|id| {
            !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
        }) {
            return Err(ERROR.into());
        }
    }
    if let Some(user) = record
        .checkpoint
        .get("sourceUser")
        .filter(|value| !value.is_null())
    {
        if !user.as_str().is_some_and(|user| {
            !user.is_empty() && user.len() <= 1024 && !user.chars().any(char::is_control)
        }) {
            return Err(ERROR.into());
        }
    }
    match record.checkpoint["kind"].as_str() {
        Some("user")
            if record.frame.as_ref().is_some_and(|frame| {
                frame["type"] == "user.message"
                    && record.checkpoint["contentHash"].as_str()
                        == Some(content_hash(frame).as_str())
            }) =>
        {
            Ok(())
        }
        Some("text" | "thinking" | "tool")
            if record.frame.as_ref().is_some_and(|frame| {
                frame["type"] == "assistant.block"
                    && frame["messageId"] == record.checkpoint["messageId"]
                    && frame["index"] == record.checkpoint["index"]
                    && frame["block"]["kind"] == record.checkpoint["kind"]
                    && (record.checkpoint["kind"] != "tool"
                        || (record.checkpoint["toolId"].is_string()
                            && frame["block"]["id"] == record.checkpoint["toolId"]))
            }) =>
        {
            Ok(())
        }
        Some("result")
            if record.frame.as_ref().is_some_and(|frame| {
                frame["type"] == "tool.completed"
                    && frame["toolId"] == record.checkpoint["toolId"]
                    && record.checkpoint["messageId"].is_string()
                    && record.checkpoint["index"].is_u64()
            }) =>
        {
            Ok(())
        }
        Some("turn") if record.frame.is_none() => Ok(()),
        Some("metadata")
            if record.frame.as_ref().is_some_and(|frame| {
                record.checkpoint["messageId"].is_string()
                    && record.checkpoint["index"].is_u64()
                    && record.checkpoint["frameType"] == frame["type"]
                    && matches!(
                        frame["type"].as_str(),
                        Some(
                            "telemetry.usage"
                                | "context.updated"
                                | "commands.updated"
                                | "system.summary"
                                | "context.reported"
                                | "system.notice"
                                | "context.compaction"
                                | "context.compacted"
                        )
                    )
            }) =>
        {
            Ok(())
        }
        Some("completion")
            if record.frame.as_ref().is_some_and(|frame| {
                frame["type"] == "turn.completed"
                    && record.checkpoint["nativeTurn"]
                        .as_str()
                        .is_some_and(|turn| !turn.is_empty())
            }) =>
        {
            Ok(())
        }
        _ => Err(ERROR.into()),
    }
}
fn count(file: &mut File) -> Result<CanonicalPosition, String> {
    file.seek(SeekFrom::Start(0)).map_err(|_| ERROR)?;
    let mut bytes = 0;
    let mut lines = 0;
    let mut last = None;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| ERROR)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        lines += buffer[..n].iter().filter(|byte| **byte == b'\n').count() as u64;
        last = Some(buffer[n - 1]);
    }
    if last.is_some_and(|byte| byte != b'\n') {
        return Err(ERROR.into());
    }
    Ok(CanonicalPosition { bytes, lines })
}
fn input_hash(text: &str) -> String {
    content_hash(&json!({"content":[{"kind":"text","text":text}]}))
}
fn content_hash(frame: &Value) -> String {
    Sha256::digest(frame["content"].to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests;
