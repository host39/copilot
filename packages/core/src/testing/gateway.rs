//! A scripted `LlmGateway`.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::ports::llm_gateway::{LlmGateway, ProviderTarget, RemoteModel};
use crate::shared::errors::{DomainError, DomainResult};

/// Answers each call with the next scripted result, in order, and records every target it was
/// called with. An unscripted call fails with an internal error.
#[derive(Debug, Default)]
pub struct ScriptedGateway {
    models: Mutex<VecDeque<DomainResult<Vec<RemoteModel>>>>,
    targets: Mutex<Vec<ProviderTarget>>,
}

impl ScriptedGateway {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next `models` call lists these.
    pub fn push_models(&self, models: Vec<RemoteModel>) {
        self.models
            .lock()
            .expect("gateway lock")
            .push_back(Ok(models));
    }

    /// The next `models` call fails with this error.
    pub fn push_error(&self, error: DomainError) {
        self.models
            .lock()
            .expect("gateway lock")
            .push_back(Err(error));
    }

    /// The targets of every call so far, oldest first.
    pub fn targets(&self) -> Vec<ProviderTarget> {
        self.targets.lock().expect("gateway lock").clone()
    }
}

#[async_trait]
impl LlmGateway for ScriptedGateway {
    async fn models(&self, target: &ProviderTarget) -> DomainResult<Vec<RemoteModel>> {
        self.targets
            .lock()
            .expect("gateway lock")
            .push(target.clone());
        self.models
            .lock()
            .expect("gateway lock")
            .pop_front()
            .unwrap_or_else(|| {
                Err(DomainError::Internal {
                    key: "testing.gateway.models.unscripted".to_owned(),
                    source: None,
                })
            })
    }
}

/// A listed model named `Model <id>` with nothing reported.
pub fn remote_model(id: &str) -> RemoteModel {
    RemoteModel {
        remote_id: id.to_owned(),
        display_name: format!("Model {}", id.trim()),
        ..RemoteModel::default()
    }
}
