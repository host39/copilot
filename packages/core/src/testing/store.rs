//! One in-memory store behind every repository port.
//!
//! [`MemoryStore`] implements each repository trait (one file per domain next to this one), so a
//! test passes the same `Arc<MemoryStore>` as every `Arc<dyn ...Repository>` a service needs.
//! Sharing one store lets the fakes follow the database's cross-table rules (cascades, `SET
//! NULL`, usage counts). Reads are scoped by `UserId` exactly like the SQL repositories.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::domains::file::{FileOwner, StoredFile};
use crate::domains::project::Project;
use crate::domains::provider::{Model, Provider, ProviderUsage};
use crate::domains::schedule::Schedule;
use crate::domains::settings::UserSettings;
use crate::domains::tool::ToolServer;
use crate::ports::clock::Clock;
use crate::shared::actor::Actor;
use crate::shared::errors::DomainError;
use crate::shared::ids::{
    FileId, ModelId, ProjectId, ProviderId, ScheduleId, ToolServerId, UserId,
};

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
    pub(crate) projects: BTreeMap<ProjectId, Project>,
    pub(crate) files: BTreeMap<FileId, (StoredFile, Bytes)>,
    pub(crate) tool_servers: BTreeMap<ToolServerId, ToolServer>,
    pub(crate) schedules: BTreeMap<ScheduleId, Schedule>,
    /// The two `auth.users` columns Copilot reads (`ScheduleRepository::due`).
    pub(crate) users: HashMap<UserId, Owner>,
}

/// A lib-rustauth user as `ScheduleRepository::due` sees it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Owner {
    pub(crate) is_admin: bool,
    pub(crate) disabled: bool,
}

impl MemoryStore {
    /// `clock` stamps rows that the repositories create themselves (model sync).
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            tables: Mutex::new(Tables::default()),
        }
    }

    /// Records a lib-rustauth user's role for `ScheduleRepository::due`. Owners never added count
    /// as active plain users.
    pub fn add_user(&self, actor: &Actor) {
        self.tables().users.insert(
            actor.user_id,
            Owner {
                is_admin: actor.is_admin,
                disabled: false,
            },
        );
    }

    /// Sets `auth.users.disabled_at`: the scheduler skips the user's schedules.
    pub fn disable_user(&self, user: UserId) {
        self.tables().users.entry(user).or_default().disabled = true;
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
    pub(crate) fn clear_model_references(&mut self, model: ModelId) {
        for project in self.projects.values_mut() {
            if project.default_model_id() == Some(model) {
                *project = project.clone().with_default_model(None);
            }
        }
        for schedule in self.schedules.values_mut() {
            if schedule.model_id() == Some(model) {
                *schedule = schedule.clone().with_model(None);
            }
        }
    }

    /// Rows of `user` that reference one of `models`.
    pub(crate) fn model_usage(&self, user: UserId, models: &[ModelId]) -> ProviderUsage {
        let uses = |model: Option<ModelId>| model.is_some_and(|model| models.contains(&model));
        ProviderUsage {
            projects: self
                .projects
                .values()
                .filter(|project| project.user_id() == user && uses(project.default_model_id()))
                .count() as u64,
            schedules: self
                .schedules
                .values()
                .filter(|schedule| schedule.user_id() == user && uses(schedule.model_id()))
                .count() as u64,
            ..ProviderUsage::default()
        }
    }

    /// Deletes a project and what cascades with it: its files and schedules (and, once that
    /// domain exists, its chats).
    pub(crate) fn delete_project(&mut self, id: ProjectId) {
        self.projects.remove(&id);
        self.files
            .retain(|_, (file, _)| file.owner() != FileOwner::Project(id));
        let schedules: Vec<ScheduleId> = self
            .schedules
            .values()
            .filter(|schedule| schedule.project_id() == Some(id))
            .map(Schedule::id)
            .collect();
        for schedule in schedules {
            self.delete_schedule(schedule);
        }
    }
}

/// The error a primary key collision gives.
pub(crate) fn duplicate(table: &str) -> DomainError {
    DomainError::Internal {
        key: format!("testing.{table}.insert.duplicate"),
        source: None,
    }
}
