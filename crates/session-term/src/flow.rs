//! Output flow control.
//!
//! A protocol backend pushes [`Source`] items into a small bounded channel. [`forward`] drains
//! it, coalesces consecutive data up to [`COALESCE_BYTES`] per frame and never lets more than
//! [`MAX_UNACKED_BYTES`] be sent and not yet acknowledged (`term_ack`). While the credit is
//! exhausted the forwarder stops reading, the channel fills and the backend blocks: that is
//! the back-pressure. Only `Data` payload bytes are counted (not the frame tag).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use portolano_core_model::AppError;
use tokio::sync::{mpsc, watch, Notify};

use crate::session::FrameSink;
use crate::{TermEvent, COALESCE_BYTES};

/// What a backend reports. `Exit` and `Fatal` are terminal: nothing follows them.
#[derive(Debug)]
pub(crate) enum Source {
    Data(Vec<u8>),
    Exit(Option<i32>),
    Fatal(AppError),
}

/// Capacity of the backend -> forwarder channel, in items.
pub(crate) const SOURCE_QUEUE: usize = 8;

/// Bytes sent to the UI and not yet acknowledged.
pub(crate) struct Credit {
    limit: u64,
    unacked: AtomicU64,
    notify: Notify,
}

impl Credit {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit: limit as u64,
            unacked: AtomicU64::new(0),
            notify: Notify::new(),
        }
    }

    pub(crate) fn unacked(&self) -> u64 {
        self.unacked.load(Ordering::SeqCst)
    }

    /// The UI consumed `bytes`. Over-acknowledging cannot make the balance negative.
    pub(crate) fn ack(&self, bytes: u64) {
        let mut current = self.unacked.load(Ordering::SeqCst);
        loop {
            let next = current.saturating_sub(bytes);
            match self
                .unacked
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        self.notify.notify_one();
    }

    /// Waits until `n` more bytes fit in the window, then books them. There is a single
    /// reserving task (the forwarder), so check-then-add is not racy; `notify_one` stores a
    /// permit when nobody is waiting yet, so an ack between the check and the wait is not lost.
    async fn reserve(&self, n: u64) {
        loop {
            let current = self.unacked.load(Ordering::SeqCst);
            if current == 0 || current + n <= self.limit {
                self.unacked.fetch_add(n, Ordering::SeqCst);
                return;
            }
            self.notify.notified().await;
        }
    }
}

fn is_closed(closed: &watch::Receiver<bool>) -> bool {
    *closed.borrow()
}

/// Sends `pending` as frames of at most [`COALESCE_BYTES`] bytes. `false` = stop (the receiver
/// is gone or the session was closed while waiting for credit).
async fn flush(
    pending: &mut Vec<u8>,
    sink: &FrameSink,
    credit: &Credit,
    closed: &mut watch::Receiver<bool>,
) -> bool {
    let mut offset = 0;
    while offset < pending.len() {
        let end = (offset + COALESCE_BYTES).min(pending.len());
        let len = (end - offset) as u64;
        tokio::select! {
            () = credit.reserve(len) => {}
            _ = closed.wait_for(|c| *c) => return false,
        }
        let frame = TermEvent::Data(pending[offset..end].to_vec()).encode();
        if !sink(frame) {
            return false;
        }
        offset = end;
    }
    pending.clear();
    true
}

/// Drains `rx` into `sink` until a terminal item, the end of the channel, the closing of the
/// session or the loss of the receiver.
pub(crate) async fn forward(
    mut rx: mpsc::Receiver<Source>,
    sink: FrameSink,
    credit: Arc<Credit>,
    mut closed: watch::Receiver<bool>,
) {
    let mut pending: Vec<u8> = Vec::with_capacity(COALESCE_BYTES);
    loop {
        let item = tokio::select! {
            item = rx.recv() => item,
            _ = closed.wait_for(|c| *c) => return,
        };
        let Some(mut item) = item else { return };
        loop {
            match item {
                Source::Data(data) => {
                    pending.extend_from_slice(&data);
                    // Coalesce whatever is already queued, without waiting for more.
                    let next = loop {
                        if pending.len() >= COALESCE_BYTES {
                            break None;
                        }
                        match rx.try_recv() {
                            Ok(Source::Data(more)) => pending.extend_from_slice(&more),
                            Ok(other) => break Some(other),
                            Err(_) => break None,
                        }
                    };
                    if !flush(&mut pending, &sink, &credit, &mut closed).await {
                        return;
                    }
                    match next {
                        Some(other) => item = other,
                        None => break,
                    }
                }
                Source::Exit(code) => {
                    sink(TermEvent::Exit { code }.encode());
                    return;
                }
                Source::Fatal(err) => {
                    sink(TermEvent::Error(err).encode());
                    return;
                }
            }
        }
        if is_closed(&closed) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use portolano_core_model::ErrorCode;

    use super::*;
    use crate::{MAX_UNACKED_BYTES, TAG_DATA};

    type Frames = Arc<Mutex<Vec<Vec<u8>>>>;

    fn collecting_sink() -> (FrameSink, Frames) {
        let frames: Frames = Arc::default();
        let f = frames.clone();
        let sink: FrameSink = Arc::new(move |frame| {
            f.lock().expect("frames").push(frame);
            true
        });
        (sink, frames)
    }

    fn data_bytes(frames: &Frames) -> usize {
        frames
            .lock()
            .expect("frames")
            .iter()
            .filter(|f| f[0] == TAG_DATA)
            .map(|f| f.len() - 1)
            .sum()
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn coalesces_up_to_the_frame_size() {
        let (sink, frames) = collecting_sink();
        let (tx, rx) = mpsc::channel(64);
        let (_ctx, crx) = watch::channel(false);
        for _ in 0..40 {
            tx.send(Source::Data(vec![7; 5_000])).await.expect("send");
        }
        tx.send(Source::Exit(Some(0))).await.expect("send");
        forward(rx, sink, Arc::new(Credit::new(MAX_UNACKED_BYTES)), crx).await;
        let frames = frames.lock().expect("frames");
        // 200 000 bytes: frames of at most COALESCE_BYTES, followed by the exit frame.
        let data: Vec<&Vec<u8>> = frames.iter().filter(|f| f[0] == TAG_DATA).collect();
        assert!(data.iter().all(|f| f.len() - 1 <= COALESCE_BYTES));
        assert_eq!(data.iter().map(|f| f.len() - 1).sum::<usize>(), 200_000);
        assert!(
            data.len() <= 5,
            "expected coalescing, got {} frames",
            data.len()
        );
        assert_eq!(frames.last().expect("exit")[0], crate::TAG_EXIT);
    }

    #[tokio::test]
    async fn never_exceeds_the_credit_window_without_acks() {
        let (sink, frames) = collecting_sink();
        let credit = Arc::new(Credit::new(MAX_UNACKED_BYTES));
        let (tx, rx) = mpsc::channel(SOURCE_QUEUE);
        let (_ctx, crx) = watch::channel(false);
        let task = tokio::spawn(forward(rx, sink, credit.clone(), crx));

        // A producer that wants to send 4 MiB, in 16 KiB chunks.
        let producer = tokio::spawn(async move {
            for _ in 0..256 {
                if tx.send(Source::Data(vec![1; 16 * 1024])).await.is_err() {
                    return;
                }
            }
            let _ = tx.send(Source::Exit(Some(0))).await;
        });

        settle().await;
        assert!(data_bytes(&frames) <= MAX_UNACKED_BYTES);
        assert!(data_bytes(&frames) >= MAX_UNACKED_BYTES - COALESCE_BYTES);
        assert!(credit.unacked() <= MAX_UNACKED_BYTES as u64);
        assert!(!producer.is_finished(), "the producer must be blocked");

        // Acknowledge everything progressively: the stream completes, still within the window.
        let mut total_ack = 0u64;
        for _ in 0..200 {
            let sent = data_bytes(&frames) as u64;
            if sent > total_ack {
                credit.ack(sent - total_ack);
                total_ack = sent;
            }
            assert!(credit.unacked() <= MAX_UNACKED_BYTES as u64);
            if task.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        task.await.expect("forwarder");
        producer.await.expect("producer");
        assert_eq!(data_bytes(&frames), 256 * 16 * 1024);
    }

    #[tokio::test]
    async fn over_acknowledging_is_harmless() {
        let credit = Credit::new(100);
        credit.ack(50);
        assert_eq!(credit.unacked(), 0);
        credit.reserve(60).await;
        credit.ack(1_000);
        assert_eq!(credit.unacked(), 0);
    }

    #[tokio::test]
    async fn fatal_error_is_forwarded_after_the_data() {
        let (sink, frames) = collecting_sink();
        let (tx, rx) = mpsc::channel(8);
        let (_ctx, crx) = watch::channel(false);
        tx.send(Source::Data(vec![1, 2])).await.expect("send");
        tx.send(Source::Fatal(AppError::new(ErrorCode::AuthFailed, "no")))
            .await
            .expect("send");
        forward(rx, sink, Arc::new(Credit::new(MAX_UNACKED_BYTES)), crx).await;
        let frames = frames.lock().expect("frames");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], vec![TAG_DATA, 1, 2]);
        assert_eq!(frames[1][0], crate::TAG_ERROR);
    }

    #[tokio::test]
    async fn closing_releases_a_forwarder_waiting_for_credit() {
        let (sink, frames) = collecting_sink();
        let (tx, rx) = mpsc::channel(8);
        let (ctx, crx) = watch::channel(false);
        let credit = Arc::new(Credit::new(1_000));
        let task = tokio::spawn(forward(rx, sink, credit, crx));
        tx.send(Source::Data(vec![0; 900])).await.expect("send");
        settle().await;
        tx.send(Source::Data(vec![0; 900])).await.expect("send");
        settle().await;
        assert_eq!(data_bytes(&frames), 900);
        ctx.send(true).expect("close");
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("forwarder stops")
            .expect("join");
    }
}
