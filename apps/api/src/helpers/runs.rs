//! `RunRegistry`: the chats that are answering right now (`docs/design/copilot.md`, Run
//! Registry).
//!
//! A run is reserved before its turn is prepared, so a chat answers once at a time and a user
//! has at most `MAX_ACTIVE_RUNS_PER_USER` runs. The slot is released when the run sends `done`
//! (so a client that saw `done` can send again at once) or when the reservation is dropped (a
//! failed `prepare`, or the engine's task ending). Its [`RunSink`] numbers every event
//! (`seq`), keeps it in the run's replay buffer, and broadcasts it to the attached streams. The
//! buffer merges consecutive deltas of one part into one event that carries the last merged
//! `seq`, so it grows with the answer's text (which the engine caps), not with the number of
//! deltas; a replay may skip `seq` values but never text.
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
    events: Vec<Buffered>,
    /// The last `seq` given out.
    seq: u64,
    /// The bytes of text each part has streamed so far.
    lengths: HashMap<PartKey, usize>,
    sender: broadcast::Sender<ChatStreamEvent>,
    done: bool,
}

/// A part of a message: its id and its index.
type PartKey = (Uuid, u32);

/// A buffered event. A text or reasoning delta may hold several merged deltas of its part; it
/// starts `start` bytes into the part's text.
#[derive(Clone, Debug)]
struct Buffered {
    event: ChatStreamEvent,
    start: usize,
}

impl Buffer {
    /// Keeps `event`, merging a delta into the previous one when both belong to the same part.
    fn keep(&mut self, event: &ChatStreamEvent) {
        let Some((key, text)) = delta(event) else {
            self.events.push(Buffered {
                event: event.clone(),
                start: 0,
            });
            return;
        };
        let length = self.lengths.entry(key).or_default();
        let start = *length;
        *length += text.len();
        if let Some(last) = self.events.last_mut()
            && same_part(&last.event, event)
        {
            merge(&mut last.event, text, event.seq());
            return;
        }
        self.events.push(Buffered {
            event: event.clone(),
            start,
        });
    }
}

/// The part and text of a text or reasoning delta.
fn delta(event: &ChatStreamEvent) -> Option<(PartKey, &str)> {
    match event {
        ChatStreamEvent::TextDelta {
            message_id,
            index,
            text,
            ..
        }
        | ChatStreamEvent::ReasoningDelta {
            message_id,
            index,
            text,
            ..
        } => Some(((*message_id, *index), text)),
        _ => None,
    }
}

/// Whether both are deltas of the same kind and part.
fn same_part(first: &ChatStreamEvent, second: &ChatStreamEvent) -> bool {
    std::mem::discriminant(first) == std::mem::discriminant(second)
        && matches!((delta(first), delta(second)), (Some((a, _)), Some((b, _))) if a == b)
}

/// Appends `more` to a delta and gives it `seq`.
fn merge(event: &mut ChatStreamEvent, more: &str, next: u64) {
    if let ChatStreamEvent::TextDelta { seq, text, .. }
    | ChatStreamEvent::ReasoningDelta { seq, text, .. } = event
    {
        text.push_str(more);
        *seq = next;
    }
}

/// A delta without its first `skip` bytes.
fn cut(mut event: ChatStreamEvent, skip: usize) -> ChatStreamEvent {
    if let ChatStreamEvent::TextDelta { text, .. } | ChatStreamEvent::ReasoningDelta { text, .. } =
        &mut event
    {
        *text = text.get(skip..).unwrap_or_default().to_owned();
    }
    event
}

impl RunSink {
    fn new(registry: Arc<RunRegistry>, chat: ChatId, run: Uuid) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            state: Arc::new(Mutex::new(Buffer {
                events: Vec::new(),
                seq: 0,
                lengths: HashMap::new(),
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
            delivered: HashMap::new(),
            finished: false,
        }
    }

    /// Events in the replay buffer.
    #[cfg(test)]
    fn buffered(&self) -> usize {
        lock(&self.state).events.len()
    }
}

impl TurnSink for RunSink {
    fn emit(&self, event: TurnEvent) {
        let mut buffer = lock(&self.state);
        if buffer.done {
            return;
        }
        buffer.seq += 1;
        let event = serializer::event(event, buffer.seq);
        buffer.done = matches!(event, ChatStreamEvent::Done { .. });
        if buffer.done {
            self.registry.release(self.chat, self.run);
        }
        buffer.keep(&event);
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
    replay: VecDeque<Buffered>,
    receiver: broadcast::Receiver<ChatStreamEvent>,
    state: Arc<Mutex<Buffer>>,
    /// The `seq` of the last event sent.
    last: u64,
    /// The bytes of each part's text sent so far, to cut a merged delta it has partly sent.
    delivered: HashMap<PartKey, usize>,
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
            if let Some(entry) = self.replay.pop_front() {
                if entry.event.seq() <= self.last {
                    continue;
                }
                if let Some(event) = self.rest(entry) {
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
                        .filter(|entry| entry.event.seq() > last)
                        .cloned()
                        .collect();
                }
                Err(RecvError::Closed) => return None,
            }
        };
        self.last = event.seq();
        if let Some((key, text)) = delta(&event) {
            *self.delivered.entry(key).or_default() += text.len();
        }
        self.finished = matches!(event, ChatStreamEvent::Done { .. });
        Some(event)
    }

    /// The part of a buffered event this stream has not sent yet (`None` when it sent it all).
    fn rest(&self, entry: Buffered) -> Option<ChatStreamEvent> {
        let Some((key, text)) = delta(&entry.event) else {
            return Some(entry.event);
        };
        let sent = self.delivered.get(&key).copied().unwrap_or_default();
        let skip = sent.saturating_sub(entry.start);
        if skip == 0 {
            Some(entry.event)
        } else if skip < text.len() {
            Some(cut(entry.event, skip))
        } else {
            None
        }
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

    fn delta(message_id: MessageId, index: u32, text: &str) -> TurnEvent {
        TurnEvent::TextDelta {
            message_id,
            index,
            text: text.to_owned(),
        }
    }

    /// The text of a stream's text deltas, and their `seq` values.
    fn texts(events: &[ChatStreamEvent]) -> (String, Vec<u64>) {
        events
            .iter()
            .filter_map(|event| match event {
                ChatStreamEvent::TextDelta { seq, text, .. } => Some((text.as_str(), *seq)),
                _ => None,
            })
            .fold(
                (String::new(), Vec::new()),
                |(mut all, mut seqs), (text, seq)| {
                    all.push_str(text);
                    seqs.push(seq);
                    (all, seqs)
                },
            )
    }

    #[tokio::test]
    async fn the_buffer_merges_the_deltas_of_a_part() {
        let registry = registry(3);
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        let sink = reservation.sink();
        let message = MessageId::generate();
        let mut expected = String::new();
        for index in 0..1_000 {
            let piece = (index % 10).to_string();
            expected.push_str(&piece);
            sink.emit(delta(message, 0, &piece));
        }
        sink.emit(TurnEvent::ReasoningDelta {
            message_id: message,
            index: 1,
            text: "why".to_owned(),
        });
        sink.emit(delta(message, 0, "!"));
        sink.emit(TurnEvent::Done);
        assert_eq!(sink.buffered(), 4, "text, reasoning, text, done");

        let late: Vec<ChatStreamEvent> = reservation.subscribe().into_stream().collect().await;
        let (text, seqs) = texts(&late);
        assert_eq!(text, format!("{expected}!"));
        assert_eq!(
            seqs,
            vec![1_000, 1_002],
            "each merged delta keeps its last seq"
        );
        assert_eq!(late.last().map(ChatStreamEvent::seq), Some(1_003));
    }

    #[tokio::test]
    async fn a_stream_behind_a_merged_delta_gets_only_the_text_it_missed() {
        let registry = registry(3);
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        let sink = reservation.sink();
        let message = MessageId::generate();
        let mut stream = Box::pin(reservation.subscribe().into_stream());
        let mut expected = String::new();
        for index in 0..10 {
            let piece = format!("{index},");
            expected.push_str(&piece);
            sink.emit(delta(message, 0, &piece));
        }
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(stream.next().await.expect("a live event"));
        }
        for index in 10..(CHANNEL_CAPACITY * 2) {
            let piece = format!("{index},");
            expected.push_str(&piece);
            sink.emit(delta(message, 0, &piece));
        }
        sink.emit(TurnEvent::Done);
        seen.extend(stream.collect::<Vec<_>>().await);

        let (text, seqs) = texts(&seen);
        assert_eq!(text, expected, "no text repeated or skipped");
        assert_eq!(seqs[..5], [1, 2, 3, 4, 5]);
        assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            seen.last().map(ChatStreamEvent::seq),
            Some(CHANNEL_CAPACITY as u64 * 2 + 1)
        );
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
