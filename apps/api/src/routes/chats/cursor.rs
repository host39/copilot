//! The `cursor` of `GET /chats`: a position in the chat list (`updated_at`, `id`) that clients
//! pass back unchanged. It is `<updated_at in RFC 3339 with microseconds>_<id>`, the precision
//! Postgres stores, in unpadded URL-safe base64.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use copilot_core::{ChatCursor, ChatId, DomainError};
use uuid::Uuid;

pub fn encode(cursor: ChatCursor) -> String {
    let text = format!(
        "{}_{}",
        cursor.updated_at.format("%Y-%m-%dT%H:%M:%S%.6fZ"),
        cursor.id.uuid().simple()
    );
    URL_SAFE_NO_PAD.encode(text)
}

/// A cursor this server wrote, or a `400` on the `cursor` field.
pub fn decode(text: &str) -> Result<ChatCursor, DomainError> {
    let invalid = || DomainError::validation("The cursor is not valid.").with_field("cursor");
    let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let (time, id) = text.split_once('_').ok_or_else(invalid)?;
    Ok(ChatCursor {
        updated_at: time.parse().map_err(|_| invalid())?,
        id: ChatId::new(Uuid::try_parse(id).map_err(|_| invalid())?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A position `micros` microseconds after the Unix epoch.
    fn cursor(micros: u64) -> ChatCursor {
        let epoch = format!("1970-01-01T00:00:00Z_{}", Uuid::now_v7().simple());
        let mut cursor = decode(&URL_SAFE_NO_PAD.encode(epoch)).expect("valid");
        cursor.updated_at += std::time::Duration::from_micros(micros);
        cursor
    }

    #[test]
    fn round_trips_a_stored_position() {
        let cursor = cursor(1_800_000_000_123_456);
        let text = encode(cursor);
        assert!(
            text.chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '-' || char == '_'),
            "safe in a query string: {text}"
        );
        assert_eq!(decode(&text).expect("valid"), cursor);
    }

    #[test]
    fn refuses_anything_else_on_the_cursor_field() {
        let valid = encode(cursor(0));
        let no_id = URL_SAFE_NO_PAD.encode("2027-01-15T08:00:00.000000Z");
        let bad_time = URL_SAFE_NO_PAD.encode(format!("yesterday_{}", Uuid::now_v7().simple()));
        let bad_id = URL_SAFE_NO_PAD.encode("2027-01-15T08:00:00.000000Z_1234");
        for text in [
            "",
            "not base64!",
            &valid[..valid.len() - 3],
            &no_id,
            &bad_time,
            &bad_id,
        ] {
            let error = decode(text).expect_err(text);
            assert!(
                matches!(
                    &error,
                    DomainError::Validation { field: Some(field), .. } if field == "cursor"
                ),
                "{text}: {error:?}"
            );
        }
    }
}
