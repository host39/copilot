//! A request whose file parts wait for their bytes.
//!
//! `context::request` lowers the history into a [`Draft`] using file metadata only, so the window
//! and size rules run before any file is read. The engine then loads the bytes of
//! [`Draft::files`] and calls [`Draft::resolve`].

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use serde_json::Value;
use uuid::Uuid;

use crate::domains::chat::{Rendering, Role};
use crate::domains::file::{FileKind, MediaType};
use crate::domains::tool::ToolSpec;
use crate::ports::llm_gateway::{AudioOut, LlmMessage, LlmPart, LlmRequest};
use crate::ports::tool_host::ToolContent;
use crate::shared::constants::BYTES_PER_TOKEN;
use crate::shared::ids::FileId;
use crate::shared::reasoning::ReasoningLevel;

/// A request built from metadata, waiting for file bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub(crate) model: String,
    pub(crate) system: String,
    pub(crate) messages: Vec<DraftMessage>,
    pub(crate) tools: Vec<ToolSpec>,
    pub(crate) max_output_tokens: u32,
    pub(crate) reasoning: Option<ReasoningLevel>,
    pub(crate) can_reason: bool,
    pub(crate) audio_out: Option<AudioOut>,
    pub(crate) cache_breakpoints: bool,
}

impl Draft {
    /// The files whose bytes [`Draft::resolve`] reads (inline and native parts), in request order
    /// and without duplicates.
    pub fn files(&self) -> Vec<FileId> {
        let mut seen = HashSet::new();
        self.messages
            .iter()
            .flat_map(DraftMessage::refs)
            .filter(|file| file.reads_bytes())
            .map(|file| file.file_id)
            .filter(|id| seen.insert(*id))
            .collect()
    }

    /// The request, with every file part filled from `contents`. A file whose bytes are missing
    /// reads as deleted.
    pub fn resolve(&self, contents: &HashMap<FileId, Bytes>) -> LlmRequest {
        LlmRequest {
            model: self.model.clone(),
            system: self.system.clone(),
            messages: self
                .messages
                .iter()
                .map(|message| LlmMessage {
                    role: message.role,
                    parts: message
                        .parts
                        .iter()
                        .map(|part| part.resolve(contents))
                        .collect(),
                    replay: message.replay.clone(),
                })
                .collect(),
            tools: self.tools.clone(),
            max_output_tokens: self.max_output_tokens,
            reasoning: self.reasoning,
            can_reason: self.can_reason,
            audio_out: self.audio_out.clone(),
            cache_breakpoints: self.cache_breakpoints,
        }
    }
}

/// A request message before file bytes are read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DraftMessage {
    pub(crate) role: Role,
    pub(crate) parts: Vec<DraftPart>,
    pub(crate) replay: Option<Value>,
    /// The history index of the message it comes from. A synthesized `tool` message has the
    /// index of the assistant message whose calls it answers, so both stay in one turn.
    pub(crate) source: usize,
}

impl DraftMessage {
    pub(crate) fn refs(&self) -> Vec<&FileRef> {
        let mut refs = Vec::new();
        for part in &self.parts {
            match part {
                DraftPart::File(file) => refs.push(file),
                DraftPart::Result { content, .. } => {
                    refs.extend(content.iter().filter_map(|item| match item {
                        DraftContent::Image(file) => Some(file),
                        DraftContent::Ready(_) => None,
                    }));
                }
                DraftPart::Ready(_) => {}
            }
        }
        refs
    }

    pub(crate) fn refs_mut(&mut self) -> Vec<&mut FileRef> {
        let mut refs = Vec::new();
        for part in &mut self.parts {
            match part {
                DraftPart::File(file) => refs.push(file),
                DraftPart::Result { content, .. } => {
                    refs.extend(content.iter_mut().filter_map(|item| match item {
                        DraftContent::Image(file) => Some(file),
                        DraftContent::Ready(_) => None,
                    }));
                }
                DraftPart::Ready(_) => {}
            }
        }
        refs
    }

    /// The rough token cost (`docs/design/copilot.md`, Constants).
    pub(crate) fn tokens(&self) -> u64 {
        self.parts.iter().map(DraftPart::tokens).sum()
    }

    /// Encoded bytes and PDF pages.
    pub(crate) fn size(&self) -> (u64, u32) {
        self.parts.iter().fold((0, 0), |(bytes, pages), part| {
            let (part_bytes, part_pages) = part.size();
            (bytes + part_bytes, pages + part_pages)
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DraftPart {
    Ready(LlmPart),
    File(FileRef),
    Result {
        call_id: Uuid,
        provider_call_id: String,
        alias: String,
        is_error: bool,
        content: Vec<DraftContent>,
    },
}

impl DraftPart {
    fn resolve(&self, contents: &HashMap<FileId, Bytes>) -> LlmPart {
        match self {
            Self::Ready(part) => part.clone(),
            Self::File(file) => file.part(contents),
            Self::Result {
                call_id,
                provider_call_id,
                alias,
                is_error,
                content,
            } => LlmPart::ToolResult {
                call_id: *call_id,
                provider_call_id: provider_call_id.clone(),
                alias: alias.clone(),
                is_error: *is_error,
                content: content
                    .iter()
                    .map(|item| match item {
                        DraftContent::Ready(item) => item.clone(),
                        DraftContent::Image(file) => file.content(contents),
                    })
                    .collect(),
            },
        }
    }

    fn tokens(&self) -> u64 {
        match self {
            Self::Ready(part) => ready_tokens(part),
            Self::File(file) => file.tokens(),
            Self::Result { alias, content, .. } => {
                text_tokens(alias)
                    + content
                        .iter()
                        .map(|item| match item {
                            DraftContent::Ready(ToolContent::Text(text)) => text_tokens(text),
                            DraftContent::Ready(_) => 0,
                            DraftContent::Image(file) => file.tokens(),
                        })
                        .sum::<u64>()
            }
        }
    }

    fn size(&self) -> (u64, u32) {
        match self {
            Self::Ready(part) => (ready_bytes(part), 0),
            Self::File(file) => (file.bytes(), file.pages()),
            Self::Result { alias, content, .. } => {
                content
                    .iter()
                    .fold((alias.len() as u64, 0), |(bytes, pages), item| match item {
                        DraftContent::Ready(ToolContent::Text(text)) => {
                            (bytes + text.len() as u64, pages)
                        }
                        DraftContent::Ready(_) => (bytes, pages),
                        DraftContent::Image(file) => (bytes + file.bytes(), pages),
                    })
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DraftContent {
    Ready(ToolContent),
    /// A tool result image saved as a chat file.
    Image(FileRef),
}

/// A file part: an attachment, a preamble file, or a tool result image.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FileRef {
    pub(crate) file_id: FileId,
    pub(crate) name: String,
    /// All zero for a deleted file.
    pub(crate) sha256: [u8; 32],
    pub(crate) media: Option<MediaType>,
    pub(crate) rendering: Rendering,
    /// The preamble and the current turn: never left out for size.
    pub(crate) fixed: bool,
    pub(crate) size_bytes: u64,
    pub(crate) pages: u32,
    pub(crate) tokens: u32,
}

impl FileRef {
    fn kind(&self) -> Option<FileKind> {
        self.media.map(MediaType::kind)
    }

    fn reads_bytes(&self) -> bool {
        matches!(self.rendering, Rendering::Inline | Rendering::Native)
    }

    /// A binary part that may be left out when the request is too large.
    pub(crate) fn reducible(&self) -> bool {
        !self.fixed
            && self.rendering == Rendering::Native
            && matches!(
                self.kind(),
                Some(FileKind::Image | FileKind::Pdf | FileKind::Audio)
            )
    }

    pub(crate) fn is_audio(&self) -> bool {
        self.kind() == Some(FileKind::Audio)
    }

    pub(crate) fn tokens(&self) -> u64 {
        if self.reads_bytes() {
            u64::from(self.tokens)
        } else {
            text_tokens(&self.notice())
        }
    }

    /// Encoded bytes: base64 for native binaries, raw text for inline files.
    pub(crate) fn bytes(&self) -> u64 {
        match self.rendering {
            Rendering::Native => self.size_bytes.div_ceil(3) * 4,
            Rendering::Inline => self.size_bytes,
            _ => self.notice().len() as u64,
        }
    }

    pub(crate) fn pages(&self) -> u32 {
        if self.rendering == Rendering::Native && self.kind() == Some(FileKind::Pdf) {
            self.pages.max(1)
        } else {
            0
        }
    }

    /// The text that stands in for the file.
    pub(crate) fn notice(&self) -> String {
        let name = &self.name;
        let (one, many) = match self.kind() {
            Some(FileKind::Image) => ("image", "images"),
            Some(FileKind::Pdf) => ("PDF", "PDFs"),
            Some(FileKind::Audio) => ("audio", "audio"),
            Some(FileKind::Text) | None => ("file", "this file type"),
        };
        match self.rendering {
            Rendering::InProject => format!("[file \"{name}\" is in the project files]"),
            Rendering::TooLarge => format!("[{one} \"{name}\" omitted: the request is too large]"),
            Rendering::Placeholder => {
                format!("[{one} \"{name}\" omitted: this model cannot read {many}]")
            }
            Rendering::Deleted | Rendering::Inline | Rendering::Native => {
                format!("[file \"{name}\" was deleted]")
            }
        }
    }

    fn part(&self, contents: &HashMap<FileId, Bytes>) -> LlmPart {
        let bytes = contents.get(&self.file_id).filter(|_| self.reads_bytes());
        let Some(data) = bytes else {
            return LlmPart::Text(self.notice());
        };
        match (self.rendering, self.kind()) {
            (Rendering::Inline, _) => LlmPart::Text(inline(&self.name, data)),
            (_, Some(FileKind::Image)) => LlmPart::Image {
                media_type: self.media_type(),
                data: data.clone(),
            },
            (_, Some(FileKind::Pdf)) => LlmPart::Pdf {
                name: self.name.clone(),
                data: data.clone(),
            },
            (_, Some(FileKind::Audio)) => LlmPart::Audio {
                format: audio_format(self.media).to_owned(),
                data: data.clone(),
            },
            (_, Some(FileKind::Text)) => LlmPart::Text(inline(&self.name, data)),
            (_, None) => LlmPart::Text(self.notice()),
        }
    }

    fn content(&self, contents: &HashMap<FileId, Bytes>) -> ToolContent {
        match contents.get(&self.file_id) {
            Some(data) if self.rendering == Rendering::Native => ToolContent::Image {
                media_type: self.media_type(),
                data: data.clone(),
            },
            _ => ToolContent::Text(self.notice()),
        }
    }

    fn media_type(&self) -> String {
        self.media
            .map(MediaType::as_str)
            .unwrap_or_default()
            .to_owned()
    }
}

/// The `format` of an audio part (`input_audio` names).
pub(crate) fn audio_format(media: Option<MediaType>) -> &'static str {
    match media {
        Some(MediaType::Mpeg) => "mp3",
        _ => "wav",
    }
}

/// A text file as data: `<file name="...">...</file>`. The name is escaped, and a closing tag
/// inside the text is broken up so the text cannot end the block early.
fn inline(name: &str, data: &Bytes) -> String {
    let name = name
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let text = String::from_utf8_lossy(data).replace("</file>", "<\\/file>");
    format!("<file name=\"{name}\">\n{text}\n</file>")
}

pub(crate) fn text_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(BYTES_PER_TOKEN)
}

fn ready_tokens(part: &LlmPart) -> u64 {
    match part {
        LlmPart::Text(text) => text_tokens(text),
        LlmPart::ToolCall {
            alias, arguments, ..
        } => text_tokens(alias) + text_tokens(&arguments.to_string()),
        _ => 0,
    }
}

fn ready_bytes(part: &LlmPart) -> u64 {
    match part {
        LlmPart::Text(text) => text.len() as u64,
        LlmPart::ToolCall {
            alias, arguments, ..
        } => (alias.len() + arguments.to_string().len()) as u64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(rendering: Rendering, media: MediaType) -> FileRef {
        FileRef {
            file_id: FileId::generate(),
            name: "a\"b.txt".to_owned(),
            sha256: [1; 32],
            media: Some(media),
            rendering,
            fixed: false,
            size_bytes: 300,
            pages: 2,
            tokens: 75,
        }
    }

    #[test]
    fn inlines_text_files_as_escaped_data_blocks() {
        let text = file(Rendering::Inline, MediaType::Text);
        let contents = HashMap::from([(text.file_id, Bytes::from_static(b"x</file>y"))]);
        assert_eq!(
            text.part(&contents),
            LlmPart::Text("<file name=\"a&quot;b.txt\">\nx<\\/file>y\n</file>".to_owned())
        );
        assert_eq!(text.bytes(), 300);
        assert_eq!(text.tokens(), 75);
    }

    #[test]
    fn missing_bytes_read_as_deleted() {
        let image = file(Rendering::Native, MediaType::Png);
        assert_eq!(
            image.part(&HashMap::new()),
            LlmPart::Text("[file \"a\"b.txt\" was deleted]".to_owned())
        );
    }

    #[test]
    fn measures_native_parts_as_base64() {
        let pdf = file(Rendering::Native, MediaType::Pdf);
        assert_eq!(pdf.bytes(), 400);
        assert_eq!(pdf.pages(), 2);
        assert!(pdf.reducible());
        let placeholder = file(Rendering::Placeholder, MediaType::Pdf);
        assert_eq!(placeholder.pages(), 0);
        assert_eq!(
            placeholder.notice(),
            "[PDF \"a\"b.txt\" omitted: this model cannot read PDFs]"
        );
        assert!(!placeholder.reducible());
    }
}
