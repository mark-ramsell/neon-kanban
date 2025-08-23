use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
    time::Instant,
};

use axum::response::sse::Event;
use futures::{StreamExt, TryStreamExt, future};
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_stream::wrappers::BroadcastStream;

use crate::{log_msg::LogMsg, stream_lines::LinesStreamExt};

// 100 MB Limit
const HISTORY_BYTES: usize = 100000 * 1024;

#[derive(Debug)]
pub struct MemoryMetrics {
    pub total_messages: usize,
    pub total_bytes: usize,
    pub oldest_message_age_secs: u64,
    pub broadcast_receivers: usize,
    pub history_capacity: usize,
}

#[derive(Clone)]
struct StoredMsg {
    msg: LogMsg,
    bytes: usize,
    timestamp: Instant,
}

struct Inner {
    history: VecDeque<StoredMsg>,
    total_bytes: usize,
    created_at: Instant,
}

pub struct MsgStore {
    inner: RwLock<Inner>,
    sender: broadcast::Sender<LogMsg>,
}

impl Default for MsgStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MsgStore {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(10000);
        Self {
            inner: RwLock::new(Inner {
                history: VecDeque::with_capacity(32),
                total_bytes: 0,
                created_at: Instant::now(),
            }),
            sender,
        }
    }

    pub fn push(&self, msg: LogMsg) {
        let _ = self.sender.send(msg.clone()); // live listeners
        let bytes = msg.approx_bytes();

        let mut inner = self.inner.write().unwrap();
        while inner.total_bytes.saturating_add(bytes) > HISTORY_BYTES {
            if let Some(front) = inner.history.pop_front() {
                inner.total_bytes = inner.total_bytes.saturating_sub(front.bytes);
            } else {
                break;
            }
        }
        inner.history.push_back(StoredMsg {
            msg,
            bytes,
            timestamp: Instant::now(),
        });
        inner.total_bytes = inner.total_bytes.saturating_add(bytes);
    }

    // Convenience
    pub fn push_stdout<S: Into<String>>(&self, s: S) {
        self.push(LogMsg::Stdout(s.into()));
    }
    pub fn push_stderr<S: Into<String>>(&self, s: S) {
        self.push(LogMsg::Stderr(s.into()));
    }
    pub fn push_patch(&self, patch: json_patch::Patch) {
        self.push(LogMsg::JsonPatch(patch));
    }

    pub fn push_session_id(&self, session_id: String) {
        self.push(LogMsg::SessionId(session_id));
    }

    pub fn push_finished(&self) {
        self.push(LogMsg::Finished);
    }

    pub fn get_receiver(&self) -> broadcast::Receiver<LogMsg> {
        self.sender.subscribe()
    }
    pub fn get_history(&self) -> Vec<LogMsg> {
        self.inner
            .read()
            .unwrap()
            .history
            .iter()
            .map(|s| s.msg.clone())
            .collect()
    }

    /// Get memory usage statistics
    pub fn get_memory_metrics(&self) -> MemoryMetrics {
        let inner = self.inner.read().unwrap();
        let now = Instant::now();

        let oldest_message_age_secs = inner
            .history
            .front()
            .map(|msg| now.duration_since(msg.timestamp).as_secs())
            .unwrap_or(0);

        MemoryMetrics {
            total_messages: inner.history.len(),
            total_bytes: inner.total_bytes,
            oldest_message_age_secs,
            broadcast_receivers: self.sender.receiver_count(),
            history_capacity: inner.history.capacity(),
        }
    }

    /// Force cleanup of old messages beyond age limit
    pub fn cleanup_old_messages(&self, max_age_secs: u64) {
        let mut inner = self.inner.write().unwrap();
        let now = Instant::now();
        let mut cleaned_count = 0;

        while let Some(front) = inner.history.front() {
            if now.duration_since(front.timestamp).as_secs() > max_age_secs {
                if let Some(old_msg) = inner.history.pop_front() {
                    inner.total_bytes = inner.total_bytes.saturating_sub(old_msg.bytes);
                    cleaned_count += 1;
                }
            } else {
                break;
            }
        }

        if cleaned_count > 0 {
            tracing::debug!("Cleaned up {} old messages", cleaned_count);
        }
    }

    /// History then live, as `LogMsg`.
    pub fn history_plus_stream(
        &self,
    ) -> futures::stream::BoxStream<'static, Result<LogMsg, std::io::Error>> {
        let (history, rx) = (self.get_history(), self.get_receiver());

        let hist = futures::stream::iter(history.into_iter().map(Ok::<_, std::io::Error>));
        let live = BroadcastStream::new(rx)
            .filter_map(|res| async move { res.ok().map(Ok::<_, std::io::Error>) });

        Box::pin(hist.chain(live))
    }

    pub fn stdout_chunked_stream(
        &self,
    ) -> futures::stream::BoxStream<'static, Result<String, std::io::Error>> {
        self.history_plus_stream()
            .take_while(|res| future::ready(!matches!(res, Ok(LogMsg::Finished))))
            .filter_map(|res| async move {
                match res {
                    Ok(LogMsg::Stdout(s)) => Some(Ok(s)),
                    _ => None,
                }
            })
            .boxed()
    }

    pub fn stdout_lines_stream(
        &self,
    ) -> futures::stream::BoxStream<'static, std::io::Result<String>> {
        self.stdout_chunked_stream().lines()
    }

    pub fn stderr_chunked_stream(
        &self,
    ) -> futures::stream::BoxStream<'static, Result<String, std::io::Error>> {
        self.history_plus_stream()
            .take_while(|res| future::ready(!matches!(res, Ok(LogMsg::Finished))))
            .filter_map(|res| async move {
                match res {
                    Ok(LogMsg::Stderr(s)) => Some(Ok(s)),
                    _ => None,
                }
            })
            .boxed()
    }

    pub fn stderr_lines_stream(
        &self,
    ) -> futures::stream::BoxStream<'static, std::io::Result<String>> {
        self.stderr_chunked_stream().lines()
    }

    /// Same stream but mapped to `Event` for SSE handlers.
    pub fn sse_stream(&self) -> futures::stream::BoxStream<'static, Result<Event, std::io::Error>> {
        self.history_plus_stream()
            .map_ok(|m| m.to_sse_event())
            .boxed()
    }

    /// Forward a stream of typed log messages into this store.
    pub fn spawn_forwarder<S, E>(self: Arc<Self>, stream: S) -> JoinHandle<()>
    where
        S: futures::Stream<Item = Result<LogMsg, E>> + Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        tokio::spawn(async move {
            tokio::pin!(stream);

            while let Some(next) = stream.next().await {
                match next {
                    Ok(msg) => self.push(msg),
                    Err(e) => self.push(LogMsg::Stderr(format!("stream error: {e}"))),
                }
            }
        })
    }

    /// Log current memory statistics
    pub fn log_memory_stats(&self) {
        let metrics = self.get_memory_metrics();
        tracing::info!(
            "MsgStore metrics - Messages: {}, Bytes: {}, Oldest: {}s, Receivers: {}, Capacity: {}",
            metrics.total_messages,
            metrics.total_bytes,
            metrics.oldest_message_age_secs,
            metrics.broadcast_receivers,
            metrics.history_capacity
        );
    }
}
