//! Record reduction is separate from canonical conversation events and terminal presentation.
use std::collections::BTreeMap;

use super::{valid_id, valid_segment, ProgramStatusReport, ProgramStatusState};

pub const PROGRAM_STATUS_RECORD_LIMIT: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProgramStatusSnapshot {
    pub root: Option<ProgramStatusReport>,
    pub children: BTreeMap<String, ProgramStatusReport>,
}

impl ProgramStatusSnapshot {
    /// A finished root does not imply its background workers have finished.
    pub fn has_active_work(&self) -> bool {
        self.records().any(|record| active(record.state))
    }

    pub fn has_blocked_work(&self) -> bool {
        self.records()
            .any(|record| record.state == ProgramStatusState::Blocked)
    }

    /// Missing parents are permitted; display metadata comes from the nearest existing ancestor.
    /// No inherited value is written back into a record.
    pub fn inherited_app(&self, id: Option<&str>) -> Option<&str> {
        if let Some(mut id) = id {
            if !valid_id(id.as_bytes()) {
                return None;
            }
            loop {
                if let Some(app) = self
                    .children
                    .get(id)
                    .and_then(|record| record.app.as_deref())
                {
                    return Some(app);
                }
                let Some((parent, _)) = id.rsplit_once('/') else {
                    break;
                };
                id = parent;
            }
        }
        self.root.as_ref().and_then(|record| record.app.as_deref())
    }

    fn records(&self) -> impl Iterator<Item = &ProgramStatusReport> {
        self.root.iter().chain(self.children.values())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatusTransition {
    /// The addressed record. A child's transition never changes the root record.
    pub id: Option<String>,
    pub previous: Option<ProgramStatusReport>,
    pub current: Option<ProgramStatusReport>,
    /// Records cleared explicitly or evicted at capacity. Removal is never completion.
    pub removed: Vec<ProgramStatusReport>,
    /// Only this record's observed working/blocked -> done/error transition.
    pub completed: bool,
}

impl ProgramStatusTransition {
    pub fn changed(&self) -> bool {
        self.previous != self.current || !self.removed.is_empty()
    }
}

/// One store belongs to one PTY incarnation. There are no timers or silence-based transitions.
#[derive(Default)]
pub struct ProgramStatusStore {
    // Least recently updated first. The fixed cap bounds scans and avoids a clock or sequence id.
    records: Vec<ProgramStatusReport>,
}

impl ProgramStatusStore {
    pub fn snapshot(&self) -> ProgramStatusSnapshot {
        let mut snapshot = ProgramStatusSnapshot::default();
        for record in &self.records {
            if let Some(id) = &record.id {
                snapshot.children.insert(id.clone(), record.clone());
            } else {
                snapshot.root = Some(record.clone());
            }
        }
        snapshot
    }

    /// Reports replace all fields of one record. Duplicates refresh recency but emit no change
    /// or completion. Invalid manually constructed reports leave every stored record untouched.
    pub fn apply(&mut self, report: ProgramStatusReport) -> Option<ProgramStatusTransition> {
        if !valid_report(&report) {
            return None;
        }
        let id = report.id.clone();
        let index = self.records.iter().position(|record| record.id == id);
        let previous = index.map(|index| self.records[index].clone());
        if report.state == ProgramStatusState::Clear {
            let removed = self.remove_where(|record| match id.as_deref() {
                None => true,
                Some(id) => record.id.as_deref().is_some_and(|candidate| {
                    candidate == id
                        || candidate
                            .strip_prefix(id)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                }),
            });
            return Some(ProgramStatusTransition {
                id,
                previous,
                current: None,
                removed,
                completed: false,
            });
        }

        let completed = previous.as_ref().is_some_and(|previous| {
            active(previous.state)
                && matches!(
                    report.state,
                    ProgramStatusState::Done | ProgramStatusState::Error
                )
        });
        let mut removed = Vec::new();
        if let Some(index) = index {
            self.records.remove(index);
        } else if self.records.len() == PROGRAM_STATUS_RECORD_LIMIT {
            removed.push(self.records.remove(0));
        }
        self.records.push(report.clone());
        Some(ProgramStatusTransition {
            id,
            previous,
            current: Some(report),
            removed,
            completed,
        })
    }

    /// The host calls this on attached process exit or the next shell prompt. Done/error records
    /// survive, including children of removed records. This does not announce completion.
    pub fn release_active(&mut self) -> Vec<ProgramStatusReport> {
        self.remove_where(|record| active(record.state))
    }

    /// The host calls this for a new PTY incarnation or a full terminal reset (RIS).
    pub fn reset(&mut self) -> Vec<ProgramStatusReport> {
        std::mem::take(&mut self.records)
    }

    fn remove_where(
        &mut self,
        predicate: impl Fn(&ProgramStatusReport) -> bool,
    ) -> Vec<ProgramStatusReport> {
        let mut removed = Vec::new();
        self.records.retain(|record| {
            if predicate(record) {
                removed.push(record.clone());
                false
            } else {
                true
            }
        });
        removed
    }
}

fn active(state: ProgramStatusState) -> bool {
    matches!(
        state,
        ProgramStatusState::Working | ProgramStatusState::Blocked
    )
}

fn valid_report(report: &ProgramStatusReport) -> bool {
    report
        .id
        .as_deref()
        .is_none_or(|id| valid_id(id.as_bytes()))
        && report
            .app
            .as_deref()
            .is_none_or(|app| valid_segment(app.as_bytes()))
        && report
            .message
            .as_deref()
            .is_none_or(|message| valid_text(message, 2048))
        && report
            .title
            .as_deref()
            .is_none_or(|title| valid_text(title, 192))
        && (report.kind.is_none() || report.state == ProgramStatusState::Blocked)
        && report
            .progress
            .is_none_or(|progress| progress <= 100 && active(report.state))
}

fn valid_text(text: &str, limit: usize) -> bool {
    text.len() <= limit && !text.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_status::{ProgramStatusEvent, ProgramStatusParser};

    fn apply(store: &mut ProgramStatusStore, body: &str) -> ProgramStatusTransition {
        let mut events =
            ProgramStatusParser::default().feed(format!("\x1b]7501;{body}\x07").as_bytes());
        let ProgramStatusEvent::Report(report) = events.pop().expect("valid report") else {
            panic!("expected report");
        };
        store.apply(report).expect("valid report")
    }

    #[test]
    fn terminal_status_tree_preserves_independent_states_and_inherits_only_app() {
        let mut store = ProgramStatusStore::default();
        apply(&mut store, "state=working:app=agent:title=Um9vdA==");
        apply(&mut store, "state=working:id=worker:app=tools");
        apply(&mut store, "state=blocked:id=worker/missing/leaf:kind=auth");
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.inherited_app(Some("worker/missing/leaf")),
            Some("tools")
        );
        assert_eq!(snapshot.inherited_app(Some("other")), Some("agent"));
        assert_eq!(snapshot.inherited_app(Some("/invalid")), None);
        assert_eq!(snapshot.children["worker/missing/leaf"].app, None);
        assert_eq!(snapshot.children["worker/missing/leaf"].title, None);
        assert!(snapshot.has_blocked_work());

        let child_done = apply(&mut store, "state=done:id=worker/missing/leaf");
        assert!(child_done.completed);
        assert_eq!(child_done.id.as_deref(), Some("worker/missing/leaf"));
        assert_eq!(
            store.snapshot().root.unwrap().state,
            ProgramStatusState::Working
        );
        apply(&mut store, "state=working:id=worker");
        assert_eq!(
            store.snapshot().inherited_app(Some("worker/missing/leaf")),
            Some("agent")
        );
    }

    #[test]
    fn terminal_status_completion_requires_an_observed_active_record() {
        for previous in ["idle", "working", "blocked", "done", "error"] {
            for current in ["idle", "working", "blocked", "done", "error", "clear"] {
                let mut store = ProgramStatusStore::default();
                assert!(!apply(&mut store, &format!("state={previous}")).completed);
                let transition = apply(&mut store, &format!("state={current}"));
                assert_eq!(
                    transition.completed,
                    matches!(previous, "working" | "blocked")
                        && matches!(current, "done" | "error"),
                    "{previous} -> {current}"
                );
            }
        }
    }

    #[test]
    fn terminal_status_duplicate_reports_do_not_repeat_changes_or_completion() {
        let mut store = ProgramStatusStore::default();
        assert!(apply(&mut store, "state=working").changed());
        assert!(!apply(&mut store, "state=working").changed());
        assert!(apply(&mut store, "state=done:msg=T0s=").completed);
        let before = store.snapshot();
        let duplicate = apply(&mut store, "state=done:msg=T0s=");
        assert!(!duplicate.changed());
        assert!(!duplicate.completed);
        assert_eq!(store.snapshot(), before);
        let metadata = apply(&mut store, "state=done:msg=UmVhZHk=");
        assert!(metadata.changed());
        assert!(!metadata.completed);
    }

    #[test]
    fn terminal_status_background_work_holds_activity_until_observed_or_released() {
        let mut store = ProgramStatusStore::default();
        apply(&mut store, "state=working");
        apply(&mut store, "state=blocked:id=worker");
        apply(&mut store, "state=done:id=worker/result");
        apply(&mut store, "state=error:id=failed");
        apply(&mut store, "state=idle:id=waiting");
        assert!(apply(&mut store, "state=done").completed);
        let background = store.snapshot();
        assert!(background.has_active_work());
        assert!(background.has_blocked_work());
        assert_eq!(store.snapshot(), background);
        let removed = store.release_active();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].id.as_deref(), Some("worker"));
        let snapshot = store.snapshot();
        assert!(!snapshot.has_active_work());
        assert_eq!(snapshot.root.unwrap().state, ProgramStatusState::Done);
        assert_eq!(snapshot.children.len(), 3);
        assert_eq!(
            snapshot.children["worker/result"].state,
            ProgramStatusState::Done
        );
        assert!(store.release_active().is_empty());
    }

    #[test]
    fn terminal_status_clear_removes_exact_subtrees_without_completing_them() {
        let mut store = ProgramStatusStore::default();
        apply(&mut store, "state=working");
        apply(&mut store, "state=working:id=worker/one");
        apply(&mut store, "state=done:id=worker/one/result");
        apply(&mut store, "state=working:id=worker2");
        let clear = apply(&mut store, "state=clear:id=worker");
        assert!(clear.changed());
        assert!(!clear.completed);
        assert!(clear.previous.is_none());
        assert!(clear.current.is_none());
        assert_eq!(clear.removed.len(), 2);
        assert!(store.snapshot().children.contains_key("worker2"));
        assert!(store.snapshot().root.is_some());
        assert!(!apply(&mut store, "state=clear:id=worker").changed());
        let clear = apply(&mut store, "state=clear");
        assert!(!clear.completed);
        assert_eq!(clear.removed.len(), 2);
        assert_eq!(store.snapshot(), ProgramStatusSnapshot::default());
        assert!(!apply(&mut store, "state=clear").changed());
    }

    #[test]
    fn terminal_status_capacity_evicts_the_least_recent_record_without_completion() {
        let mut store = ProgramStatusStore::default();
        apply(&mut store, "state=working");
        for id in 0..PROGRAM_STATUS_RECORD_LIMIT - 1 {
            apply(&mut store, &format!("state=working:id=worker{id}"));
        }
        assert!(!apply(&mut store, "state=working").changed());
        let transition = apply(&mut store, "state=done:id=new");
        assert!(!transition.completed);
        assert_eq!(transition.removed.len(), 1);
        assert_eq!(transition.removed[0].id.as_deref(), Some("worker0"));
        let snapshot = store.snapshot();
        assert!(snapshot.root.is_some());
        assert_eq!(snapshot.children.len(), PROGRAM_STATUS_RECORD_LIMIT - 1);
        assert_eq!(store.reset().len(), PROGRAM_STATUS_RECORD_LIMIT);
        assert_eq!(store.snapshot(), ProgramStatusSnapshot::default());
        assert!(store.reset().is_empty());
    }

    #[test]
    fn terminal_status_reducer_rejects_invalid_constructed_reports_atomically() {
        let mut store = ProgramStatusStore::default();
        let record = apply(&mut store, "state=working").current.unwrap();
        let before = store.snapshot();
        let mut invalid = record.clone();
        invalid.id = Some("invalid//child".into());
        assert!(store.apply(invalid).is_none());
        let mut invalid = record.clone();
        invalid.message = Some("x".repeat(2049));
        assert!(store.apply(invalid).is_none());
        let mut invalid = record;
        invalid.title = Some("Control\ntext".into());
        assert!(store.apply(invalid).is_none());
        assert_eq!(store.snapshot(), before);
    }
}
