//! A scripted `LlmGateway`.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures_util::stream::{self, BoxStream, StreamExt};

use crate::domains::chat::{FinishReason, Usage};
use crate::ports::llm_gateway::{
    LlmEvent, LlmGateway, LlmRequest, ModelTarget, PartKind, ProviderTarget, RemoteModel,
};
use crate::shared::errors::{DomainError, DomainResult};

/// One step of a scripted answer.
#[derive(Debug)]
pub enum Step {
    Event(DomainResult<LlmEvent>),
    /// The stream stops here and never ends (to test cancellation).
    Stall,
}

/// Answers each call with the next scripted result, in order, and records every target and
/// request it was called with. An unscripted call fails with an internal error.
#[derive(Debug, Default)]
pub struct ScriptedGateway {
    models: Mutex<VecDeque<DomainResult<Vec<RemoteModel>>>>,
    targets: Mutex<Vec<ProviderTarget>>,
    streams: Mutex<VecDeque<DomainResult<Vec<Step>>>>,
    requests: Mutex<Vec<(ModelTarget, LlmRequest)>>,
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

    /// The next `stream` call yields these events and ends.
    pub fn push_events(&self, events: Vec<LlmEvent>) {
        self.push_steps(
            events
                .into_iter()
                .map(|event| Step::Event(Ok(event)))
                .collect(),
        );
    }

    /// The next `stream` call follows these steps.
    pub fn push_steps(&self, steps: Vec<Step>) {
        self.streams
            .lock()
            .expect("gateway lock")
            .push_back(Ok(steps));
    }

    /// The next `stream` call fails before its first event.
    pub fn push_stream_error(&self, error: DomainError) {
        self.streams
            .lock()
            .expect("gateway lock")
            .push_back(Err(error));
    }

    /// The targets of every `models` call so far, oldest first.
    pub fn targets(&self) -> Vec<ProviderTarget> {
        self.targets.lock().expect("gateway lock").clone()
    }

    /// The requests of every `stream` call so far, oldest first.
    pub fn requests(&self) -> Vec<LlmRequest> {
        self.requests
            .lock()
            .expect("gateway lock")
            .iter()
            .map(|(_, request)| request.clone())
            .collect()
    }

    /// The targets of every `stream` call so far, oldest first.
    pub fn stream_targets(&self) -> Vec<ModelTarget> {
        self.requests
            .lock()
            .expect("gateway lock")
            .iter()
            .map(|(target, _)| target.clone())
            .collect()
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
            .unwrap_or_else(|| Err(unscripted("models")))
    }

    async fn stream(
        &self,
        target: &ModelTarget,
        request: LlmRequest,
    ) -> DomainResult<BoxStream<'static, DomainResult<LlmEvent>>> {
        self.requests
            .lock()
            .expect("gateway lock")
            .push((target.clone(), request));
        let steps = self
            .streams
            .lock()
            .expect("gateway lock")
            .pop_front()
            .unwrap_or_else(|| Err(unscripted("stream")))?;
        let mut events = Vec::new();
        let mut stalls = false;
        for step in steps {
            match step {
                Step::Event(event) => events.push(event),
                Step::Stall => {
                    stalls = true;
                    break;
                }
            }
        }
        let head = stream::iter(events);
        Ok(if stalls {
            head.chain(stream::pending()).boxed()
        } else {
            head.boxed()
        })
    }
}

fn unscripted(call: &str) -> DomainError {
    DomainError::Internal {
        key: format!("testing.gateway.{call}.unscripted"),
        source: None,
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

/// A plain answer: one text part, usage, and `stop`.
pub fn text_reply(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::PartStarted {
            index: 0,
            kind: PartKind::Text,
        },
        LlmEvent::TextDelta {
            index: 0,
            text: text.to_owned(),
        },
        LlmEvent::Usage(Usage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            ..Usage::default()
        }),
        LlmEvent::Finished {
            reason: FinishReason::Stop,
            refusal: None,
        },
    ]
}

/// An answer that calls one tool per `(alias, arguments JSON)`, then finishes with
/// `tool_calls`. The provider's call ids are `call_1`, `call_2`, ...
pub fn call_reply(calls: &[(&str, &str)]) -> Vec<LlmEvent> {
    let mut events = Vec::new();
    for (index, (alias, json)) in (0_u32..).zip(calls) {
        events.push(LlmEvent::ToolCallStarted {
            index,
            provider_call_id: Some(format!("call_{}", index + 1)),
            alias: (*alias).to_owned(),
        });
        events.push(LlmEvent::ToolCallArgs {
            index,
            json: (*json).to_owned(),
        });
        events.push(LlmEvent::ToolCallEnded { index });
    }
    events.push(LlmEvent::Finished {
        reason: FinishReason::ToolCalls,
        refusal: None,
    });
    events
}
