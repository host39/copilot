//! One in-memory store behind every repository port.
//!
//! [`MemoryStore`] implements each repository trait (one file per domain next to this one), so a
//! test passes the same `Arc<MemoryStore>` as every `Arc<dyn ...Repository>` a service needs.
//! Sharing one store lets the fakes follow the database's cross-table rules (cascades, `SET
//! NULL`, usage counts). Reads are scoped by `UserId` exactly like the SQL repositories.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};

use crate::domains::settings::UserSettings;
use crate::ports::clock::Clock;
use crate::shared::ids::UserId;

pub struct MemoryStore {
    clock: Arc<dyn Clock>,
    tables: Mutex<Tables>,
}

/// The stored rows, keyed by id. `BTreeMap` keeps iteration in id order, which for v7 ids is
/// creation order.
#[derive(Default)]
pub(crate) struct Tables {
    pub(crate) settings: HashMap<UserId, UserSettings>,
}

impl MemoryStore {
    /// `clock` stamps rows that the repositories create themselves.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            tables: Mutex::new(Tables::default()),
        }
    }

    pub(crate) fn tables(&self) -> MutexGuard<'_, Tables> {
        self.tables.lock().expect("memory store lock")
    }

    // Used by the repositories that stamp rows themselves (model sync).
    #[allow(dead_code)]
    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}
