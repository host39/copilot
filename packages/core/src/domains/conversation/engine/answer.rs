//! An assistant message while it streams: provider events become stored parts and live events,
//! within the answer's [`Ceiling`].

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::domains::chat::{Approval, FinishReason, Part, ToolCallPart, TurnContext, Usage};
use crate::domains::conversation::events::TurnEvent;
use crate::domains::tool::ToolAlias;
use crate::ports::llm_gateway::{LlmEvent, PartKind, Refusal};
use crate::ports::turn_sink::TurnSink;
use crate::shared::constants::{
    ANSWER_MAX_BYTES, ANSWER_MAX_PARTS, ANSWER_SLACK, AUDIO_OUT_FORMAT, AUDIO_OUT_MAX_BYTES,
    BYTES_PER_TOKEN,
};
use crate::shared::errors::{DomainError, DomainResult};
use crate::shared::ids::{CallId, MessageId};

/// The error of an answer past its [`Ceiling`].
pub const TOO_LONG: &str = "The provider sent more than this model's output limit allows.";
/// Its code: the provider did not keep to the request.
pub const TOO_LONG_CODE: &str = "PROVIDER_BAD_RESPONSE";

/// What one answer may stream before the engine ends it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Ceiling {
    /// Text, reasoning, transcripts, and tool call names, ids, and arguments, in bytes.
    pub(crate) bytes: u64,
    /// Spoken reply audio, in bytes.
    pub(crate) audio: u64,
    pub(crate) parts: usize,
}

impl Ceiling {
    /// The ceiling of an answer requested with `output_tokens` as its token limit.
    pub(crate) fn new(output_tokens: u32) -> Self {
        Self {
            bytes: (u64::from(output_tokens) * BYTES_PER_TOKEN * ANSWER_SLACK)
                .min(ANSWER_MAX_BYTES),
            audio: AUDIO_OUT_MAX_BYTES,
            parts: ANSWER_MAX_PARTS,
        }
    }
}

/// The parts, state, usage, and finish of one streamed answer.
pub(crate) struct Answer {
    message_id: MessageId,
    ceiling: Ceiling,
    /// What counts toward `ceiling.bytes` so far.
    streamed: u64,
    /// The spoken reply's PCM16 audio so far (it counts toward `ceiling.audio`). Once the answer
    /// ends it becomes a WAV chat file and the answer's last part, so it holds a part from its
    /// first byte on.
    speech: Vec<u8>,
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
    pub(crate) fn new(message_id: MessageId, ceiling: Ceiling) -> Self {
        Self {
            message_id,
            ceiling,
            streamed: 0,
            speech: Vec::new(),
            parts: Vec::new(),
            slots: HashMap::new(),
            transcript: None,
            provider_state: None,
            usage: None,
            finish: None,
        }
    }

    /// Applies one provider event and reports what changed. A tool call takes its server and
    /// original name from the snapshot (an unknown alias keeps the alias as its name). An event
    /// that would take the answer past its ceiling changes nothing and fails with [`TOO_LONG`].
    pub(crate) fn apply(
        &mut self,
        event: LlmEvent,
        context: &TurnContext,
        sink: &dyn TurnSink,
    ) -> DomainResult<()> {
        self.admit(&event)?;
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
                self.speech.extend_from_slice(&data);
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
        Ok(())
    }

    /// Counts `event` toward the ceiling, or refuses it when it would go past.
    fn admit(&mut self, event: &LlmEvent) -> DomainResult<()> {
        let (bytes, audio, opens) = match event {
            LlmEvent::PartStarted { index, kind } => match kind {
                PartKind::Text | PartKind::Reasoning => (0, 0, self.opens(*index, *kind)),
                PartKind::ToolCall | PartKind::Audio => (0, 0, false),
            },
            LlmEvent::TextDelta { index, text } => {
                (size(text), 0, self.opens(*index, PartKind::Text))
            }
            LlmEvent::ReasoningDelta { index, text } => {
                (size(text), 0, self.opens(*index, PartKind::Reasoning))
            }
            LlmEvent::AudioDelta { data, transcript } => {
                let text = transcript.as_deref().unwrap_or_default();
                let opens = self.transcript.is_none() && !text.is_empty();
                (size(text), data.len(), opens)
            }
            LlmEvent::ToolCallStarted {
                provider_call_id,
                alias,
                ..
            } => (
                provider_call_id.as_deref().map_or(0, size) + size(alias),
                0,
                true,
            ),
            LlmEvent::ToolCallArgs { json, .. } => (size(json), 0, false),
            LlmEvent::ToolCallEnded { .. }
            | LlmEvent::ReplayState(_)
            | LlmEvent::Usage(_)
            | LlmEvent::Finished { .. } => (0, 0, false),
        };
        let streamed = self.streamed + bytes;
        let spoken = self.speech.len() + audio;
        let parts = self.parts.len() + usize::from(opens) + usize::from(spoken > 0);
        if streamed > self.ceiling.bytes
            || spoken as u64 > self.ceiling.audio
            || parts > self.ceiling.parts
        {
            return Err(DomainError::new(TOO_LONG, TOO_LONG_CODE));
        }
        self.streamed = streamed;
        Ok(())
    }

    /// The spoken reply's PCM16 audio (empty when the answer spoke nothing).
    pub(crate) fn speech(&self) -> &[u8] {
        &self.speech
    }

    /// Whether adapter index `index` as `kind` starts a new part (see [`Answer::slot`]).
    fn opens(&self, index: u32, kind: PartKind) -> bool {
        self.slots.get(&index).is_none_or(|&position| {
            !matches!(
                (&self.parts[position], kind),
                (Part::Text { .. }, PartKind::Text) | (Part::Reasoning { .. }, PartKind::Reasoning)
            )
        })
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
        if !self.opens(index, kind)
            && let Some(&position) = self.slots.get(&index)
        {
            return position;
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

fn size(text: &str) -> u64 {
    text.len() as u64
}

fn wire(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::domains::chat::ContextHash;
    use crate::testing::RecordingSink;

    fn context() -> TurnContext {
        TurnContext {
            hash: ContextHash::new([0; 32]),
            system: String::new(),
            tools: Vec::new(),
            files: Vec::new(),
            window_start_id: None,
            reasoning: None,
        }
    }

    fn small(bytes: u64, audio: u64, parts: usize) -> Answer {
        Answer::new(
            MessageId::generate(),
            Ceiling {
                bytes,
                audio,
                parts,
            },
        )
    }

    fn text(index: u32, text: &str) -> LlmEvent {
        LlmEvent::TextDelta {
            index,
            text: text.to_owned(),
        }
    }

    #[test]
    fn the_ceiling_follows_the_output_limit_up_to_the_maximum() {
        let ceiling = Ceiling::new(8_192);
        assert_eq!(ceiling.bytes, 8_192 * BYTES_PER_TOKEN * ANSWER_SLACK);
        assert_eq!(ceiling.audio, AUDIO_OUT_MAX_BYTES);
        assert_eq!(ceiling.parts, ANSWER_MAX_PARTS);
        assert_eq!(Ceiling::new(u32::MAX).bytes, ANSWER_MAX_BYTES);
    }

    #[test]
    fn text_past_the_ceiling_is_refused_and_changes_nothing() {
        let sink = RecordingSink::new();
        let mut answer = small(10, 0, 4);
        answer.apply(text(0, "Hello"), &context(), &sink).unwrap();
        answer.apply(text(0, " you"), &context(), &sink).unwrap();
        let error = answer.apply(text(0, "!!"), &context(), &sink).unwrap_err();
        assert_eq!(error.to_string(), TOO_LONG);
        assert_eq!(error.code(), TOO_LONG_CODE);
        assert_eq!(
            answer.parts,
            [Part::Text {
                text: "Hello you".to_owned()
            }]
        );
        assert_eq!(sink.events().len(), 3, "part.started and two deltas");
        answer.apply(text(0, "!"), &context(), &sink).unwrap();
    }

    #[test]
    fn reasoning_and_tool_calls_count_toward_the_ceiling() {
        let sink = RecordingSink::new();
        let mut answer = small(12, 0, 4);
        answer
            .apply(
                LlmEvent::ReasoningDelta {
                    index: 0,
                    text: "think".to_owned(),
                },
                &context(),
                &sink,
            )
            .unwrap();
        answer
            .apply(
                LlmEvent::ToolCallStarted {
                    index: 1,
                    provider_call_id: Some("c1".to_owned()),
                    alias: "a__b".to_owned(),
                },
                &context(),
                &sink,
            )
            .unwrap();
        let refused = answer.apply(
            LlmEvent::ToolCallArgs {
                index: 1,
                json: "{}".to_owned(),
            },
            &context(),
            &sink,
        );
        assert!(refused.is_err(), "5 + 6 + 2 bytes is past 12");
    }

    #[test]
    fn parts_past_the_ceiling_are_refused() {
        let sink = RecordingSink::new();
        let mut answer = small(100, 0, 2);
        answer.apply(text(0, "a"), &context(), &sink).unwrap();
        answer.apply(text(0, "b"), &context(), &sink).unwrap();
        answer
            .apply(
                LlmEvent::PartStarted {
                    index: 1,
                    kind: PartKind::Reasoning,
                },
                &context(),
                &sink,
            )
            .unwrap();
        let refused = answer.apply(
            LlmEvent::PartStarted {
                index: 2,
                kind: PartKind::Text,
            },
            &context(),
            &sink,
        );
        assert!(refused.is_err());
        let refused = answer.apply(
            LlmEvent::ToolCallStarted {
                index: 3,
                provider_call_id: None,
                alias: "x".to_owned(),
            },
            &context(),
            &sink,
        );
        assert!(refused.is_err());
        answer.apply(text(1, "c"), &context(), &sink).unwrap_err();
        assert_eq!(answer.parts.len(), 2);
    }

    #[test]
    fn spoken_audio_has_its_own_ceiling() {
        let sink = RecordingSink::new();
        let mut answer = small(100, 4, 4);
        let audio = |data: &'static [u8]| LlmEvent::AudioDelta {
            data: Bytes::from_static(data),
            transcript: Some("hi".to_owned()),
        };
        answer.apply(audio(b"ab"), &context(), &sink).unwrap();
        answer.apply(audio(b"cd"), &context(), &sink).unwrap();
        answer.apply(audio(b"e"), &context(), &sink).unwrap_err();
        assert_eq!(
            answer.parts,
            [Part::Text {
                text: "hihi".to_owned()
            }],
            "the transcript is a text part"
        );
        assert_eq!(answer.speech(), b"abcd", "the refused delta is not kept");
    }

    #[test]
    fn spoken_audio_holds_a_part() {
        let sink = RecordingSink::new();
        let mut answer = small(100, 100, 2);
        answer.apply(text(0, "a"), &context(), &sink).unwrap();
        let audio = LlmEvent::AudioDelta {
            data: Bytes::from_static(b"ab"),
            transcript: None,
        };
        answer.apply(audio.clone(), &context(), &sink).unwrap();
        answer.apply(audio, &context(), &sink).unwrap();
        let refused = answer.apply(
            LlmEvent::PartStarted {
                index: 1,
                kind: PartKind::Reasoning,
            },
            &context(),
            &sink,
        );
        assert!(
            refused.is_err(),
            "the text part and the audio part fill two"
        );
        assert_eq!(answer.speech(), b"abab");
    }
}
