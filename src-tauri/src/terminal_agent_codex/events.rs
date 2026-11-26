//! Bound retained live JSON by bytes as well as message count.
use super::{wire::Error, QUEUE_LIMIT};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};

const LIVE_QUEUE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) struct Sender {
    sender: mpsc::SyncSender<Queued>,
    bytes: Arc<AtomicUsize>,
}

pub(crate) struct Events(mpsc::Receiver<Queued>);

struct Queued {
    value: Option<Result<Value, String>>,
    size: usize,
    bytes: Arc<AtomicUsize>,
}

impl Drop for Queued {
    fn drop(&mut self) {
        self.bytes.fetch_sub(self.size, Ordering::Relaxed);
    }
}

pub(crate) fn channel() -> (Sender, Events) {
    let (sender, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
    (
        Sender {
            sender,
            bytes: Arc::new(AtomicUsize::new(0)),
        },
        Events(receiver),
    )
}

impl Sender {
    pub(crate) fn try_send(&self, value: Result<Value, String>) -> Result<(), Error> {
        let size = match &value {
            Ok(value) => value.to_string().len(),
            Err(error) => error.len(),
        };
        self.bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bytes| {
                bytes
                    .checked_add(size)
                    .filter(|sum| *sum <= LIVE_QUEUE_BYTES)
            })
            .map_err(|_| Error::Limit)?;
        self.sender
            .try_send(Queued {
                value: Some(value),
                size,
                bytes: self.bytes.clone(),
            })
            .map_err(|_| Error::Limit)
    }
}

impl Events {
    pub(crate) fn try_recv(&self) -> Result<Result<Value, String>, mpsc::TryRecvError> {
        self.0
            .try_recv()
            .map(|mut queued| queued.value.take().unwrap())
    }

    #[cfg(test)]
    pub(crate) fn try_iter(&self) -> impl Iterator<Item = Result<Value, String>> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }

    #[cfg(test)]
    pub(crate) fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Result<Value, String>, mpsc::RecvTimeoutError> {
        self.0
            .recv_timeout(timeout)
            .map(|mut queued| queued.value.take().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_consumer_has_a_byte_budget_that_receive_and_drop_release() {
        let (sender, events) = channel();
        let image = || serde_json::json!({"image_url": "A".repeat(LIVE_QUEUE_BYTES / 2)});
        sender.try_send(Ok(image())).unwrap();
        let retained = sender.bytes.load(Ordering::Relaxed);
        assert!(retained > LIVE_QUEUE_BYTES / 2);
        assert!(matches!(sender.try_send(Ok(image())), Err(Error::Limit)));
        assert_eq!(sender.bytes.load(Ordering::Relaxed), retained);
        assert!(events.try_recv().unwrap().is_ok());
        assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
        sender.try_send(Ok(image())).unwrap();
        drop(events);
        assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
        assert!(sender.try_send(Ok(serde_json::json!({}))).is_err());
        assert_eq!(sender.bytes.load(Ordering::Relaxed), 0);
    }
}
