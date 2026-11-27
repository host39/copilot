//! rmcp's tool list and tool results to the core port types. Everything here is untrusted
//! server input.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use copilot_core::{RemoteTool, ToolContent, ToolOutput};
use rmcp::model::{CallToolResult, ContentBlock, ResourceContents, Tool};
use serde_json::Value;

/// A listed tool. The title comes from the tool or its annotations; `readOnlyHint` is kept as
/// a hint only.
pub(crate) fn tool(tool: Tool) -> RemoteTool {
    let annotations = tool.annotations.as_ref();
    let title = tool
        .title
        .clone()
        .or_else(|| annotations.and_then(|annotations| annotations.title.clone()));
    RemoteTool {
        name: tool.name.into_owned(),
        title,
        description: tool
            .description
            .map(|description| description.into_owned())
            .unwrap_or_default(),
        input_schema: Value::Object(tool.input_schema.as_ref().clone()),
        read_only: annotations.and_then(|annotations| annotations.read_only_hint) == Some(true),
    }
}

/// A call result: text, images, and audio as they are (base64 decoded; core saves images and
/// cuts text), resource links and blob resources as links, embedded text resources as text.
/// `structuredContent` becomes JSON text only when the result has no content blocks, since
/// servers send it as text too.
pub(crate) fn output(result: CallToolResult) -> ToolOutput {
    let mut content: Vec<ToolContent> = result.content.into_iter().map(block).collect();
    if content.is_empty()
        && let Some(structured) = result.structured_content
    {
        content.push(ToolContent::Text(structured.to_string()));
    }
    ToolOutput {
        content,
        is_error: result.is_error == Some(true),
    }
}

fn block(block: ContentBlock) -> ToolContent {
    match block {
        ContentBlock::Text(text) => ToolContent::Text(text.text),
        ContentBlock::Image(image) => match decode(&image.data) {
            Some(data) => ToolContent::Image {
                media_type: image.mime_type,
                data,
            },
            None => ToolContent::Binary {
                media_type: Some(image.mime_type),
            },
        },
        ContentBlock::Audio(audio) => match decode(&audio.data) {
            Some(data) => ToolContent::Audio {
                media_type: audio.mime_type,
                data,
            },
            None => ToolContent::Binary {
                media_type: Some(audio.mime_type),
            },
        },
        ContentBlock::ResourceLink(link) => ToolContent::Link {
            uri: link.uri,
            name: Some(link.name),
        },
        ContentBlock::Resource(embedded) => match embedded.resource {
            ResourceContents::TextResourceContents { text, .. } => ToolContent::Text(text),
            ResourceContents::BlobResourceContents { uri, .. } => {
                ToolContent::Link { uri, name: None }
            }
            _ => ToolContent::Binary { media_type: None },
        },
        // A block kind newer than this client.
        _ => ToolContent::Binary { media_type: None },
    }
}

/// Standard base64, surrounding white space ignored; `None` when it does not decode.
fn decode(data: &str) -> Option<Bytes> {
    STANDARD.decode(data.trim()).ok().map(Bytes::from)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rmcp::model::{EmbeddedResource, Resource, ToolAnnotations};
    use serde_json::json;

    use super::*;

    fn schema() -> Arc<serde_json::Map<String, Value>> {
        let Value::Object(object) =
            json!({"type": "object", "properties": {"q": {"type": "string"}}})
        else {
            unreachable!("an object literal");
        };
        Arc::new(object)
    }

    #[test]
    fn maps_a_listed_tool_with_its_hint_and_title() {
        let mut listed = Tool::new("search", "Searches the docs.", schema());
        listed.annotations = Some(ToolAnnotations::with_title("Doc search").read_only(true));
        let remote = tool(listed);
        assert_eq!(remote.name, "search");
        assert_eq!(remote.title.as_deref(), Some("Doc search"));
        assert_eq!(remote.description, "Searches the docs.");
        assert_eq!(remote.input_schema["properties"]["q"]["type"], "string");
        assert!(remote.read_only);

        let mut bare = Tool::new("write", "", schema());
        bare.description = None;
        let remote = tool(bare);
        assert_eq!(remote.description, "");
        assert_eq!(remote.title, None);
        assert!(!remote.read_only, "no hint means not read-only");
    }

    #[test]
    fn maps_every_content_block() {
        let png = STANDARD.encode(b"\x89PNG");
        let result = CallToolResult::success(vec![
            ContentBlock::text("hello"),
            ContentBlock::image(png.clone(), "image/png"),
            ContentBlock::image("not base64!", "image/png"),
            ContentBlock::audio(STANDARD.encode(b"RIFF"), "audio/wav"),
            ContentBlock::ResourceLink(Resource::new("https://example.com/a", "a.txt")),
            ContentBlock::Resource(EmbeddedResource::new(ResourceContents::text(
                "inline",
                "file:///b",
            ))),
            ContentBlock::Resource(EmbeddedResource::new(
                ResourceContents::BlobResourceContents {
                    uri: "file:///c.bin".to_owned(),
                    mime_type: None,
                    blob: STANDARD.encode(b"blob"),
                    meta: None,
                },
            )),
        ]);
        let converted = output(result);
        assert!(!converted.is_error);
        assert_eq!(
            converted.content,
            vec![
                ToolContent::Text("hello".to_owned()),
                ToolContent::Image {
                    media_type: "image/png".to_owned(),
                    data: Bytes::from_static(b"\x89PNG"),
                },
                ToolContent::Binary {
                    media_type: Some("image/png".to_owned()),
                },
                ToolContent::Audio {
                    media_type: "audio/wav".to_owned(),
                    data: Bytes::from_static(b"RIFF"),
                },
                ToolContent::Link {
                    uri: "https://example.com/a".to_owned(),
                    name: Some("a.txt".to_owned()),
                },
                ToolContent::Text("inline".to_owned()),
                ToolContent::Link {
                    uri: "file:///c.bin".to_owned(),
                    name: None,
                },
            ]
        );
    }

    #[test]
    fn uses_structured_content_only_without_blocks() {
        let mut result = CallToolResult::structured(json!({"total": 3}));
        result.content.clear();
        assert_eq!(
            output(result).content,
            vec![ToolContent::Text("{\"total\":3}".to_owned())]
        );

        let mut both = CallToolResult::success(vec![ContentBlock::text("3 results")]);
        both.structured_content = Some(json!({"total": 3}));
        assert_eq!(
            output(both).content,
            vec![ToolContent::Text("3 results".to_owned())]
        );
    }

    #[test]
    fn keeps_the_error_flag() {
        let result = CallToolResult::error(vec![ContentBlock::text("no such file")]);
        let converted = output(result);
        assert!(converted.is_error);
        assert_eq!(
            converted.content,
            vec![ToolContent::Text("no such file".to_owned())]
        );
    }
}
