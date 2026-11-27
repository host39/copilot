//! `Streams`: the long-lived streams each user has open, at most a set number per user
//! (`docs/design/copilot.md`, Limits): the run streams (`CHAT_STREAMS_PER_USER`, the turn routes
//! and `GET /api/chats/{id}/stream`) and the `GET /api/events` streams
//! (`EVENT_STREAMS_PER_USER`). Opening one more ends that user's oldest, so one account never
//! holds more than its share of connections, tasks, and buffers, and streams a client left
//! behind (a network that dropped without the server noticing yet) never lock its devices out:
//! the oldest goes first, and a live client connects again.
//!
//! Each stream holds a [`Ticket`] and ends when its ticket's `ended` token is cancelled. What a
//! user's streams share (the user's event channel) lives with their tickets and goes when the
//! last one is dropped.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use copilot_core::UserId;
use tokio_util::sync::CancellationToken;

/// The open streams of each user, at most `cap` each. Share it in an `Arc`.
#[derive(Debug)]
pub struct Streams<T> {
    cap: usize,
    users: Mutex<HashMap<UserId, Open<T>>>,
    next: AtomicU64,
}

/// One user's open streams.
#[derive(Debug)]
struct Open<T> {
    /// What the user's streams share.
    shared: T,
    /// The open streams' ids and `ended` tokens, oldest first.
    tickets: VecDeque<(u64, CancellationToken)>,
}

impl<T> Streams<T> {
    /// At most `cap` (at least one) open streams per user.
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            users: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        }
    }

    /// Opens a stream of `user`, ending the user's oldest when `cap` are open. `make` creates
    /// what the user's streams share when none is open; `with` reads it for the new stream.
    pub fn open<R>(
        self: &Arc<Self>,
        user: UserId,
        make: impl FnOnce() -> T,
        with: impl FnOnce(&T) -> R,
    ) -> (Ticket<T>, R) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let ended = CancellationToken::new();
        let mut users = self.lock();
        let open = users.entry(user).or_insert_with(|| Open {
            shared: make(),
            tickets: VecDeque::new(),
        });
        while open.tickets.len() >= self.cap {
            if let Some((_, oldest)) = open.tickets.pop_front() {
                oldest.cancel();
            }
        }
        open.tickets.push_back((id, ended.clone()));
        let read = with(&open.shared);
        let ticket = Ticket {
            streams: Arc::clone(self),
            user,
            id,
            ended,
        };
        (ticket, read)
    }

    /// Reads what the streams of `user` share; `None` while the user has none open.
    pub fn shared<R>(&self, user: UserId, with: impl FnOnce(&T) -> R) -> Option<R> {
        self.lock().get(&user).map(|open| with(&open.shared))
    }

    /// How many streams `user` has open.
    #[cfg(test)]
    pub fn count(&self, user: UserId) -> usize {
        self.lock().get(&user).map_or(0, |open| open.tickets.len())
    }

    /// How many users have a stream open.
    #[cfg(test)]
    pub fn users(&self) -> usize {
        self.lock().len()
    }

    fn close(&self, user: UserId, id: u64) {
        let mut users = self.lock();
        if let Some(open) = users.get_mut(&user) {
            open.tickets.retain(|(ticket, _)| *ticket != id);
            if open.tickets.is_empty() {
                users.remove(&user);
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<UserId, Open<T>>> {
        self.users.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One open stream ([`Streams::open`]); dropping it closes the stream's place.
#[derive(Debug)]
pub struct Ticket<T> {
    streams: Arc<Streams<T>>,
    user: UserId,
    id: u64,
    ended: CancellationToken,
}

impl<T> Ticket<T> {
    /// Cancelled when a newer stream of the user took this one's place: end the stream.
    pub fn ended(&self) -> &CancellationToken {
        &self.ended
    }
}

impl<T> Drop for Ticket<T> {
    fn drop(&mut self) {
        self.streams.close(self.user, self.id);
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn user() -> UserId {
        UserId::new(Uuid::now_v7())
    }

    #[test]
    fn opening_past_the_cap_ends_the_oldest_stream_of_that_user_only() {
        let streams = Arc::new(Streams::new(2));
        let (alice, bob) = (user(), user());
        let (first, ()) = streams.open(alice, || (), |()| ());
        let (second, ()) = streams.open(alice, || (), |()| ());
        let (theirs, ()) = streams.open(bob, || (), |()| ());
        assert_eq!(streams.count(alice), 2);

        let (third, ()) = streams.open(alice, || (), |()| ());
        assert!(first.ended().is_cancelled(), "the oldest makes room");
        assert!(!second.ended().is_cancelled());
        assert!(!third.ended().is_cancelled());
        assert!(
            !theirs.ended().is_cancelled(),
            "another user's streams stay"
        );
        assert_eq!(streams.count(alice), 2);

        // The ended stream goes away later; it never frees a place it no longer holds.
        drop(first);
        assert_eq!(streams.count(alice), 2);
        drop(second);
        assert_eq!(
            streams.count(alice),
            1,
            "a stream that ends frees its place"
        );
        let (fourth, ()) = streams.open(alice, || (), |()| ());
        assert!(!third.ended().is_cancelled(), "a free place ends nothing");
        drop((third, fourth, theirs));
        assert_eq!(streams.users(), 0, "users without streams are forgotten");
    }

    #[test]
    fn what_a_users_streams_share_lives_as_long_as_their_streams() {
        let streams = Arc::new(Streams::new(3));
        let alice = user();
        assert_eq!(streams.shared(alice, |value| *value), None);
        let (first, made) = streams.open(alice, || 7, |value| *value);
        assert_eq!(made, 7);
        let (second, kept) = streams.open(alice, || 9, |value| *value);
        assert_eq!(kept, 7, "made once per user");
        assert_eq!(streams.shared(alice, |value| *value), Some(7));
        drop(first);
        drop(second);
        assert_eq!(streams.shared(alice, |value| *value), None);
        let (_third, fresh) = streams.open(alice, || 9, |value| *value);
        assert_eq!(fresh, 9, "made again after the last stream closed");
    }
}
