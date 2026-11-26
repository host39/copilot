//! `RunRegistry`: the chats that are answering right now (`docs/design/copilot.md`, Run
//! Registry).
//!
//! A run is reserved before its turn is prepared, so a chat answers once at a time and a user
//! has at most `MAX_ACTIVE_RUNS_PER_USER` runs. The slot is released when the run sends `done`
//! (so a client that saw `done` can send again at once) or when the reservation is dropped (a
//! failed `prepare`, or the engine's task ending). Its [`RunSink`] numbers every event
//! (`seq`), keeps it in the run's replay buffer, and broadcasts it to the attached streams.
//! Disconnecting a stream never cancels the run; the root shutdown token does (each run gets a
//! child token).
//!
//! The walking skeleton (P1.9) attaches the stream of the request that started the run. P3.1
//! adds reattaching (`GET /chats/{id}/stream`), stop, and the per-user event stream.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use copilot_contracts::ChatStreamEvent;
use copilot_core::shared::constants::MAX_ACTIVE_RUNS_PER_USER;
use copilot_core::{ChatId, DomainError, DomainResult, TurnEvent, TurnSink, UserId};
use futures_util::stream::{self, Stream};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::routes::chats::serializer;

/// Events a slow stream may fall behind before it catches up from the replay buffer.
const CHANNEL_CAPACITY: usize = 256;
/// The error code of a chat that is already answering.
pub const CHAT_BUSY: &str = "CHAT_BUSY";
/// The error code of a user with `MAX_ACTIVE_RUNS_PER_USER` runs.
pub const TOO_MANY_RUNS: &str = "TOO_MANY_RUNS";

/// The live runs, by chat. Cheap to share (`Arc`).
#[derive(Debug)]
pub struct RunRegistry {
    runs: Mutex<HashMap<ChatId, Entry>>,
    shutdown: CancellationToken,
    max_per_user: usize,
}

#[derive(Debug)]
struct Entry {
    owner: UserId,
    run: Uuid,
}

impl RunRegistry {
    /// Runs stop when `shutdown` is cancelled.
    pub fn new(shutdown: CancellationToken) -> Self {
        Self::with_limit(shutdown, MAX_ACTIVE_RUNS_PER_USER)
    }

    fn with_limit(shutdown: CancellationToken, max_per_user: usize) -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            shutdown,
            max_per_user,
        }
    }

    /// Reserves `chat` for a new run of `owner`. Load the chat with the caller's id first, so
    /// another user's chat is a `404`, never `CHAT_BUSY`.
    pub fn reserve(self: &Arc<Self>, owner: UserId, chat: ChatId) -> DomainResult<Reservation> {
        let mut runs = self.lock();
        if runs.contains_key(&chat) {
            return Err(DomainError::business_rule(
                "This chat is already answering. Stop it or wait for it to finish.",
                CHAT_BUSY,
            ));
        }
        let active = runs.values().filter(|entry| entry.owner == owner).count();
        if active >= self.max_per_user {
            return Err(DomainError::business_rule(
                format!("{active} chats are already answering. Wait for one to finish or stop it."),
                TOO_MANY_RUNS,
            ));
        }
        let run = Uuid::now_v7();
        runs.insert(chat, Entry { owner, run });
        Ok(Reservation {
            registry: Arc::clone(self),
            chat,
            run,
            sink: Arc::new(RunSink::new(Arc::clone(self), chat, run)),
            cancel: self.shutdown.child_token(),
        })
    }

    /// Whether `chat` is answering.
    pub fn is_running(&self, chat: ChatId) -> bool {
        self.lock().contains_key(&chat)
    }

    fn release(&self, chat: ChatId, run: Uuid) {
        let mut runs = self.lock();
        if runs.get(&chat).is_some_and(|entry| entry.run == run) {
            runs.remove(&chat);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ChatId, Entry>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A reserved slot. Dropping it releases the chat.
#[derive(Debug)]
pub struct Reservation {
    registry: Arc<RunRegistry>,
    chat: ChatId,
    run: Uuid,
    sink: Arc<RunSink>,
    cancel: CancellationToken,
}

impl Reservation {
    /// Where the engine sends the run's events.
    pub fn sink(&self) -> Arc<RunSink> {
        Arc::clone(&self.sink)
    }

    /// Cancelled on shutdown (and, from P3.1 on, by `POST /chats/{id}/stop`).
    pub fn cancel(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// The run's events so far, then live ones, ending with `done`.
    pub fn subscribe(&self) -> Subscription {
        self.sink.subscribe()
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.registry.release(self.chat, self.run);
    }
}

/// The run's events: numbered, buffered for replay, and broadcast. `done` releases the slot
/// before it is sent.
#[derive(Debug)]
pub struct RunSink {
    state: Arc<Mutex<Buffer>>,
    registry: Arc<RunRegistry>,
    chat: ChatId,
    run: Uuid,
}

#[derive(Debug)]
struct Buffer {
    events: Vec<ChatStreamEvent>,
    sender: broadcast::Sender<ChatStreamEvent>,
    done: bool,
}

impl RunSink {
    fn new(registry: Arc<RunRegistry>, chat: ChatId, run: Uuid) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            state: Arc::new(Mutex::new(Buffer {
                events: Vec::new(),
                sender,
                done: false,
            })),
            registry,
            chat,
            run,
        }
    }

    /// Whether the run sent its last event (`done`).
    pub fn is_done(&self) -> bool {
        lock(&self.state).done
    }

    fn subscribe(&self) -> Subscription {
        let buffer = lock(&self.state);
        Subscription {
            replay: buffer.events.iter().cloned().collect(),
            receiver: buffer.sender.subscribe(),
            state: Arc::clone(&self.state),
            last: 0,
            finished: false,
        }
    }
}

impl TurnSink for RunSink {
    fn emit(&self, event: TurnEvent) {
        let mut buffer = lock(&self.state);
        if buffer.done {
            return;
        }
        let seq = buffer.events.len() as u64 + 1;
        let event = serializer::event(event, seq);
        buffer.done = matches!(event, ChatStreamEvent::Done { .. });
        if buffer.done {
            self.registry.release(self.chat, self.run);
        }
        buffer.events.push(event.clone());
        // No receiver is not an error: the stream may have disconnected.
        let _ = buffer.sender.send(event);
    }
}

fn lock(state: &Mutex<Buffer>) -> MutexGuard<'_, Buffer> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One attached stream: the replay buffer first, then live events, without gaps or repeats.
#[derive(Debug)]
pub struct Subscription {
    replay: VecDeque<ChatStreamEvent>,
    receiver: broadcast::Receiver<ChatStreamEvent>,
    state: Arc<Mutex<Buffer>>,
    last: u64,
    finished: bool,
}

impl Subscription {
    /// The events in `seq` order, ending after `done`.
    pub fn into_stream(self) -> impl Stream<Item = ChatStreamEvent> + Send {
        stream::unfold(self, |mut subscription| async move {
            let event = subscription.next().await?;
            Some((event, subscription))
        })
    }

    async fn next(&mut self) -> Option<ChatStreamEvent> {
        if self.finished {
            return None;
        }
        let event = loop {
            if let Some(event) = self.replay.pop_front() {
                if event.seq() > self.last {
                    break event;
                }
                continue;
            }
            match self.receiver.recv().await {
                Ok(event) if event.seq() <= self.last => {}
                Ok(event) => break event,
                Err(RecvError::Lagged(_)) => {
                    // Fell behind the channel: catch up from the buffer, then go on live.
                    let last = self.last;
                    self.replay = lock(&self.state)
                        .events
                        .iter()
                        .filter(|event| event.seq() > last)
                        .cloned()
                        .collect();
                }
                Err(RecvError::Closed) => return None,
            }
        };
        self.last = event.seq();
        self.finished = matches!(event, ChatStreamEvent::Done { .. });
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use copilot_core::{MessageId, Role};
    use futures_util::StreamExt;

    use super::*;

    fn registry(limit: usize) -> Arc<RunRegistry> {
        Arc::new(RunRegistry::with_limit(CancellationToken::new(), limit))
    }

    fn text(index: u32) -> TurnEvent {
        TurnEvent::TextDelta {
            message_id: MessageId::generate(),
            index: 0,
            text: index.to_string(),
        }
    }

    #[test]
    fn a_chat_answers_once_at_a_time() {
        let registry = registry(3);
        let user = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let reservation = registry.reserve(user, chat).unwrap();
        let busy = registry.reserve(user, chat).unwrap_err();
        assert_eq!(busy.code(), CHAT_BUSY);
        assert!(registry.is_running(chat));
        drop(reservation);
        assert!(!registry.is_running(chat), "dropping releases the chat");
        registry.reserve(user, chat).unwrap();
    }

    #[test]
    fn done_frees_the_chat_before_anyone_sees_it() {
        let registry = registry(3);
        let user = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let reservation = registry.reserve(user, chat).unwrap();
        reservation.sink().emit(TurnEvent::Done);
        assert!(!registry.is_running(chat));
        let next = registry.reserve(user, chat).unwrap();
        drop(reservation);
        assert!(
            registry.is_running(chat),
            "the old reservation never releases the new run"
        );
        drop(next);
    }

    #[test]
    fn a_user_has_at_most_the_limit_of_runs() {
        let registry = registry(2);
        let user = UserId::new(Uuid::now_v7());
        let other = UserId::new(Uuid::now_v7());
        let _first = registry.reserve(user, ChatId::generate()).unwrap();
        let second = registry.reserve(user, ChatId::generate()).unwrap();
        let refused = registry.reserve(user, ChatId::generate()).unwrap_err();
        assert_eq!(refused.code(), TOO_MANY_RUNS);
        let _theirs = registry.reserve(other, ChatId::generate()).unwrap();
        drop(second);
        registry.reserve(user, ChatId::generate()).unwrap();
    }

    #[test]
    fn shutdown_cancels_every_run() {
        let shutdown = CancellationToken::new();
        let registry = Arc::new(RunRegistry::new(shutdown.clone()));
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        assert!(!reservation.cancel().is_cancelled());
        shutdown.cancel();
        assert!(reservation.cancel().is_cancelled());
    }

    #[tokio::test]
    async fn streams_replay_then_follow_live_events_in_order() {
        let registry = registry(3);
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        let sink = reservation.sink();
        sink.emit(TurnEvent::MessageStarted {
            message_id: MessageId::generate(),
            role: Role::Assistant,
            window_start_id: None,
        });
        let early = reservation.subscribe().into_stream();
        sink.emit(text(1));
        sink.emit(TurnEvent::Done);
        sink.emit(text(2));
        assert!(sink.is_done());

        let seqs: Vec<u64> = early.map(|event| event.seq()).collect().await;
        assert_eq!(seqs, vec![1, 2, 3], "nothing after done");
        let late: Vec<u64> = reservation
            .subscribe()
            .into_stream()
            .map(|event| event.seq())
            .collect()
            .await;
        assert_eq!(late, vec![1, 2, 3], "a late stream replays the run");
    }

    #[tokio::test]
    async fn a_stream_that_falls_behind_catches_up_from_the_buffer() {
        let registry = registry(3);
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        let sink = reservation.sink();
        let stream = reservation.subscribe().into_stream();
        let count = CHANNEL_CAPACITY as u32 * 2;
        for index in 0..count {
            sink.emit(text(index));
        }
        sink.emit(TurnEvent::Done);
        let seqs: Vec<u64> = stream.map(|event| event.seq()).collect().await;
        let expected: Vec<u64> = (1..=u64::from(count) + 1).collect();
        assert_eq!(seqs, expected);
    }
}
