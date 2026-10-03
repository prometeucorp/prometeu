//! Bounded resident effects. Completion belongs to the host, even after a window detaches.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

enum State<T> {
    Running(mpsc::Receiver<Result<T, String>>),
    Settling,
    Done(Result<Value, String>, Instant),
}
pub struct Jobs<T>(Mutex<HashMap<String, State<T>>>);
impl<T> Default for Jobs<T> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}
impl<T: Send + 'static> Jobs<T> {
    fn expire(jobs: &mut HashMap<String, State<T>>) {
        jobs.retain(|_, state| !matches!(state, State::Done(_, until) if *until <= Instant::now()));
    }
    pub fn idle(&self) -> bool {
        let mut jobs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut jobs);
        jobs.is_empty()
    }
    pub fn running(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|s| !matches!(s, State::Done(..)))
    }
    pub fn start(
        &self,
        effect: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<Value, String> {
        let mut jobs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut jobs);
        if jobs.len() >= 8 {
            return Err("Too many pending operations".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("resident-operation".into())
            .spawn(move || {
                let _ = sender.send(effect());
            })
            .map_err(|e| e.to_string())?;
        jobs.insert(id.clone(), State::Running(receiver));
        Ok(json!({"job":id}))
    }
    pub fn take_ready(&self) -> Vec<(String, Result<T, String>)> {
        let mut jobs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut jobs);
        jobs.iter_mut()
            .filter_map(|(id, state)| {
                let State::Running(receiver) = state else {
                    return None;
                };
                let result = match receiver.try_recv() {
                    Ok(result) => result,
                    Err(mpsc::TryRecvError::Empty) => return None,
                    Err(mpsc::TryRecvError::Disconnected) => Err("Operation worker stopped".into()),
                };
                *state = State::Settling;
                Some((id.clone(), result))
            })
            .collect()
    }
    pub fn complete(&self, id: &str, result: Result<Value, String>) {
        let result = match serde_json::to_vec(&result) {
            Ok(bytes) if bytes.len() < crate::host::MAX_RESPONSE - 1024 => result,
            _ => Err("Operation result exceeds 8 MiB".into()),
        };
        let mut jobs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = jobs.get_mut(id) {
            *state = State::Done(result, Instant::now() + Duration::from_secs(600));
        }
    }
    pub fn poll(&self, id: &str) -> Result<Value, String> {
        let mut jobs = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut jobs);
        match jobs.get(id).ok_or("Unknown operation")? {
            State::Running(_) | State::Settling => Ok(json!({"done":false})),
            State::Done(..) => {
                let Some(State::Done(result, _)) = jobs.remove(id) else {
                    unreachable!()
                };
                result.map(|result| json!({"done":true,"result":result}))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_jobs_keep_running_effects_and_consume_completed_results_once() {
        let jobs = Jobs::default();
        let mut release = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..8 {
            let (tx, rx) = mpsc::channel();
            release.push(tx);
            ids.push(
                jobs.start(move || {
                    rx.recv().unwrap();
                    Ok(json!("finished"))
                })
                .unwrap()["job"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        assert!(jobs
            .start(|| panic!("rejected work must not start"))
            .is_err());
        assert!(jobs.running());
        assert!(!jobs.idle());
        assert_eq!(jobs.poll(&ids[0]).unwrap(), json!({"done":false}));
        for tx in release {
            tx.send(()).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while jobs.running() {
            for (id, result) in jobs.take_ready() {
                jobs.complete(&id, result);
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!jobs.idle(), "retain unclaimed results for reattachment");
        for id in ids {
            assert_eq!(jobs.poll(&id).unwrap()["result"], "finished");
            assert!(jobs.poll(&id).is_err());
        }
        assert!(jobs.idle());
    }
    #[test]
    fn oversized_results_and_panicked_workers_release_admission() {
        let jobs = Jobs::<Value>::default();
        let id = jobs.start(|| panic!("worker failure")).unwrap()["job"]
            .as_str()
            .unwrap()
            .to_owned();
        let deadline = Instant::now() + Duration::from_secs(5);
        while jobs.running() {
            for (id, result) in jobs.take_ready() {
                jobs.complete(&id, result);
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(jobs.poll(&id).unwrap_err(), "Operation worker stopped");
        let id = "oversized";
        jobs.0.lock().unwrap().insert(id.into(), State::Settling);
        jobs.complete(id, Ok(json!("x".repeat(crate::host::MAX_RESPONSE))));
        assert_eq!(jobs.poll(id).unwrap_err(), "Operation result exceeds 8 MiB");
        jobs.0.lock().unwrap().insert(
            "expired".into(),
            State::Done(Ok(Value::Null), Instant::now()),
        );
        assert!(jobs.idle());
        assert!(jobs.poll("expired").is_err());
    }
}
