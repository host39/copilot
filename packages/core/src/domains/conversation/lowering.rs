//! History lowering: stored messages to request messages, in place and deterministic
//! (`docs/design/copilot.md`, Context Building, step 4).
//!
//! - A part the model cannot read becomes a notice (`[image "a.png" omitted: ...]`).
//! - An attachment that is also an included project file, or that was deleted, becomes a notice.
//! - Assistant audio becomes its transcript; reasoning never goes back as text.
//! - A call whose alias the request does not offer (or every call, when the request has no
//!   tools) becomes text on both sides: `[called <tool> with <arguments>]` and
//!   `[result: <text>]`.
//! - `cancelled` and `error` answers keep their text and finished calls; calls whose arguments
//!   never finished are dropped. `refused` answers are left out.
//! - Every call sent gets a result; a missing one is synthesized as cancelled.
//! - No message goes out without parts, and the request never ends with an assistant message.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::domains::chat::{
    Message, MessageStatus, Part, Rendering, ResultContent, Role, SnapshotTool, ToolResultPart,
};
use crate::domains::conversation::context::ContextInputs;
use crate::domains::conversation::draft::{DraftContent, DraftMessage, DraftPart, FileRef};
use crate::domains::conversation::replay::{ReplayContext, replays};
use crate::domains::file::{FileKind, StoredFile};
use crate::ports::llm_gateway::LlmPart;
use crate::ports::tool_host::ToolContent;
use crate::shared::ids::{CallId, FileId};

/// The text of a result synthesized for a call that has none (after a restart).
pub(crate) const UNFINISHED: &str = "The call was interrupted before it finished.";

/// What to lower and how.
pub(crate) struct Plan<'a> {
    /// The first history index in the window.
    pub(crate) start: usize,
    /// The index of the current turn's user message.
    pub(crate) current: usize,
    /// The file preamble, placed at the start of the first user message in the window.
    pub(crate) preamble: Vec<FileRef>,
    /// The preamble's files: attachments among them become "is in the project files".
    pub(crate) in_project: HashSet<FileId>,
    pub(crate) tools: &'a [SnapshotTool],
    pub(crate) sends_tools: bool,
    /// `None` while planning the window: nothing replays.
    pub(crate) replay: Option<ReplayContext>,
}

/// How a call went out, so its result goes out the same way.
enum CallForm {
    Structured {
        provider_call_id: String,
        alias: String,
    },
    Text,
}

struct Lowering<'a> {
    inputs: &'a ContextInputs<'a>,
    plan: &'a Plan<'a>,
    out: Vec<DraftMessage>,
    forms: HashMap<CallId, CallForm>,
    /// The calls of the last assistant message that still need a result, in call order.
    open: Vec<CallId>,
    /// The history index of that assistant message.
    open_source: usize,
}

pub(crate) fn lower(inputs: &ContextInputs<'_>, plan: &Plan<'_>) -> Vec<DraftMessage> {
    let mut lowering = Lowering {
        inputs,
        plan,
        out: Vec::new(),
        forms: HashMap::new(),
        open: Vec::new(),
        open_source: 0,
    };
    let start = plan.start.min(inputs.history.len());
    for (index, message) in inputs.history.iter().enumerate().skip(start) {
        match message.role() {
            Role::User => {
                lowering.close();
                lowering.user(index, message);
            }
            Role::Assistant => {
                lowering.close();
                lowering.assistant(index, message);
            }
            Role::Tool => lowering.tool(index, message),
        }
    }
    lowering.close();
    lowering.finish()
}

/// The id a rebuilt call goes out with: `call_` and its Copilot id without dashes.
pub(crate) fn neutral_id(id: CallId) -> String {
    format!("call_{}", id.uuid().simple())
}

impl Lowering<'_> {
    fn current(&self, index: usize) -> bool {
        index >= self.plan.current
    }

    fn user(&mut self, index: usize, message: &Message) {
        let fixed = self.current(index);
        let mut parts = Vec::new();
        for part in message.parts() {
            match part {
                Part::Text { text } if !text.is_empty() => {
                    parts.push(DraftPart::Ready(LlmPart::Text(text.clone())));
                }
                Part::File { file_id, name, .. } => {
                    parts.push(DraftPart::File(self.attachment(*file_id, name, fixed)));
                }
                Part::Audio { file_id, .. } => {
                    parts.push(DraftPart::File(self.attachment(*file_id, "audio", fixed)));
                }
                _ => {}
            }
        }
        self.push(Role::User, parts, None, index);
    }

    fn assistant(&mut self, index: usize, message: &Message) {
        if message.status() == MessageStatus::Refused {
            return;
        }
        let replayed = self
            .plan
            .replay
            .as_ref()
            .is_some_and(|context| replays(message, self.current(index), context));
        let unfinished = matches!(
            message.status(),
            MessageStatus::Cancelled | MessageStatus::Error
        );
        let mut parts = Vec::new();
        for part in message.parts() {
            match part {
                Part::Text { text } if !text.is_empty() => {
                    parts.push(DraftPart::Ready(LlmPart::Text(text.clone())));
                }
                Part::Audio {
                    transcript: Some(text),
                    ..
                } if !text.is_empty() => {
                    parts.push(DraftPart::Ready(LlmPart::Text(text.clone())));
                }
                Part::ToolCall(call) => {
                    if unfinished && !call.complete {
                        continue;
                    }
                    let known = self.plan.tools.iter().any(|tool| tool.alias == call.alias);
                    if self.plan.sends_tools && (known || replayed) {
                        let provider_call_id = if replayed {
                            call.provider_call_id
                                .clone()
                                .unwrap_or_else(|| neutral_id(call.id))
                        } else {
                            neutral_id(call.id)
                        };
                        parts.push(DraftPart::Ready(LlmPart::ToolCall {
                            call_id: call.id.uuid(),
                            provider_call_id: provider_call_id.clone(),
                            alias: call.alias.as_str().to_owned(),
                            arguments: call
                                .arguments
                                .clone()
                                .filter(Value::is_object)
                                .unwrap_or_else(|| Value::Object(Map::new())),
                        }));
                        self.forms.insert(
                            call.id,
                            CallForm::Structured {
                                provider_call_id,
                                alias: call.alias.as_str().to_owned(),
                            },
                        );
                    } else {
                        let arguments = if call.arguments_raw.trim().is_empty() {
                            "{}"
                        } else {
                            call.arguments_raw.as_str()
                        };
                        parts.push(DraftPart::Ready(LlmPart::Text(format!(
                            "[called {} with {arguments}]",
                            call.tool_name
                        ))));
                        self.forms.insert(call.id, CallForm::Text);
                    }
                    self.open.push(call.id);
                }
                _ => {}
            }
        }
        self.open_source = index;
        let replay = replayed
            .then(|| message.provider_state().cloned())
            .flatten();
        self.push(Role::Assistant, parts, replay, index);
    }

    fn tool(&mut self, index: usize, message: &Message) {
        let results: HashMap<CallId, &ToolResultPart> = message
            .parts()
            .iter()
            .filter_map(|part| match part {
                Part::ToolResult(result) => Some((result.call_id, result)),
                _ => None,
            })
            .collect();
        self.answer(&results, self.current(index), index);
    }

    /// Closes the open calls with synthesized results (no `tool` message followed them).
    fn close(&mut self) {
        if !self.open.is_empty() {
            let source = self.open_source;
            self.answer(&HashMap::new(), self.current(source), source);
        }
    }

    /// One `tool` message with a result for every open call, in call order: the stored result,
    /// else a synthesized one. Results lowered to text follow the structured ones.
    fn answer(&mut self, results: &HashMap<CallId, &ToolResultPart>, fixed: bool, source: usize) {
        let mut structured = Vec::new();
        let mut texts = Vec::new();
        for id in std::mem::take(&mut self.open) {
            let Some(form) = self.forms.get(&id) else {
                continue;
            };
            let (is_error, content) = match results.get(&id) {
                Some(result) => (
                    result.is_error,
                    result
                        .content
                        .iter()
                        .map(|item| self.result_content(item, fixed))
                        .collect(),
                ),
                None => (
                    true,
                    vec![DraftContent::Ready(ToolContent::Text(
                        UNFINISHED.to_owned(),
                    ))],
                ),
            };
            match form {
                CallForm::Structured {
                    provider_call_id,
                    alias,
                } => structured.push(DraftPart::Result {
                    call_id: id.uuid(),
                    provider_call_id: provider_call_id.clone(),
                    alias: alias.clone(),
                    is_error,
                    content,
                }),
                CallForm::Text => texts.push(DraftPart::Ready(LlmPart::Text(format!(
                    "[result: {}]",
                    result_text(&content)
                )))),
            }
        }
        let role = if structured.is_empty() {
            Role::User
        } else {
            Role::Tool
        };
        structured.extend(texts);
        self.push(role, structured, None, source);
    }

    fn result_content(&self, item: &ResultContent, fixed: bool) -> DraftContent {
        match item {
            ResultContent::Text(text) => DraftContent::Ready(ToolContent::Text(text.clone())),
            ResultContent::Image { file_id } => {
                let file = self.inputs.files.get(file_id);
                let rendering = match file {
                    None => Rendering::Deleted,
                    Some(_) if self.inputs.capabilities.vision => Rendering::Native,
                    Some(_) => Rendering::Placeholder,
                };
                DraftContent::Image(file_ref(*file_id, file, "image", rendering, fixed))
            }
        }
    }

    /// An attachment by what the model can read.
    fn attachment(&self, id: FileId, name: &str, fixed: bool) -> FileRef {
        let Some(file) = self.inputs.files.get(&id) else {
            return file_ref(id, None, name, Rendering::Deleted, fixed);
        };
        let rendering = if self.plan.in_project.contains(&id) {
            Rendering::InProject
        } else {
            readable(file, self.inputs)
        };
        file_ref(id, Some(file), name, rendering, fixed)
    }

    fn push(&mut self, role: Role, parts: Vec<DraftPart>, replay: Option<Value>, source: usize) {
        if !parts.is_empty() {
            self.out.push(DraftMessage {
                role,
                parts,
                replay,
                source,
            });
        }
    }

    /// Places the preamble and drops trailing assistant messages.
    fn finish(mut self) -> Vec<DraftMessage> {
        let preamble: Vec<DraftPart> = self
            .plan
            .preamble
            .iter()
            .cloned()
            .map(DraftPart::File)
            .collect();
        if !preamble.is_empty() {
            match self
                .out
                .iter_mut()
                .find(|message| message.role == Role::User && message.replay.is_none())
            {
                Some(first) => {
                    let rest = std::mem::take(&mut first.parts);
                    first.parts = preamble.into_iter().chain(rest).collect();
                }
                None => self.out.insert(
                    0,
                    DraftMessage {
                        role: Role::User,
                        parts: preamble,
                        replay: None,
                        source: self.plan.start,
                    },
                ),
            }
        }
        while self
            .out
            .last()
            .is_some_and(|message| message.role == Role::Assistant)
        {
            self.out.pop();
        }
        self.out
    }
}

/// How a file goes to this model by its kind and the model's capabilities. Text files always go
/// inline; a file of unknown type is a notice.
pub(crate) fn readable(file: &StoredFile, inputs: &ContextInputs<'_>) -> Rendering {
    let capabilities = inputs.capabilities;
    match file.media().map(|media| media.kind()) {
        Some(FileKind::Text) => Rendering::Inline,
        Some(FileKind::Image) if capabilities.vision => Rendering::Native,
        Some(FileKind::Pdf) if capabilities.pdf => Rendering::Native,
        Some(FileKind::Audio) if capabilities.audio_in => Rendering::Native,
        _ => Rendering::Placeholder,
    }
}

/// A file part from its metadata (`None` for a deleted file, named `fallback`).
pub(crate) fn file_ref(
    id: FileId,
    file: Option<&StoredFile>,
    fallback: &str,
    rendering: Rendering,
    fixed: bool,
) -> FileRef {
    match file {
        Some(file) => FileRef {
            file_id: id,
            name: file.name().to_owned(),
            sha256: *file.sha256(),
            media: file.media(),
            rendering,
            fixed,
            size_bytes: file.size_bytes(),
            pages: file.page_count().unwrap_or(0),
            tokens: file.token_estimate(),
        },
        None => FileRef {
            file_id: id,
            name: fallback.to_owned(),
            sha256: [0; 32],
            media: None,
            rendering: Rendering::Deleted,
            fixed,
            size_bytes: 0,
            pages: 0,
            tokens: 0,
        },
    }
}

/// The text of a result lowered to text; images read `[image]`.
fn result_text(content: &[DraftContent]) -> String {
    let texts: Vec<&str> = content
        .iter()
        .map(|item| match item {
            DraftContent::Ready(ToolContent::Text(text)) => text.as_str(),
            _ => "[image]",
        })
        .collect();
    texts.join("\n")
}
