//! `Slots`: the uploads whose bodies are being read now, and the downloads being sent.
//!
//! An upload is read whole into memory (at most `MAX_UPLOAD_MB`, and the database driver copies
//! it once more), so the upload routes take a slot before they read the body: at most
//! `UPLOADS_PER_USER` per user and `UPLOADS_AT_ONCE` in all. A download holds a few pieces of
//! its file at a time, and takes a slot of its own set (`DOWNLOADS_PER_USER`,
//! `DOWNLOADS_AT_ONCE`) until its body is sent. A request past either limit waits for a free
//! slot without reading its body; dropping the request frees its place in line. The per-user
//! limit comes first, so one user's waiting requests never hold slots others could use.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use copilot_core::UserId;
use copilot_core::shared::constants::{
    DOWNLOADS_AT_ONCE, DOWNLOADS_PER_USER, UPLOADS_AT_ONCE, UPLOADS_PER_USER,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// One set of slots of the server (uploads or downloads). Cheap to share (`Arc`).
#[derive(Debug)]
pub struct Slots {
    total: Arc<Semaphore>,
    per_user: usize,
    /// The users with requests in flight or waiting.
    users: Mutex<HashMap<UserId, User>>,
}

#[derive(Debug)]
struct User {
    semaphore: Arc<Semaphore>,
    /// Slots of the user, held or waited for; the entry goes when none is left.
    slots: usize,
}

impl Slots {
    /// `UPLOADS_PER_USER` per user, `UPLOADS_AT_ONCE` in all.
    pub fn uploads() -> Self {
        Self::new(UPLOADS_PER_USER, UPLOADS_AT_ONCE)
    }

    /// `DOWNLOADS_PER_USER` per user, `DOWNLOADS_AT_ONCE` in all.
    pub fn downloads() -> Self {
        Self::new(DOWNLOADS_PER_USER, DOWNLOADS_AT_ONCE)
    }

    pub fn new(per_user: usize, total: usize) -> Self {
        Self {
            total: Arc::new(Semaphore::new(total)),
            per_user,
            users: Mutex::new(HashMap::new()),
        }
    }

    /// Waits for a slot for one request of `user`. The slot is free again when dropped, and so
    /// is a place in line when the wait is dropped.
    pub async fn acquire(self: &Arc<Self>, user: UserId) -> Slot {
        let semaphore = {
            let mut users = self.lock();
            let entry = users.entry(user).or_insert_with(|| User {
                semaphore: Arc::new(Semaphore::new(self.per_user)),
                slots: 0,
            });
            entry.slots += 1;
            entry.semaphore.clone()
        };
        // Counted above: dropping it (with the wait) gives the count back.
        let mut slot = Slot {
            slots: self.clone(),
            user,
            mine: None,
            shared: None,
        };
        // Neither semaphore is ever closed.
        slot.mine = semaphore.acquire_owned().await.ok();
        slot.shared = self.total.clone().acquire_owned().await.ok();
        slot
    }

    /// Users with requests in flight or waiting.
    #[cfg(test)]
    fn users(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<UserId, User>> {
        self.users.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One request's place: held while an upload's body is read and stored, or a download's is sent.
#[derive(Debug)]
pub struct Slot {
    slots: Arc<Slots>,
    user: UserId,
    mine: Option<OwnedSemaphorePermit>,
    shared: Option<OwnedSemaphorePermit>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut users = self.slots.lock();
        self.shared.take();
        self.mine.take();
        if let Some(entry) = users.get_mut(&self.user) {
            entry.slots -= 1;
            if entry.slots == 0 {
                users.remove(&self.user);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use super::*;

    fn user() -> UserId {
        UserId::new(Uuid::now_v7())
    }

    /// Whether `future` finishes within a short wait.
    async fn ready<T>(future: impl Future<Output = T>) -> Option<T> {
        tokio::time::timeout(Duration::from_millis(50), future)
            .await
            .ok()
    }

    #[tokio::test]
    async fn a_user_waits_past_their_own_limit() {
        let slots = Arc::new(Slots::new(2, 8));
        let alice = user();
        let first = slots.acquire(alice).await;
        let _second = slots.acquire(alice).await;
        assert!(
            ready(slots.acquire(alice)).await.is_none(),
            "a third upload of the same user waits"
        );
        let _bob = ready(slots.acquire(user()))
            .await
            .expect("other users are not held up");
        drop(first);
        assert!(ready(slots.acquire(alice)).await.is_some());
    }

    #[tokio::test]
    async fn every_upload_waits_past_the_total() {
        let slots = Arc::new(Slots::new(2, 2));
        let first = slots.acquire(user()).await;
        let _second = slots.acquire(user()).await;
        assert!(ready(slots.acquire(user())).await.is_none());
        drop(first);
        assert!(ready(slots.acquire(user())).await.is_some());
    }

    #[tokio::test]
    async fn users_without_uploads_leave_no_entry() {
        let slots = Arc::new(Slots::new(1, 8));
        let alice = user();
        let held = slots.acquire(alice).await;
        // A waiter that gives up (the client went away) keeps nothing.
        assert!(ready(slots.acquire(alice)).await.is_none());
        assert_eq!(slots.users(), 1);
        drop(held);
        assert_eq!(slots.users(), 0);

        let held = slots.acquire(alice).await;
        let waiting = tokio::spawn({
            let slots = slots.clone();
            async move { slots.acquire(alice).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(held);
        let next = waiting.await.unwrap();
        assert_eq!(slots.users(), 1, "the waiter took the slot");
        drop(next);
        assert_eq!(slots.users(), 0);
    }
}
