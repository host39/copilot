//! One in-memory store behind every repository port.
//!
//! [`MemoryStore`] implements each repository trait (one file per domain next to this one), so a
//! test passes the same `Arc<MemoryStore>` as every `Arc<dyn ...Repository>` a service needs.
//! Sharing one store lets the fakes follow the database's cross-table rules (cascades, `SET
//! NULL`, usage counts). Reads are scoped by `UserId` exactly like the SQL repositories.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};

use crate::domains::provider::{Model, Provider, ProviderUsage};
use crate::domains::settings::UserSettings;
use crate::ports::clock::Clock;
use crate::shared::errors::DomainError;
use crate::shared::ids::{ModelId, ProviderId, UserId};

pub struct MemoryStore {
    clock: Arc<dyn Clock>,
    tables: Mutex<Tables>,
}

/// The stored rows, keyed by id. `BTreeMap` keeps iteration in id order, which for v7 ids is
/// creation order.
#[derive(Default)]
pub(crate) struct Tables {
    pub(crate) settings: HashMap<UserId, UserSettings>,
    pub(crate) providers: BTreeMap<ProviderId, Provider>,
    pub(crate) models: BTreeMap<ModelId, Model>,
}

impl MemoryStore {
    /// `clock` stamps rows that the repositories create themselves (model sync).
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            tables: Mutex::new(Tables::default()),
        }
    }

    pub(crate) fn tables(&self) -> MutexGuard<'_, Tables> {
        self.tables.lock().expect("memory store lock")
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
}

impl Tables {
    /// Nulls every reference to a deleted model (`ON DELETE SET NULL (model_id)`).
    pub(crate) fn clear_model_references(&mut self, _model: ModelId) {}

    /// Rows of `user` that reference one of `models`.
    pub(crate) fn model_usage(&self, _user: UserId, _models: &[ModelId]) -> ProviderUsage {
        ProviderUsage::default()
    }
}

/// The error a primary key collision gives.
pub(crate) fn duplicate(table: &str) -> DomainError {
    DomainError::Internal {
        key: format!("testing.{table}.insert.duplicate"),
        source: None,
    }
}
