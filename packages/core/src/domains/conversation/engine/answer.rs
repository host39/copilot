//! An assistant message while it streams: provider events become stored parts and live events.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::domains::chat::{Approval, FinishReason, Part, ToolCallPart, TurnContext, Usage};
use crate::domains::conversation::events::TurnEvent;
use crate::domains::tool::ToolAlias;
use crate::ports::llm_gateway::{LlmEvent, PartKind, Refusal};
use crate::ports::turn_sink::TurnSink;
use crate::shared::constants::AUDIO_OUT_FORMAT;
use crate::shared::ids::{CallId, MessageId};

/// The parts, state, usage, and finish of one streamed answer.
pub(crate) struct Answer {
    message_id: MessageId,
    pub(crate) parts: Vec<Part>,
    /// The adapter's part index to the stored part's position.
    slots: HashMap<u32, usize>,
    /// The text part that collects spoken reply transcripts.
    transcript: Option<usize>,
    pub(crate) provider_state: Option<Value>,
    pub(crate) usage: Option<Usage>,
    pub(crate) finish: Option<(FinishReason, Option<Refusal>)>,
}

impl Answer {
    pub(crate) fn new(message_id: MessageId) -> Self {
        Self {
            message_id,
            parts: Vec::new(),
            slots: HashMap::new(),
            transcript: None,
            provider_state: None,
            usage: None,
            finish: None,
        }
    }

    /// Applies one provider event and reports what changed. A tool call takes its server and
    /// original name from the snapshot (an unknown alias keeps the alias as its name).
    pub(crate) fn apply(&mut self, event: LlmEvent, context: &TurnContext, sink: &dyn TurnSink) {
        match event {
            LlmEvent::PartStarted { index, kind } => match kind {
                PartKind::Text | PartKind::Reasoning => {
                    self.slot(index, kind, sink);
                }
                PartKind::ToolCall | PartKind::Audio => {}
            },
            LlmEvent::TextDelta { index, text } => {
                let position = self.slot(index, PartKind::Text, sink);
                self.append(position, &text);
                sink.emit(TurnEvent::TextDelta {
                    message_id: self.message_id,
                    index: wire(position),
                    text,
                });
            }
            LlmEvent::ReasoningDelta { index, text } => {
                let position = self.slot(index, PartKind::Reasoning, sink);
                self.append(position, &text);
                sink.emit(TurnEvent::ReasoningDelta {
                    message_id: self.message_id,
                    index: wire(position),
                    text,
                });
            }
            LlmEvent::AudioDelta { data, transcript } => {
                sink.emit(TurnEvent::AudioDelta {
                    message_id: self.message_id,
                    data,
                    format: AUDIO_OUT_FORMAT.to_owned(),
                });
                if let Some(text) = transcript.filter(|text| !text.is_empty()) {
                    let position = match self.transcript {
                        Some(position) => position,
                        None => {
                            let position = self.start(Part::Text {
                                text: String::new(),
                            });
                            self.started(position, PartKind::Text, sink);
                            self.transcript = Some(position);
                            position
                        }
                    };
                    self.append(position, &text);
                    sink.emit(TurnEvent::TextDelta {
                        message_id: self.message_id,
                        index: wire(position),
                        text,
                    });
                }
            }
            LlmEvent::ToolCallStarted {
                index,
                provider_call_id,
                alias,
            } => {
                let alias = ToolAlias::restore(alias);
                let tool = context.tool(&alias);
                let position = self.start(Part::ToolCall(ToolCallPart {
                    id: CallId::generate(),
                    provider_call_id,
                    server_id: tool.and_then(|tool| tool.server_id),
                    tool_name: tool.map_or_else(|| alias.to_string(), |tool| tool.name.clone()),
                    alias,
                    arguments_raw: String::new(),
                    arguments: None,
                    complete: false,
                    approval: Approval::Auto,
                    deny_reason: None,
                }));
                self.slots.insert(index, position);
                self.started(position, PartKind::ToolCall, sink);
            }
            LlmEvent::ToolCallArgs { index, json } => {
                if let Some(call) = self.call(index) {
                    call.arguments_raw.push_str(&json);
                }
            }
            LlmEvent::ToolCallEnded { index } => {
                if let Some(call) = self.call(index) {
                    end(call);
                }
            }
            LlmEvent::ReplayState(state) => self.provider_state = Some(state),
            LlmEvent::Usage(usage) => self.usage.get_or_insert_default().merge(usage),
            LlmEvent::Finished { reason, refusal } => {
                // The answer is over, so every call's arguments are as complete as they get.
                for part in &mut self.parts {
                    if let Part::ToolCall(call) = part
                        && !call.complete
                    {
                        end(call);
                    }
                }
                self.finish = Some((reason, refusal));
            }
        }
    }

    /// The tool calls, in call order.
    pub(crate) fn calls(&self) -> Vec<ToolCallPart> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect()
    }

    /// The part for adapter index `index`, started as `kind` when it is new or of another kind.
    fn slot(&mut self, index: u32, kind: PartKind, sink: &dyn TurnSink) -> usize {
        if let Some(&position) = self.slots.get(&index) {
            let same = matches!(
                (&self.parts[position], kind),
                (Part::Text { .. }, PartKind::Text) | (Part::Reasoning { .. }, PartKind::Reasoning)
            );
            if same {
                return position;
            }
        }
        let part = match kind {
            PartKind::Reasoning => Part::Reasoning {
                text: String::new(),
            },
            _ => Part::Text {
                text: String::new(),
            },
        };
        let position = self.start(part);
        self.slots.insert(index, position);
        self.started(position, kind, sink);
        position
    }

    fn start(&mut self, part: Part) -> usize {
        self.parts.push(part);
        self.parts.len() - 1
    }

    fn started(&self, position: usize, kind: PartKind, sink: &dyn TurnSink) {
        sink.emit(TurnEvent::PartStarted {
            message_id: self.message_id,
            index: wire(position),
            kind,
        });
    }

    fn append(&mut self, position: usize, more: &str) {
        if let Part::Text { text } | Part::Reasoning { text } = &mut self.parts[position] {
            text.push_str(more);
        }
    }

    fn call(&mut self, index: u32) -> Option<&mut ToolCallPart> {
        let position = *self.slots.get(&index)?;
        match &mut self.parts[position] {
            Part::ToolCall(call) => Some(call),
            _ => None,
        }
    }
}

/// Marks a call's arguments finished: blank arguments read as `{}`, others must parse to a JSON
/// object.
fn end(call: &mut ToolCallPart) {
    call.complete = true;
    call.arguments = if call.arguments_raw.trim().is_empty() {
        Some(Value::Object(Map::new()))
    } else {
        serde_json::from_str::<Value>(&call.arguments_raw)
            .ok()
            .filter(Value::is_object)
    };
}

fn wire(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX)
}
