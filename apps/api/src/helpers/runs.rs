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
//! deltas; a replay may skip `seq` values but never text. Each merged event also keeps where
//! each of its deltas ended (4 bytes per delta), so a stream can start after any `seq` the run
//! sent without repeating or skipping text ([`RunRegistry::attach`]). A run whose buffer passes
//! `RUN_BUFFER_MAX_BYTES` is stopped.
//!
//! Streams read the shared buffer in place, one event at a time, and send a long merged delta
//! in pieces of about `REPLAY_CHUNK_BYTES` that end where one of its deltas ended (each with
//! that delta's `seq`), so an attached stream never holds a copy of the buffer. A user has at
//! most `CHAT_STREAMS_PER_USER` run streams and `EVENT_STREAMS_PER_USER` event streams open;
//! one more ends the user's oldest ([`Streams`]).
//!
//! Disconnecting a stream never cancels the run; [`RunRegistry::stop`] and shutdown do (each run
//! gets a child of the root shutdown token). [`RunRegistry::drain`] stops new runs, gives the
//! live ones a moment to finish, then cancels them and waits for them to save.
//!
//! Scheduled runs take one of the `MAX_SCHEDULED_RUNS` slots: a claimed fire takes a slot first
//! ([`RunRegistry::slot`]) and reserves its new chat with it ([`RunRegistry::reserve_scheduled`]),
//! and a Run now does both at once ([`RunRegistry::reserve_now`]), refused while its caller
//! holds a slot already, so one user's Run now never takes both slots and keeps other users'
//! schedules waiting ([`RunRegistry::holders`] lets the scheduler serve them first). Scheduled
//! runs are bounded by the slots, not by their owner's
//! `MAX_ACTIVE_RUNS_PER_USER`, so a claimed fire never fails because its owner is chatting, and
//! a user's chats never wait for their schedules. Otherwise they are runs like any other: one
//! per chat, stopped and drained the same way, with the same events.
//!
//! A reserved run is not visible until it starts ([`Reservation::start`], when its prepared turn
//! is launched, or its first event): until then `attach` answers idle and `watch` leaves it out,
//! so a reservation whose `prepare` fails is never attached or announced. A reservation dropped
//! after its run started but before `done` (its task panicked) ends the run's streams with
//! `error` and `done`.
//!
//! The registry also fans out each user's `GET /api/events` stream ([`RunRegistry::publish`],
//! [`RunRegistry::watch`]): a channel per user with a stream open, dropped once no stream reads
//! it. The sink publishes `run.started` when the run starts, and `approval.pending` (when the
//! run paused for an approval) and `run.finished` just before it frees the chat at `done`, so a
//! run started right after `done` always follows the last run's `run.finished`.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use copilot_contracts::{ChatStreamEvent, UserEvent};
use copilot_core::shared::constants::{
    CHAT_STREAMS_PER_USER, EVENT_STREAMS_PER_USER, MAX_ACTIVE_RUNS_PER_USER, MAX_SCHEDULED_RUNS,
    RUN_BUFFER_MAX_BYTES,
};
use copilot_core::{
    ChatId, DomainError, DomainResult, MessageStatus, ProviderId, TurnEvent, TurnOutcome, TurnSink,
    UserId,
};
use futures_util::stream::{self, Stream};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::helpers::streams::{Streams, Ticket};
use crate::routes::chats::serializer;
use crate::routes::convert::ToDto;

/// Events a slow stream may fall behind before it catches up from the replay buffer.
const CHANNEL_CAPACITY: usize = 256;
/// User events a slow `GET /api/events` stream may fall behind before it ends (the client
/// connects again and reads the chat list afresh).
const USER_CHANNEL_CAPACITY: usize = 64;
/// How much of a merged delta a replay sends at once, at least (it ends where one of the
/// merged deltas ended).
const REPLAY_CHUNK_BYTES: usize = 64 * 1_024;
/// What keeping an event costs beyond its text, for `RUN_BUFFER_MAX_BYTES`.
const EVENT_BYTES: usize = 64;
/// The error code of a chat that is already answering.
pub const CHAT_BUSY: &str = "CHAT_BUSY";
/// The error code of a user with `MAX_ACTIVE_RUNS_PER_USER` runs.
pub const TOO_MANY_RUNS: &str = "TOO_MANY_RUNS";
/// The error code of a run asked for while the server shuts down.
pub const SERVER_STOPPING: &str = "SERVER_STOPPING";
/// The error code of a Run now while `MAX_SCHEDULED_RUNS` scheduled runs are live, or while its
/// caller has one.
pub const SCHEDULER_BUSY: &str = "SCHEDULER_BUSY";

/// The live runs, by chat. Cheap to share (`Arc`).
#[derive(Debug)]
pub struct RunRegistry {
    runs: Mutex<Runs>,
    shutdown: CancellationToken,
    max_per_user: usize,
    /// How many runs are live, for [`RunRegistry::drain`].
    live: watch::Sender<usize>,
    /// The open `GET /api/events` streams, with each user's event channel.
    watchers: Arc<Streams<broadcast::Sender<UserEvent>>>,
    /// The open run streams.
    streams: Arc<Streams<()>>,
    /// The scheduled-run slots (`MAX_SCHEDULED_RUNS`).
    scheduled: Arc<Semaphore>,
}

#[derive(Debug, Default)]
struct Runs {
    entries: HashMap<ChatId, Entry>,
    /// When draining started: no new runs from then on.
    closing: Option<Instant>,
}

#[derive(Debug)]
struct Entry {
    owner: UserId,
    run: Uuid,
    /// A scheduled run: not counted against its owner's `MAX_ACTIVE_RUNS_PER_USER`.
    scheduled: bool,
    /// The run's own token (a child of the shutdown token).
    cancel: CancellationToken,
    /// Cancelled when the slot is released.
    released: CancellationToken,
    /// The run's events, for streams that attach later.
    state: Arc<Mutex<Buffer>>,
    /// The connection the run's model belongs to, once the turn is prepared.
    provider: Option<ProviderId>,
}

impl RunRegistry {
    /// Runs stop when `shutdown` is cancelled.
    pub fn new(shutdown: CancellationToken) -> Self {
        Self::with_limit(shutdown, MAX_ACTIVE_RUNS_PER_USER)
    }

    fn with_limit(shutdown: CancellationToken, max_per_user: usize) -> Self {
        Self {
            runs: Mutex::new(Runs::default()),
            shutdown,
            max_per_user,
            live: watch::Sender::new(0),
            watchers: Arc::new(Streams::new(EVENT_STREAMS_PER_USER)),
            streams: Arc::new(Streams::new(CHAT_STREAMS_PER_USER)),
            scheduled: Arc::new(Semaphore::new(MAX_SCHEDULED_RUNS)),
        }
    }

    /// Reserves `chat` for a new run of `owner`. Load the chat with the caller's id first, so
    /// another user's chat is a `404`, never `CHAT_BUSY`.
    pub fn reserve(self: &Arc<Self>, owner: UserId, chat: ChatId) -> DomainResult<Reservation> {
        let mut runs = self.lock();
        open(&runs, chat)?;
        let active = runs
            .entries
            .values()
            .filter(|entry| entry.owner == owner && !entry.scheduled)
            .count();
        if active >= self.max_per_user {
            return Err(DomainError::business_rule(
                format!("{active} chats are already answering. Wait for one to finish or stop it."),
                TOO_MANY_RUNS,
            ));
        }
        Ok(self.insert(&mut runs, owner, chat, None))
    }

    /// A free scheduled-run slot, held until the run it reserves ends; `None` while
    /// `MAX_SCHEDULED_RUNS` scheduled runs hold theirs.
    pub fn slot(&self) -> Option<Slot> {
        Arc::clone(&self.scheduled)
            .try_acquire_owned()
            .ok()
            .map(|permit| Slot { _permit: permit })
    }

    /// The owners of the live scheduled runs: each holds a scheduled-run slot.
    pub fn holders(&self) -> Vec<UserId> {
        let mut owners: Vec<UserId> = self
            .lock()
            .entries
            .values()
            .filter(|entry| entry.scheduled)
            .map(|entry| entry.owner)
            .collect();
        owners.sort_unstable_by_key(|owner| owner.uuid());
        owners.dedup();
        owners
    }

    /// Reserves the new `chat` of a Run now of `owner` in a free scheduled-run slot, at once.
    /// `SCHEDULER_BUSY` while `owner` holds a slot already (a Run now or a claimed fire) or
    /// `MAX_SCHEDULED_RUNS` scheduled runs are live.
    pub fn reserve_now(self: &Arc<Self>, owner: UserId, chat: ChatId) -> DomainResult<Reservation> {
        let mut runs = self.lock();
        open(&runs, chat)?;
        if runs
            .entries
            .values()
            .any(|entry| entry.scheduled && entry.owner == owner)
        {
            return Err(DomainError::business_rule(
                "One of your schedules is running. Try again when it finishes.",
                SCHEDULER_BUSY,
            ));
        }
        let slot = self.slot().ok_or_else(|| {
            DomainError::business_rule(
                format!(
                    "{MAX_SCHEDULED_RUNS} scheduled runs are running. Try again when one finishes."
                ),
                SCHEDULER_BUSY,
            )
        })?;
        Ok(self.insert(&mut runs, owner, chat, Some(slot)))
    }

    /// Reserves the new `chat` of a scheduled run of `owner` in `slot`. Not counted against the
    /// owner's `MAX_ACTIVE_RUNS_PER_USER`; fails only while the server shuts down (or for a chat
    /// that is answering, which a new chat never is).
    pub fn reserve_scheduled(
        self: &Arc<Self>,
        slot: Slot,
        owner: UserId,
        chat: ChatId,
    ) -> DomainResult<Reservation> {
        let mut runs = self.lock();
        open(&runs, chat)?;
        Ok(self.insert(&mut runs, owner, chat, Some(slot)))
    }

    /// Adds the run of `chat` (checked by [`open`]) and its reservation.
    fn insert(
        self: &Arc<Self>,
        runs: &mut Runs,
        owner: UserId,
        chat: ChatId,
        slot: Option<Slot>,
    ) -> Reservation {
        let run = Uuid::now_v7();
        let cancel = self.shutdown.child_token();
        let sink = Arc::new(RunSink::new(
            Arc::clone(self),
            owner,
            chat,
            run,
            cancel.clone(),
        ));
        runs.entries.insert(
            chat,
            Entry {
                owner,
                run,
                scheduled: slot.is_some(),
                cancel: cancel.clone(),
                released: CancellationToken::new(),
                state: Arc::clone(&sink.state),
                provider: None,
            },
        );
        self.live.send_replace(runs.entries.len());
        Reservation {
            registry: Arc::clone(self),
            chat,
            run,
            sink,
            cancel,
            _slot: slot,
        }
    }

    /// The running turn of `chat`: its events after `after` (all of them for `0`, or for a
    /// `seq` the run never sent), then live ones, ending with `done`. `None` when the chat is
    /// idle or its run has not started yet (it is still being prepared; the owner's event
    /// streams hear `run.started` when it starts). Load the chat with the caller's id first.
    pub fn attach(&self, chat: ChatId, after: u64) -> Option<Subscription> {
        // The buffer is locked after the registry lock is released: the sink takes them the
        // other way round when `done` releases the slot.
        let (owner, state) = {
            let runs = self.lock();
            let entry = runs.entries.get(&chat)?;
            (entry.owner, Arc::clone(&entry.state))
        };
        if !lock(&state).started {
            return None;
        }
        Some(subscribe(&state, after, self.ticket(owner)))
    }

    /// A place for a run stream of `owner` (see [`Streams`]).
    fn ticket(&self, owner: UserId) -> Ticket<()> {
        self.streams.open(owner, || (), |()| ()).0
    }

    /// The chats of `owner` that are answering now.
    pub fn chats(&self, owner: UserId) -> Vec<ChatId> {
        self.lock()
            .entries
            .iter()
            .filter(|(_, entry)| entry.owner == owner)
            .map(|(chat, _)| *chat)
            .collect()
    }

    /// The chats of `owner` whose runs answer with a model of `provider`.
    pub fn chats_on(&self, owner: UserId, provider: ProviderId) -> Vec<ChatId> {
        self.lock()
            .entries
            .iter()
            .filter(|(_, entry)| entry.owner == owner && entry.provider == Some(provider))
            .map(|(chat, _)| *chat)
            .collect()
    }

    /// Cancels the run of `chat`, if one is live, and waits up to `wait` for it to end (the
    /// engine saves the cancelled answer, then sends `done`, which releases the slot). `false`
    /// when the run was still live after `wait`.
    pub async fn stop(&self, chat: ChatId, wait: Duration) -> bool {
        let released = {
            let runs = self.lock();
            let Some(entry) = runs.entries.get(&chat) else {
                return true;
            };
            entry.cancel.cancel();
            entry.released.clone()
        };
        tokio::time::timeout(wait, released.cancelled())
            .await
            .is_ok()
    }

    /// Shuts the registry down: new runs fail with `SERVER_STOPPING` from now on; live runs get
    /// `grace` to finish on their own, then the shutdown token cancels them (each saves its
    /// partial answer as `cancelled`), and they get `limit` more to save. Both times count from
    /// the first call, so calling it again waits only for what is left. `false` when a run was
    /// still live at the end. New runs are refused from the call on, before the future is
    /// polled.
    pub fn drain(&self, grace: Duration, limit: Duration) -> impl Future<Output = bool> + '_ {
        let started = *self.lock().closing.get_or_insert_with(Instant::now);
        async move {
            if tokio::time::timeout_at(started + grace, self.idle())
                .await
                .is_err()
            {
                tracing::info!(runs = *self.live.borrow(), "api.runs.drain.cancelling");
            }
            self.shutdown.cancel();
            tokio::time::timeout_at(started + grace + limit, self.idle())
                .await
                .is_ok()
        }
    }

    /// Resolves once no run is live.
    async fn idle(&self) {
        let mut live = self.live.subscribe();
        // The sender lives as long as `self`, so waiting never fails.
        let _ = live.wait_for(|count| *count == 0).await;
    }

    /// Whether draining started ([`RunRegistry::drain`]): new runs are refused.
    pub fn is_closing(&self) -> bool {
        self.lock().closing.is_some()
    }

    /// Whether `chat` is answering.
    pub fn is_running(&self, chat: ChatId) -> bool {
        self.lock().entries.contains_key(&chat)
    }

    fn release(&self, chat: ChatId, run: Uuid) {
        let mut runs = self.lock();
        if runs
            .entries
            .get(&chat)
            .is_some_and(|entry| entry.run == run)
            && let Some(entry) = runs.entries.remove(&chat)
        {
            entry.released.cancel();
            self.live.send_replace(runs.entries.len());
        }
    }

    /// Sends `event` to every `GET /api/events` stream `user` has open (none is fine).
    pub fn publish(&self, user: UserId, event: UserEvent) {
        self.watchers.shared(user, |sender| {
            // No receiver is not an error: the last stream may be ending.
            let _ = sender.send(event);
        });
    }

    /// The user events of `owner`: `run.started` for each chat answering now, then every event
    /// published from the call on. The stream ends when it falls more than
    /// `USER_CHANNEL_CAPACITY` events behind (the client then connects again), or when the
    /// owner opens more than `EVENT_STREAMS_PER_USER` (the oldest ends).
    pub fn watch(&self, owner: UserId) -> UserEvents {
        let (ticket, receiver) = self.watchers.open(
            owner,
            || broadcast::channel(USER_CHANNEL_CAPACITY).0,
            broadcast::Sender::subscribe,
        );
        // Listed after subscribing, so a run that starts meanwhile is never missed (at worst
        // its `run.started` comes twice).
        let running = self
            .started(owner)
            .into_iter()
            .map(|chat| UserEvent::RunStarted {
                chat_id: chat.into(),
            })
            .collect();
        UserEvents {
            running,
            receiver,
            ticket,
        }
    }

    /// The chats of `owner` whose runs started and have not sent `done`.
    fn started(&self, owner: UserId) -> Vec<ChatId> {
        // The buffers are locked after the registry lock is released (see `attach`).
        let states: Vec<(ChatId, Arc<Mutex<Buffer>>)> = self
            .lock()
            .entries
            .iter()
            .filter(|(_, entry)| entry.owner == owner)
            .map(|(chat, entry)| (*chat, Arc::clone(&entry.state)))
            .collect();
        states
            .into_iter()
            .filter(|(_, state)| {
                let buffer = lock(state);
                buffer.started && !buffer.done
            })
            .map(|(chat, _)| chat)
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, Runs> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Refuses a new run of `chat` while the server shuts down or the chat is answering.
fn open(runs: &Runs, chat: ChatId) -> DomainResult<()> {
    if runs.closing.is_some() {
        return Err(DomainError::business_rule(
            "The server is restarting. Try again in a moment.",
            SERVER_STOPPING,
        ));
    }
    if runs.entries.contains_key(&chat) {
        return Err(DomainError::business_rule(
            "This chat is already answering. Stop it or wait for it to finish.",
            CHAT_BUSY,
        ));
    }
    Ok(())
}

/// One of the `MAX_SCHEDULED_RUNS` scheduled-run slots ([`RunRegistry::slot`]); dropping it
/// frees the slot.
#[derive(Debug)]
pub struct Slot {
    _permit: OwnedSemaphorePermit,
}

/// One `GET /api/events` stream (see [`RunRegistry::watch`]).
#[derive(Debug)]
pub struct UserEvents {
    running: VecDeque<UserEvent>,
    receiver: broadcast::Receiver<UserEvent>,
    ticket: Ticket<broadcast::Sender<UserEvent>>,
}

impl UserEvents {
    /// The events in order; ends when the stream fell behind or a newer stream of the user took
    /// its place.
    pub fn into_stream(self) -> impl Stream<Item = UserEvent> + Send {
        stream::unfold(self, |mut events| async move {
            if events.ticket.ended().is_cancelled() {
                return None;
            }
            if let Some(event) = events.running.pop_front() {
                return Some((event, events));
            }
            let received = tokio::select! {
                biased;
                () = events.ticket.ended().cancelled() => return None,
                received = events.receiver.recv() => received,
            };
            match received {
                Ok(event) => Some((event, events)),
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "api.events.lagged");
                    None
                }
                Err(RecvError::Closed) => None,
            }
        })
    }
}

/// A reserved slot. Dropping it releases the chat (and the scheduled-run slot of a scheduled
/// run).
#[derive(Debug)]
pub struct Reservation {
    registry: Arc<RunRegistry>,
    chat: ChatId,
    run: Uuid,
    sink: Arc<RunSink>,
    cancel: CancellationToken,
    /// Held for the run's life.
    _slot: Option<Slot>,
}

impl Reservation {
    /// Where the engine sends the run's events.
    pub fn sink(&self) -> Arc<RunSink> {
        Arc::clone(&self.sink)
    }

    /// Cancelled on shutdown and by [`RunRegistry::stop`].
    pub fn cancel(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// The run's events so far, then live ones, ending with `done`.
    pub fn subscribe(&self) -> Subscription {
        let ticket = self.registry.ticket(self.sink.owner);
        subscribe(&self.sink.state, 0, ticket)
    }

    /// Makes the run visible: streams may attach and the owner's event streams hear
    /// `run.started`. Call it when the prepared turn is launched; the first event also does.
    pub fn start(&self) {
        self.sink.start();
    }

    /// Records the connection the run answers with, so deleting it stops the run first
    /// ([`RunRegistry::chats_on`]).
    pub fn serve(&self, provider: ProviderId) {
        let mut runs = self.registry.lock();
        if let Some(entry) = runs.entries.get_mut(&self.chat)
            && entry.run == self.sink.run
        {
            entry.provider = Some(provider);
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.sink.close();
        self.registry.release(self.chat, self.run);
    }
}

/// The run's events: numbered, buffered for replay, and broadcast. `done` releases the slot
/// before it is sent. The owner's event streams get `run.started` when the run starts, and
/// `approval.pending` and `run.finished` with `done`.
#[derive(Debug)]
pub struct RunSink {
    state: Arc<Mutex<Buffer>>,
    registry: Arc<RunRegistry>,
    owner: UserId,
    chat: ChatId,
    run: Uuid,
    /// The run's token: cancelled when the buffer passes `RUN_BUFFER_MAX_BYTES`.
    cancel: CancellationToken,
}

#[derive(Debug)]
struct Buffer {
    events: Vec<Buffered>,
    /// The last `seq` given out.
    seq: u64,
    /// The bytes of text each part has streamed so far.
    lengths: HashMap<PartKey, usize>,
    /// Roughly what the kept events cost (see [`weight`]), for `RUN_BUFFER_MAX_BYTES`.
    bytes: usize,
    /// The buffer passed `RUN_BUFFER_MAX_BYTES` and the run was stopped.
    full: bool,
    sender: broadcast::Sender<ChatStreamEvent>,
    /// The run started ([`RunSink::start`]): streams may attach, and `run.started` went out.
    started: bool,
    done: bool,
    /// How the run ended (`TurnSink::ended`); none when it failed with an error.
    outcome: Option<TurnOutcome>,
}

/// A part of a message: its id and its index.
type PartKey = (Uuid, u32);

/// A buffered event. A text or reasoning delta may hold several merged deltas of its part (with
/// the `seq` values `first` to its own); it starts `start` bytes into the part's text, and
/// `ends` holds where each merged delta ended, counted from `start`.
#[derive(Clone, Debug)]
struct Buffered {
    event: ChatStreamEvent,
    start: usize,
    first: u64,
    ends: Vec<u32>,
}

impl Buffer {
    /// Keeps `event`, merging a delta into the previous one when both belong to the same part.
    fn keep(&mut self, event: &ChatStreamEvent) {
        let seq = event.seq();
        let Some((key, text)) = delta(event) else {
            self.bytes += EVENT_BYTES + weight(event);
            self.events.push(Buffered {
                event: event.clone(),
                start: 0,
                first: seq,
                ends: Vec::new(),
            });
            return;
        };
        let length = self.lengths.entry(key).or_default();
        let start = *length;
        *length += text.len();
        self.bytes += text.len() + size_of::<u32>();
        if let Some(last) = self.events.last_mut()
            && same_part(&last.event, event)
        {
            merge(&mut last.event, text, seq);
            let end = last.ends.last().copied().unwrap_or_default();
            last.ends.push(end.saturating_add(width(text)));
            return;
        }
        self.bytes += EVENT_BYTES;
        self.events.push(Buffered {
            event: event.clone(),
            start,
            first: seq,
            ends: vec![width(text)],
        });
    }

    /// The bytes of each part's text that the events up to `after` carried.
    fn sent(&self, after: u64) -> HashMap<PartKey, usize> {
        let mut sent = HashMap::new();
        for entry in self.events.iter().filter(|entry| entry.first <= after) {
            let Some((key, text)) = delta(&entry.event) else {
                continue;
            };
            let length = if entry.event.seq() <= after {
                text.len()
            } else {
                let merged = usize::try_from(after - entry.first).unwrap_or(usize::MAX);
                entry
                    .ends
                    .get(merged)
                    .map_or(text.len(), |end| *end as usize)
            };
            sent.insert(key, entry.start + length);
        }
        sent
    }

    /// The index of the first event with a `seq` after `after` (a merged delta that a stream
    /// sent part of included).
    fn position(&self, after: u64) -> usize {
        self.events
            .partition_point(|entry| entry.event.seq() <= after)
    }
}

/// Roughly what keeping an event other than a text or reasoning delta costs beyond
/// [`EVENT_BYTES`]: its audio, or its tool call's arguments or output.
fn weight(event: &ChatStreamEvent) -> usize {
    match event {
        ChatStreamEvent::AudioDelta { data, .. } => data.len(),
        ChatStreamEvent::ToolRequested { .. } | ChatStreamEvent::ToolFinished { .. } => {
            serde_json::to_vec(event).map_or(0, |bytes| bytes.len())
        }
        _ => 0,
    }
}

/// The length of a delta's text for [`Buffered::ends`] (an answer is far below 4 GiB).
fn width(text: &str) -> u32 {
    u32::try_from(text.len()).unwrap_or(u32::MAX)
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

/// The bytes `from..end` of a delta's text, as a delta with `seq`.
fn slice(event: &ChatStreamEvent, from: usize, end: usize, seq: u64) -> ChatStreamEvent {
    let part = |text: &str| text.get(from..end).unwrap_or_default().to_owned();
    match event {
        ChatStreamEvent::TextDelta {
            message_id,
            index,
            text,
            ..
        } => ChatStreamEvent::TextDelta {
            seq,
            message_id: *message_id,
            index: *index,
            text: part(text),
        },
        ChatStreamEvent::ReasoningDelta {
            message_id,
            index,
            text,
            ..
        } => ChatStreamEvent::ReasoningDelta {
            seq,
            message_id: *message_id,
            index: *index,
            text: part(text),
        },
        other => other.clone(),
    }
}

/// The next piece of `entry` that a stream which sent `delivered` (and everything up to `last`)
/// has not sent, and whether it is the entry's last piece; `None` when it sent all of it. Any
/// event but a delta is one piece. A merged delta goes in pieces of at least
/// `REPLAY_CHUNK_BYTES` (or what is left), each ending where one of its deltas ended and
/// carrying that delta's `seq`.
fn piece(
    entry: &Buffered,
    delivered: &HashMap<PartKey, usize>,
    last: u64,
) -> Option<(ChatStreamEvent, bool)> {
    let seq = entry.event.seq();
    if seq <= last {
        return None;
    }
    let Some((key, text)) = delta(&entry.event) else {
        return Some((entry.event.clone(), true));
    };
    let from = delivered
        .get(&key)
        .copied()
        .unwrap_or_default()
        .saturating_sub(entry.start);
    if from >= text.len() {
        return None;
    }
    let goal = (from + REPLAY_CHUNK_BYTES).min(text.len());
    let index = entry.ends.partition_point(|end| (*end as usize) < goal);
    let (end, through) = match entry.ends.get(index) {
        Some(&end) if (end as usize) > from && (end as usize) <= text.len() => (
            end as usize,
            entry.first + u64::try_from(index).unwrap_or(u64::MAX),
        ),
        _ => (text.len(), seq),
    };
    let whole = end == text.len() && through == seq;
    Some((slice(&entry.event, from, end, through.min(seq)), whole))
}

fn is_done(event: &ChatStreamEvent) -> bool {
    matches!(event, ChatStreamEvent::Done { .. })
}

impl RunSink {
    fn new(
        registry: Arc<RunRegistry>,
        owner: UserId,
        chat: ChatId,
        run: Uuid,
        cancel: CancellationToken,
    ) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            state: Arc::new(Mutex::new(Buffer {
                events: Vec::new(),
                seq: 0,
                lengths: HashMap::new(),
                bytes: 0,
                full: false,
                sender,
                started: false,
                done: false,
                outcome: None,
            })),
            registry,
            owner,
            chat,
            run,
            cancel,
        }
    }

    /// Tells the owner's event streams how the run ended: `approval.pending` when it paused for
    /// an approval, then `run.finished` (`error` without an outcome).
    fn finished(&self, outcome: Option<&TurnOutcome>) {
        let chat_id = self.chat.into();
        let status = outcome.map_or(MessageStatus::Error, |outcome| outcome.status);
        if status == MessageStatus::AwaitingApproval
            && let Some(message) = outcome.and_then(|outcome| outcome.message_id)
        {
            self.registry.publish(
                self.owner,
                UserEvent::ApprovalPending {
                    chat_id,
                    message_id: message.into(),
                },
            );
        }
        self.registry.publish(
            self.owner,
            UserEvent::RunFinished {
                chat_id,
                status: status.to_dto(),
            },
        );
    }

    /// Whether the run sent its last event (`done`).
    #[cfg(test)]
    fn is_done(&self) -> bool {
        lock(&self.state).done
    }

    /// Starts the run (see [`Reservation::start`]); once only.
    fn start(&self) {
        let mut buffer = lock(&self.state);
        if !buffer.done {
            self.begin(&mut buffer);
        }
    }

    /// Marks the run started and tells the owner's event streams, the first time.
    fn begin(&self, buffer: &mut Buffer) {
        if buffer.started {
            return;
        }
        buffer.started = true;
        self.registry.publish(
            self.owner,
            UserEvent::RunStarted {
                chat_id: self.chat.into(),
            },
        );
    }

    /// Ends a run whose reservation is dropped: one that started but never sent `done` (its
    /// task panicked) sends `error` and `done`, so its streams end and its owner hears
    /// `run.finished`; one that never started (a failed `prepare`) ends silently.
    fn close(&self) {
        {
            let mut buffer = lock(&self.state);
            if buffer.done {
                return;
            }
            if !buffer.started {
                buffer.done = true;
                return;
            }
        }
        self.emit(TurnEvent::Error {
            message: "Something went wrong. Please try again.".to_owned(),
            code: "INTERNAL_ERROR".to_owned(),
        });
        self.emit(TurnEvent::Done);
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
        self.begin(&mut buffer);
        let event = serializer::event(event, buffer.seq);
        buffer.done = is_done(&event);
        if buffer.done {
            // Before the chat is free: a run that starts right after follows these events.
            self.finished(buffer.outcome.as_ref());
            self.registry.release(self.chat, self.run);
        }
        buffer.keep(&event);
        if buffer.bytes > RUN_BUFFER_MAX_BYTES && !buffer.full {
            buffer.full = true;
            tracing::warn!(chat_id = %self.chat, bytes = buffer.bytes, "api.runs.buffer.full");
            self.cancel.cancel();
        }
        // No receiver is not an error: the stream may have disconnected.
        let _ = buffer.sender.send(event);
    }

    fn ended(&self, outcome: &TurnOutcome) {
        lock(&self.state).outcome = Some(outcome.clone());
    }
}

fn lock(state: &Mutex<Buffer>) -> MutexGuard<'_, Buffer> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A stream of the run's events after `after` (see [`RunRegistry::attach`]), holding `ticket`.
fn subscribe(state: &Arc<Mutex<Buffer>>, after: u64, ticket: Ticket<()>) -> Subscription {
    let buffer = lock(state);
    // A `seq` this run never sent belongs to another run: replay this one from the start.
    let after = if after > buffer.seq { 0 } else { after };
    Subscription {
        receiver: buffer.sender.subscribe(),
        next: Some(buffer.position(after)),
        delivered: buffer.sent(after),
        state: Arc::clone(state),
        last: after,
        finished: false,
        ticket,
    }
}

/// One attached stream: the replay buffer first, then live events, without gaps or repeats. It
/// reads the shared buffer one piece at a time and never copies it.
#[derive(Debug)]
pub struct Subscription {
    state: Arc<Mutex<Buffer>>,
    receiver: broadcast::Receiver<ChatStreamEvent>,
    /// The buffered event to send next while the stream replays the buffer; `None` once it
    /// follows live events.
    next: Option<usize>,
    /// The `seq` of the last event sent (or that the client already had).
    last: u64,
    /// The bytes of each part's text sent so far (or that the client already had), to cut a
    /// merged delta it has partly sent.
    delivered: HashMap<PartKey, usize>,
    finished: bool,
    /// Its place among the owner's streams: cancelled when a newer stream took it.
    ticket: Ticket<()>,
}

/// What the replay buffer has for a stream.
enum Replay {
    Event(ChatStreamEvent),
    /// The stream sent the whole buffer and goes on live.
    Live,
    /// The run is over and the stream sent (or the client had) all of it.
    Finished,
}

impl Subscription {
    /// The events in `seq` order, ending after `done` (or when a newer stream of the owner took
    /// this one's place).
    pub fn into_stream(self) -> impl Stream<Item = ChatStreamEvent> + Send {
        stream::unfold(self, |mut subscription| async move {
            let event = subscription.next().await?;
            Some((event, subscription))
        })
    }

    async fn next(&mut self) -> Option<ChatStreamEvent> {
        if self.finished || self.ticket.ended().is_cancelled() {
            return None;
        }
        let event = loop {
            if self.next.is_some() {
                match self.replay() {
                    Replay::Event(event) => break event,
                    Replay::Finished => {
                        self.finished = true;
                        return None;
                    }
                    Replay::Live => {}
                }
            }
            let received = tokio::select! {
                biased;
                () = self.ticket.ended().cancelled() => return None,
                received = self.receiver.recv() => received,
            };
            match received {
                Ok(event) if event.seq() <= self.last => {}
                Ok(event) => break event,
                Err(RecvError::Lagged(_)) => {
                    // Fell behind the channel: catch up from the buffer, then go on live.
                    self.next = Some(lock(&self.state).position(self.last));
                }
                Err(RecvError::Closed) => return None,
            }
        };
        self.last = event.seq();
        if let Some((key, text)) = delta(&event) {
            *self.delivered.entry(key).or_default() += text.len();
        }
        self.finished = is_done(&event);
        Some(event)
    }

    /// The next piece of the replay buffer this stream has not sent, holding the buffer's lock
    /// only while it cuts that piece.
    fn replay(&mut self) -> Replay {
        let buffer = lock(&self.state);
        while let Some(index) = self.next {
            let Some(entry) = buffer.events.get(index) else {
                self.next = None;
                return if buffer.done {
                    Replay::Finished
                } else {
                    Replay::Live
                };
            };
            match piece(entry, &self.delivered, self.last) {
                Some((event, whole)) => {
                    if whole {
                        self.next = Some(index + 1);
                    }
                    return Replay::Event(event);
                }
                None => self.next = Some(index + 1),
            }
        }
        Replay::Live
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

    #[tokio::test]
    async fn stopping_a_run_cancels_it_and_waits_until_it_ends() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let other = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let theirs = ChatId::generate();
        let reservation = registry.reserve(owner, chat).unwrap();
        let _their_run = registry.reserve(other, theirs).unwrap();
        assert_eq!(registry.chats(owner), [chat]);

        let cancel = reservation.cancel();
        let run = tokio::spawn(async move {
            cancel.cancelled().await;
            // The engine saves the cancelled answer, then the slot is released.
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(reservation);
        });
        assert!(registry.stop(chat, Duration::from_secs(5)).await);
        assert!(
            !registry.is_running(chat),
            "the run ended before stop returned"
        );
        assert!(registry.chats(owner).is_empty());
        assert!(registry.is_running(theirs), "other runs go on");
        run.await.unwrap();
        assert!(
            registry.stop(ChatId::generate(), Duration::ZERO).await,
            "a chat without a run is stopped already"
        );
    }

    #[tokio::test]
    async fn stop_gives_up_after_its_wait() {
        let registry = registry(3);
        let chat = ChatId::generate();
        let reservation = registry.reserve(UserId::new(Uuid::now_v7()), chat).unwrap();
        assert!(!registry.stop(chat, Duration::from_millis(10)).await);
        assert!(reservation.cancel().is_cancelled(), "but it was cancelled");
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
    fn scheduled_runs_take_a_slot_and_do_not_count_against_their_owner() {
        let registry = registry(1);
        let user = UserId::new(Uuid::now_v7());
        let slots: Vec<Slot> = std::iter::from_fn(|| registry.slot()).take(5).collect();
        assert_eq!(
            slots.len(),
            MAX_SCHEDULED_RUNS,
            "at most the scheduled-run slots"
        );
        assert_eq!(
            registry
                .reserve_now(UserId::new(Uuid::now_v7()), ChatId::generate())
                .unwrap_err()
                .code(),
            SCHEDULER_BUSY,
            "a Run now waits for a slot"
        );
        let mut scheduled = Vec::new();
        for slot in slots {
            let reservation = registry
                .reserve_scheduled(slot, user, ChatId::generate())
                .unwrap();
            scheduled.push(reservation);
        }
        let _chat = registry
            .reserve(user, ChatId::generate())
            .expect("the owner's own chats do not wait for their schedules");
        assert_eq!(registry.chats(user).len(), 3);

        let refused = registry.reserve(user, ChatId::generate()).unwrap_err();
        assert_eq!(refused.code(), TOO_MANY_RUNS, "the limit counts chats only");

        drop(scheduled.pop());
        let slot = registry.slot().expect("a run that ends frees its slot");
        let _scheduled = registry
            .reserve_scheduled(slot, user, ChatId::generate())
            .unwrap();
    }

    #[test]
    fn a_run_now_waits_while_its_caller_holds_a_scheduled_run() {
        let registry = registry(3);
        let alice = UserId::new(Uuid::now_v7());
        let bob = UserId::new(Uuid::now_v7());
        assert!(registry.holders().is_empty());
        let first = registry.reserve_now(alice, ChatId::generate()).unwrap();
        assert_eq!(registry.holders(), [alice]);
        let refused = registry.reserve_now(alice, ChatId::generate()).unwrap_err();
        assert_eq!(refused.code(), SCHEDULER_BUSY, "one slot per caller");
        assert!(registry.slot().is_some(), "the other slot stays free");
        let bobs = registry.reserve_now(bob, ChatId::generate()).unwrap();
        let mut holders = vec![alice, bob];
        holders.sort_unstable_by_key(|owner| owner.uuid());
        assert_eq!(registry.holders(), holders);

        // A claimed fire of Alice's counts too.
        drop(first);
        let slot = registry.slot().expect("Alice's slot is free again");
        let _fire = registry
            .reserve_scheduled(slot, alice, ChatId::generate())
            .unwrap();
        assert_eq!(
            registry
                .reserve_now(alice, ChatId::generate())
                .unwrap_err()
                .code(),
            SCHEDULER_BUSY
        );
        drop(bobs);
        assert_eq!(registry.holders(), [alice]);
    }

    #[tokio::test]
    async fn draining_refuses_scheduled_runs_too() {
        let registry = registry(3);
        let slot = registry.slot().unwrap();
        assert!(!registry.is_closing());
        let _drain = registry.drain(Duration::ZERO, Duration::ZERO);
        assert!(registry.is_closing());
        let refused = registry
            .reserve_scheduled(slot, UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap_err();
        assert_eq!(refused.code(), SERVER_STOPPING);
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
            unavailable: Vec::new(),
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
        let mut seen = Vec::new();
        // The stream keeps up with the first five deltas, which the buffer merges.
        for index in 0..5 {
            let piece = format!("{index},");
            expected.push_str(&piece);
            sink.emit(delta(message, 0, &piece));
            seen.push(stream.next().await.expect("a live event"));
        }
        for index in 5..(CHANNEL_CAPACITY * 2) {
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
    async fn a_long_merged_delta_replays_in_pieces_that_end_on_its_deltas() {
        let registry = registry(3);
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        let sink = reservation.sink();
        let message = MessageId::generate();
        sink.emit(TurnEvent::MessageStarted {
            message_id: message,
            role: Role::Assistant,
            window_start_id: None,
            unavailable: Vec::new(),
        });
        let piece = "x".repeat(1_000);
        let count = 200;
        for _ in 0..count {
            sink.emit(delta(message, 0, &piece));
        }
        sink.emit(TurnEvent::Done);
        assert_eq!(sink.buffered(), 3, "started, one merged delta, done");

        let late: Vec<ChatStreamEvent> = reservation.subscribe().into_stream().collect().await;
        let pieces: Vec<(usize, u64)> = late
            .iter()
            .filter_map(|event| match event {
                ChatStreamEvent::TextDelta { seq, text, .. } => Some((text.len(), *seq)),
                _ => None,
            })
            .collect();
        let mut sent = 0;
        for (index, (length, seq)) in pieces.iter().enumerate() {
            assert_eq!(length % piece.len(), 0, "pieces end where a delta ended");
            if index + 1 < pieces.len() {
                assert!(
                    *length >= REPLAY_CHUNK_BYTES && *length < REPLAY_CHUNK_BYTES + piece.len()
                );
            }
            sent += length;
            assert_eq!(
                *seq,
                1 + (sent / piece.len()) as u64,
                "each piece has the seq of its last delta"
            );
        }
        assert!(pieces.len() > 1);
        assert_eq!(sent, piece.len() * count);
        assert_eq!(
            late.last().map(ChatStreamEvent::seq),
            Some(count as u64 + 2)
        );

        // After a seq inside a piece: exactly the rest, still in pieces.
        let after = 100;
        let rest: Vec<ChatStreamEvent> = reservation_stream(&reservation, after).collect().await;
        let (text, seqs) = texts(&rest);
        assert_eq!(text.len(), piece.len() * (count - (after as usize - 1)));
        assert_eq!(seqs.last(), Some(&(count as u64 + 1)));
        assert!(seqs.iter().all(|seq| *seq > after));
    }

    #[test]
    fn the_buffer_counts_each_delta_end_and_a_full_buffer_stops_the_run() {
        let registry = registry(3);
        let reservation = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        let sink = reservation.sink();
        let message = MessageId::generate();
        for _ in 0..10 {
            sink.emit(delta(message, 0, "x"));
        }
        assert_eq!(
            lock(&sink.state).bytes,
            EVENT_BYTES + 10 * (1 + size_of::<u32>()),
            "one event, ten bytes of text, ten ends"
        );
        let large = "y".repeat(1_024 * 1_024);
        while lock(&sink.state).bytes <= RUN_BUFFER_MAX_BYTES {
            assert!(!reservation.cancel().is_cancelled());
            sink.emit(delta(message, 0, &large));
        }
        assert!(
            reservation.cancel().is_cancelled(),
            "a run whose buffer passes the limit is stopped"
        );
        // The engine saves the stopped answer and ends the run as usual.
        sink.emit(TurnEvent::Done);
        assert!(sink.is_done());
    }

    #[tokio::test]
    async fn a_user_past_the_stream_limits_loses_the_oldest_stream() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let reservation = registry.reserve(owner, chat).unwrap();
        reservation.start();
        let first = registry.attach(chat, 0).expect("started").into_stream();
        let mut others: Vec<Subscription> = (1..CHAT_STREAMS_PER_USER)
            .map(|_| registry.attach(chat, 0).expect("started"))
            .collect();
        assert_eq!(registry.streams.count(owner), CHAT_STREAMS_PER_USER);
        others.push(reservation.subscribe());
        assert_eq!(registry.streams.count(owner), CHAT_STREAMS_PER_USER);
        let ended: Vec<ChatStreamEvent> =
            tokio::time::timeout(Duration::from_secs(2), first.collect())
                .await
                .expect("the oldest run stream ends");
        assert!(ended.is_empty());

        let watched = registry.watch(owner).into_stream();
        let _rest: Vec<UserEvents> = (1..EVENT_STREAMS_PER_USER)
            .map(|_| registry.watch(owner))
            .collect();
        let mut newest = Box::pin(registry.watch(owner).into_stream());
        assert_eq!(registry.watchers.count(owner), EVENT_STREAMS_PER_USER);
        let heard_first: Vec<UserEvent> =
            tokio::time::timeout(Duration::from_secs(2), watched.collect())
                .await
                .expect("the oldest event stream ends");
        assert!(heard_first.is_empty(), "it ends before its listing");
        let chat_id: Uuid = chat.into();
        assert_eq!(
            heard(&mut newest).await,
            Some(UserEvent::RunStarted { chat_id })
        );
        drop(others);
        assert_eq!(registry.streams.count(owner), 0);
    }

    /// A live run that streamed `message.started`, then `pieces` as text deltas of one part;
    /// with its chat and message.
    fn streamed(registry: &Arc<RunRegistry>, pieces: &[&str]) -> (Reservation, ChatId, MessageId) {
        let chat = ChatId::generate();
        let reservation = registry.reserve(UserId::new(Uuid::now_v7()), chat).unwrap();
        let sink = reservation.sink();
        let message = MessageId::generate();
        sink.emit(TurnEvent::MessageStarted {
            message_id: message,
            role: Role::Assistant,
            window_start_id: None,
            unavailable: Vec::new(),
        });
        for piece in pieces {
            sink.emit(delta(message, 0, piece));
        }
        (reservation, chat, message)
    }

    #[tokio::test]
    async fn attaching_after_any_seq_sends_exactly_the_rest() {
        let registry = registry(3);
        let pieces = ["Hel", "lo", ", wor", "ld", "!"];
        let (reservation, chat, _) = streamed(&registry, &pieces);
        let sink = reservation.sink();
        sink.emit(TurnEvent::Done);
        assert_eq!(sink.buffered(), 3, "started, one merged delta, done");
        // `done` released the chat, so a new stream gets nothing; an earlier attach is followed.
        assert!(registry.attach(chat, 0).is_none(), "idle after done");

        let full = "Hello, world!";
        for after in 0..=7u64 {
            let events: Vec<ChatStreamEvent> =
                reservation_stream(&reservation, after).collect().await;
            let (text, _) = texts(&events);
            // seq 1 is `message.started`; deltas are 2 to 6.
            let had: usize = pieces
                .iter()
                .take(usize::try_from(after.saturating_sub(1)).unwrap())
                .map(|piece| piece.len())
                .sum();
            if after < 7 {
                assert_eq!(text, full[had..], "after {after}");
                assert_eq!(events.last().map(ChatStreamEvent::seq), Some(7));
                assert!(events.iter().all(|event| event.seq() > after));
            } else {
                assert!(events.is_empty(), "the client had the whole run");
            }
        }
    }

    fn reservation_stream(
        reservation: &Reservation,
        after: u64,
    ) -> impl Stream<Item = ChatStreamEvent> + Send {
        let ticket = reservation.registry.ticket(reservation.sink.owner);
        subscribe(&reservation.sink.state, after, ticket).into_stream()
    }

    #[tokio::test]
    async fn attaching_follows_a_live_run_and_a_foreign_seq_replays_from_the_start() {
        let registry = registry(3);
        let (reservation, chat, message) = streamed(&registry, &["one ", "two "]);
        let sink = reservation.sink();
        let resumed = registry.attach(chat, 2).expect("live").into_stream();
        let foreign = registry.attach(chat, 99).expect("live").into_stream();
        sink.emit(delta(message, 0, "three"));
        sink.emit(TurnEvent::Done);

        let (text, seqs) = texts(&resumed.collect::<Vec<_>>().await);
        assert_eq!(text, "two three");
        // The stream reads the buffer as it sends, so the merged delta already holds "three".
        assert_eq!(seqs, vec![4]);
        let replayed: Vec<ChatStreamEvent> = foreign.collect().await;
        assert_eq!(replayed.first().map(ChatStreamEvent::seq), Some(1));
        assert_eq!(texts(&replayed).0, "one two three");
    }

    #[tokio::test]
    async fn draining_refuses_new_runs_then_cancels_the_live_ones() {
        let shutdown = CancellationToken::new();
        let registry = Arc::new(RunRegistry::new(shutdown.clone()));
        let user = UserId::new(Uuid::now_v7());
        let quick = registry.reserve(user, ChatId::generate()).unwrap();
        let slow = registry.reserve(user, ChatId::generate()).unwrap();
        let finished = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(quick);
        });
        let saved = tokio::spawn({
            let cancel = slow.cancel();
            async move {
                cancel.cancelled().await;
                drop(slow);
            }
        });
        let drained = registry.drain(Duration::from_millis(200), Duration::from_secs(5));
        let refused = registry.reserve(user, ChatId::generate()).unwrap_err();
        assert_eq!(refused.code(), SERVER_STOPPING);
        assert!(drained.await, "every run ended");
        assert!(shutdown.is_cancelled(), "the slow run was cancelled");
        finished.await.unwrap();
        saved.await.unwrap();
        assert!(
            registry
                .drain(Duration::from_secs(60), Duration::from_secs(60))
                .await,
            "again: nothing to wait for"
        );
    }

    #[tokio::test]
    async fn draining_reports_a_run_that_never_ends() {
        let registry = registry(3);
        let stuck = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();
        assert!(
            !registry
                .drain(Duration::from_millis(10), Duration::from_millis(10))
                .await
        );
        assert!(stuck.cancel().is_cancelled());
    }

    #[tokio::test]
    async fn draining_with_no_runs_cancels_at_once() {
        let shutdown = CancellationToken::new();
        let registry = RunRegistry::new(shutdown.clone());
        assert!(
            registry
                .drain(Duration::from_secs(60), Duration::from_secs(60))
                .await
        );
        assert!(shutdown.is_cancelled());
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

    /// The next user event, or `None` when none comes within 100 ms.
    async fn heard(stream: &mut (impl Stream<Item = UserEvent> + Unpin)) -> Option<UserEvent> {
        tokio::time::timeout(Duration::from_millis(100), stream.next())
            .await
            .ok()
            .flatten()
    }

    fn paused(message: MessageId) -> TurnOutcome {
        TurnOutcome {
            message_id: Some(message),
            status: MessageStatus::AwaitingApproval,
            finish_reason: None,
            error: None,
        }
    }

    #[tokio::test]
    async fn the_owner_hears_a_run_start_pause_and_finish_before_the_next_run() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let other = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let mut events = Box::pin(registry.watch(owner).into_stream());
        let mut theirs = Box::pin(registry.watch(other).into_stream());

        let first = registry.reserve(owner, chat).unwrap();
        assert_eq!(
            heard(&mut events).await,
            None,
            "a reservation is no run yet"
        );
        let sink = first.sink();
        sink.emit(text(0));
        sink.emit(text(1));
        let message = MessageId::generate();
        sink.ended(&paused(message));
        sink.emit(TurnEvent::Done);
        // The chat is free at `done`: the next run starts at once.
        let second = registry.reserve(owner, chat).unwrap();
        second.sink().emit(text(2));
        second.sink().emit(TurnEvent::Done);

        let chat_id: Uuid = chat.into();
        let expected = [
            UserEvent::RunStarted { chat_id },
            UserEvent::ApprovalPending {
                chat_id,
                message_id: message.into(),
            },
            UserEvent::RunFinished {
                chat_id,
                status: copilot_contracts::MessageStatus::AwaitingApproval,
            },
            UserEvent::RunStarted { chat_id },
            UserEvent::RunFinished {
                chat_id,
                status: copilot_contracts::MessageStatus::Error,
            },
        ];
        for event in expected {
            assert_eq!(heard(&mut events).await, Some(event));
        }
        assert_eq!(heard(&mut events).await, None, "each event once");
        assert_eq!(heard(&mut theirs).await, None, "another user hears nothing");
    }

    #[tokio::test]
    async fn a_run_is_hidden_until_it_starts_and_a_failed_prepare_announces_nothing() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let chat_id: Uuid = chat.into();
        let mut events = Box::pin(registry.watch(owner).into_stream());

        // `prepare` runs: the chat is taken, but no stream attaches and nobody hears of it.
        let preparing = registry.reserve(owner, chat).unwrap();
        assert!(registry.is_running(chat));
        assert!(registry.attach(chat, 0).is_none(), "idle until it starts");
        let mut connected = Box::pin(registry.watch(owner).into_stream());
        assert_eq!(
            heard(&mut connected).await,
            None,
            "not listed while prepared"
        );
        // `prepare` failed.
        drop(preparing);
        assert!(!registry.is_running(chat));
        assert_eq!(
            heard(&mut events).await,
            None,
            "a failed prepare announces nothing"
        );
        assert_eq!(heard(&mut connected).await, None);

        let reservation = registry.reserve(owner, chat).unwrap();
        reservation.start();
        reservation.start();
        let attached = registry
            .attach(chat, 0)
            .expect("a started run")
            .into_stream();
        assert_eq!(
            heard(&mut events).await,
            Some(UserEvent::RunStarted { chat_id }),
            "starting announces the run once"
        );
        assert_eq!(heard(&mut events).await, None);
        let mut late = Box::pin(registry.watch(owner).into_stream());
        assert_eq!(
            heard(&mut late).await,
            Some(UserEvent::RunStarted { chat_id })
        );
        let sink = reservation.sink();
        sink.emit(text(0));
        sink.emit(TurnEvent::Done);
        let seqs: Vec<u64> = attached.map(|event| event.seq()).collect().await;
        assert_eq!(seqs, vec![1, 2]);
        assert_eq!(
            heard(&mut events).await,
            Some(UserEvent::RunFinished {
                chat_id,
                status: copilot_contracts::MessageStatus::Error,
            })
        );
    }

    #[tokio::test]
    async fn a_started_run_dropped_before_done_ends_its_streams() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let chat_id: Uuid = chat.into();
        let mut events = Box::pin(registry.watch(owner).into_stream());
        let reservation = registry.reserve(owner, chat).unwrap();
        reservation.start();
        let attached = registry.attach(chat, 0).expect("started").into_stream();
        reservation.sink().emit(text(0));
        // The run's task panicked: only the reservation is left to drop.
        drop(reservation);
        assert!(!registry.is_running(chat));

        let streamed = tokio::time::timeout(Duration::from_secs(2), attached.collect::<Vec<_>>())
            .await
            .expect("the stream ends");
        let kinds: Vec<&str> = streamed
            .iter()
            .map(|event| match event {
                ChatStreamEvent::TextDelta { .. } => "text",
                ChatStreamEvent::Error { code, .. } if code == "INTERNAL_ERROR" => "error",
                ChatStreamEvent::Done { .. } => "done",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["text", "error", "done"]);
        assert_eq!(
            heard(&mut events).await,
            Some(UserEvent::RunStarted { chat_id })
        );
        assert_eq!(
            heard(&mut events).await,
            Some(UserEvent::RunFinished {
                chat_id,
                status: copilot_contracts::MessageStatus::Error,
            })
        );
    }

    #[tokio::test]
    async fn a_new_stream_hears_the_runs_already_live_first() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let chat = ChatId::generate();
        let reservation = registry.reserve(owner, chat).unwrap();
        reservation.sink().emit(text(0));
        let _theirs = registry
            .reserve(UserId::new(Uuid::now_v7()), ChatId::generate())
            .unwrap();

        let mut events = Box::pin(registry.watch(owner).into_stream());
        let chat_id: Uuid = chat.into();
        assert_eq!(
            heard(&mut events).await,
            Some(UserEvent::RunStarted { chat_id })
        );
        assert_eq!(heard(&mut events).await, None, "only the owner's run");
        registry.publish(owner, UserEvent::ChatUpdated { chat_id });
        assert_eq!(
            heard(&mut events).await,
            Some(UserEvent::ChatUpdated { chat_id })
        );
    }

    #[tokio::test]
    async fn a_stream_that_falls_behind_ends_and_channels_go_with_their_streams() {
        let registry = registry(3);
        let owner = UserId::new(Uuid::now_v7());
        let chat_id = Uuid::now_v7();
        let stream = registry.watch(owner).into_stream();
        for _ in 0..=USER_CHANNEL_CAPACITY {
            registry.publish(owner, UserEvent::ChatUpdated { chat_id });
        }
        let events: Vec<UserEvent> = stream.collect().await;
        assert!(events.is_empty(), "the client connects again instead");

        // The user's channel goes with the user's last stream.
        assert_eq!(registry.watchers.users(), 0);
        registry.publish(owner, UserEvent::ChatUpdated { chat_id });
        let kept = registry.watch(owner);
        let _other = registry.watch(UserId::new(Uuid::now_v7()));
        assert_eq!(registry.watchers.users(), 2);
        drop(kept);
        assert_eq!(registry.watchers.users(), 1);
    }
}
