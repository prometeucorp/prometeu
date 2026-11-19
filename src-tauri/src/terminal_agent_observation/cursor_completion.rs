//! Cursor's successful turn marker is written only after its Stop hook chain returns.
//! A private pre-ack witness makes that file change causal without interpreting elapsed time.
//! An authenticated aborted hook instead settles its exact request, even if native cleanup
//! later emits a completed hook for the same request.
use super::*;
use prometeu_protocols::cursor_store::{projection::CursorTailTurn, CursorRunStatus};

const MARKER_LIMIT: u64 = 4096;

#[derive(Clone, Debug)]
pub struct CursorStopStamp {
    path: PathBuf,
    before: Option<FileStamp>,
}

pub struct CursorStopWitness {
    roots: Vec<AuthorizedRoot>,
}

pub(super) struct CursorStop {
    pub(super) generation: String,
    stamp: Option<CursorStopStamp>,
    interrupted: bool,
}

pub struct CursorCompletion {
    pub native_source: String,
    pub request_id: String,
    pub interrupted: bool,
}

impl TerminalAgentObservation {
    pub(super) fn cursor_hook(&mut self, event: &NativeHookEvent) {
        let Some(cursor) = &mut self.cursor_store else {
            return;
        };
        if event.agent_id.is_some() {
            return;
        }
        if event.kind == NativeHookKind::PromptSubmitted {
            let generation = event.generation_id.as_ref().filter(|id| valid_id(id));
            cursor.prompt_generation = generation
                .filter(|id| {
                    cursor
                        .tail_turn
                        .as_ref()
                        .and_then(|tail| tail.key.request_id.as_deref())
                        != Some(id.as_str())
                })
                .cloned();
            return;
        }
        if event.kind == NativeHookKind::StopObserved {
            let Some(generation) = event.generation_id.as_ref().filter(|id| valid_id(id)) else {
                return;
            };
            if let Some(index) = cursor
                .stops
                .iter()
                .position(|stop| stop.generation == *generation)
            {
                if cursor.stops[index].interrupted || event.reason.as_deref() == Some("completed") {
                    return;
                }
                cursor.stops.remove(index);
            }
            let stop = match event.reason.as_deref() {
                Some("aborted") => Some(CursorStop {
                    generation: generation.clone(),
                    stamp: None,
                    interrupted: true,
                }),
                Some("completed") => event.cursor_stop_stamp.clone().map(|stamp| CursorStop {
                    generation: generation.clone(),
                    stamp: Some(stamp),
                    interrupted: false,
                }),
                _ => None,
            };
            if let Some(stop) = stop {
                // Independent hook deliveries can trail a subsequent request. Keep bounded
                // candidates by identity so an old stop cannot overwrite the current proof.
                if cursor.stops.len() == PROMPT_COUNT_LIMIT {
                    cursor.stops.pop_front();
                }
                cursor.stops.push_back(stop);
            }
        }
    }

    /// Called after durable source acknowledgement. The returned proof still needs the host's
    /// current input transaction guard and, for success, an empty composer. It never publishes
    /// an event itself; an interrupted native prompt may be restored as an editable draft.
    pub fn cursor_completion(&mut self) -> Result<Option<CursorCompletion>, ObservationError> {
        let Some(cursor) = &self.cursor_store else {
            return Ok(None);
        };
        if self.identity_failed || self.content_failed.is_some() || !cursor.completion_ready() {
            return Ok(None);
        }
        let Some(tail) = &cursor.tail_turn else {
            return Ok(None);
        };
        let Some(stop) = cursor
            .stops
            .iter()
            .find(|stop| tail.key.request_id.as_deref() == Some(&stop.generation))
        else {
            return Ok(None);
        };
        if cursor.completed_request.as_ref() == Some(&stop.generation)
            || cursor
                .prompt_generation
                .as_ref()
                .is_some_and(|prompt| prompt != &stop.generation)
            || tail.key.request_id.as_deref() != Some(&stop.generation)
        {
            return Ok(None);
        }
        if !stop.interrupted {
            let Some(stamp) = &stop.stamp else {
                return Ok(None);
            };
            if !matches_tail(tail, &stop.generation)
                || self.transcript.as_ref() != Some(&stamp.path)
                || !successful_marker(&mut self.roots, stamp)?
            {
                return Ok(None);
            }
        }
        let native = self
            .native_session
            .as_ref()
            .ok_or(ObservationError::Identity)?;
        Ok(Some(CursorCompletion {
            native_source: serde_json::json!(["cursor", native, stop.generation, tail.key.run_id])
                .to_string(),
            request_id: stop.generation.clone(),
            interrupted: stop.interrupted,
        }))
    }

    pub fn acknowledge_cursor_completion(&mut self, request: &str) {
        if let Some(cursor) = &mut self.cursor_store {
            if cursor.stops.iter().any(|stop| stop.generation == request) {
                cursor.completed_request = Some(request.into());
            }
        }
    }
}

fn matches_tail(tail: &CursorTailTurn, generation: &str) -> bool {
    if tail.key.request_id.as_deref() != Some(generation) {
        return false;
    }
    if tail.run.as_ref().map(|run| &run.id) != tail.key.run_id.as_ref() {
        return false;
    }
    match (&tail.run, &tail.root_run) {
        // The pinned TUI's actual store omits both records. The post-hooks success marker is
        // the completion authority; an absent optional protobuf record is not manufactured.
        (None, None) => true,
        (Some(turn), None) => turn.status == CursorRunStatus::Completed,
        (Some(turn), Some(root)) => {
            turn.id == root.id
                && turn.status == CursorRunStatus::Completed
                && root.status == CursorRunStatus::Completed
        }
        _ => false,
    }
}

fn successful_marker(
    roots: &mut [AuthorizedRoot],
    witness: &CursorStopStamp,
) -> Result<bool, ObservationError> {
    let Some(mut file) = open_authorized(roots, &witness.path)? else {
        return Ok(false);
    };
    let before = regular_stamp(&file)?;
    if witness.before.as_ref() == Some(&before) || before.len == 0 {
        return Ok(false);
    }
    let start = before.len.saturating_sub(MARKER_LIMIT);
    file.seek(SeekFrom::Start(start))
        .map_err(|_| ObservationError::Read)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MARKER_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ObservationError::Read)?;
    if bytes.len() as u64 != before.len - start || regular_stamp(&file)? != before {
        return Err(ObservationError::Unstable);
    }
    if bytes.last() != Some(&b'\n') {
        return Ok(false);
    }
    bytes.pop();
    let line = match bytes.iter().rposition(|byte| *byte == b'\n') {
        Some(index) => &bytes[index + 1..],
        None if start == 0 => &bytes[..],
        None => return Ok(false),
    };
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return Ok(false);
    };
    Ok(value["type"] == "turn_ended"
        && value["status"] == "success"
        && value.as_object().is_some_and(|fields| fields.len() == 2))
}

impl CursorStopWitness {
    pub fn new(roots: &[PathBuf]) -> Result<Self, ObservationError> {
        if roots.is_empty() {
            return Err(ObservationError::Path);
        }
        Ok(Self {
            roots: roots
                .iter()
                .cloned()
                .map(AuthorizedRoot::new)
                .collect::<Result<_, _>>()?,
        })
    }

    /// Runs on the hook listener before its acknowledgement. Neither failures nor missing files
    /// change the native hook result; a failed capture simply cannot prove completion.
    pub fn capture(&mut self, event: &NativeHookEvent) -> Option<CursorStopStamp> {
        if event.provider != ProviderId::Cursor
            || event.kind != NativeHookKind::StopObserved
            || event.agent_id.is_some()
            || event.reason.as_deref() != Some("completed")
            || !event.generation_id.as_deref().is_some_and(valid_id)
        {
            return None;
        }
        let path = event.transcript_path.as_ref()?;
        if !safe_absolute(path)
            || !session_path(ProviderId::Cursor, path, &event.native_session)
            || !self.roots.iter().any(|root| path.starts_with(&root.path))
        {
            return None;
        }
        let before = open_authorized(&mut self.roots, path)
            .ok()?
            .map(|file| regular_stamp(&file))
            .transpose()
            .ok()?;
        Some(CursorStopStamp {
            path: path.clone(),
            before,
        })
    }
}

fn regular_stamp(file: &File) -> Result<FileStamp, ObservationError> {
    let metadata = file.metadata().map_err(|_| ObservationError::Read)?;
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1
    {
        return Err(ObservationError::Path);
    }
    Ok(FileStamp::from(&metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_protocols::cursor_store::{BlobId, CursorRun, CursorTurnKey};

    #[test]
    fn cursor_completion_optional_runs_constrain_identity_and_status_when_present() {
        let mut tail = CursorTailTurn {
            key: CursorTurnKey {
                user_message_id: "user".into(),
                request_id: Some("request".into()),
                run_id: None,
            },
            version: BlobId([1; 32]),
            run: None,
            root_run: None,
        };
        assert!(matches_tail(&tail, "request"));
        assert!(!matches_tail(&tail, "other"));
        let run = CursorRun {
            id: "run".into(),
            status: CursorRunStatus::Running,
            started_at_ms: None,
            finished_at_ms: None,
        };
        tail.run = Some(run.clone());
        tail.key.run_id = Some(run.id.clone());
        assert!(!matches_tail(&tail, "request"));
        tail.run.as_mut().unwrap().status = CursorRunStatus::Completed;
        assert!(matches_tail(&tail, "request"));
        tail.root_run = Some(run);
        assert!(!matches_tail(&tail, "request"));
        tail.root_run.as_mut().unwrap().status = CursorRunStatus::Completed;
        assert!(matches_tail(&tail, "request"));
        tail.root_run.as_mut().unwrap().id = "different-run".into();
        assert!(!matches_tail(&tail, "request"));
    }
}
