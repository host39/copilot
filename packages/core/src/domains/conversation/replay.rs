//! Which assistant messages go back to the provider with their `provider_state`
//! (`docs/design/copilot.md`, Replay Rules).

use crate::domains::chat::{ContextHash, Message, MessageStatus};
use crate::shared::ids::{ModelId, ProviderId};

/// What a request is sent to, for the replay rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReplayContext {
    pub(crate) provider_id: ProviderId,
    pub(crate) model_id: ModelId,
    /// The `gemini` preset also needs the same model (thought signatures are per model).
    pub(crate) same_model: bool,
    pub(crate) hash: ContextHash,
    /// The request offers tools; without them, stored tool-use blocks cannot be sent back.
    pub(crate) sends_tools: bool,
}

/// Whether `message` goes back verbatim. `current` marks a message of the current turn (the
/// last user message onward), which replays whatever the snapshot hash is, as long as it came
/// from the same connection: provider state from another connection cannot be read there.
/// Earlier messages also need the request's `context_hash`. `cancelled`, `error`, and `refused`
/// messages never replay.
pub(crate) fn replays(message: &Message, current: bool, context: &ReplayContext) -> bool {
    if message.provider_state().is_none() {
        return false;
    }
    if !matches!(
        message.status(),
        MessageStatus::Complete | MessageStatus::AwaitingApproval
    ) {
        return false;
    }
    if message.provider_id() != Some(context.provider_id) {
        return false;
    }
    if context.same_model && message.model_id() != Some(context.model_id) {
        return false;
    }
    if !context.sends_tools && message.calls().next().is_some() {
        return false;
    }
    current || message.context_hash() == Some(&context.hash)
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::domains::chat::{Approval, MessageParts, Part, ToolCallPart};
    use crate::domains::tool::ToolAlias;
    use crate::shared::ids::{CallId, ChatId, UserId};

    fn context() -> ReplayContext {
        ReplayContext {
            provider_id: ProviderId::generate(),
            model_id: ModelId::generate(),
            same_model: false,
            hash: ContextHash::new([1; 32]),
            sends_tools: true,
        }
    }

    fn answer(context: &ReplayContext) -> MessageParts {
        let message = Message::assistant(
            UserId::new(Uuid::now_v7()),
            ChatId::generate(),
            context.model_id,
            context.provider_id,
            DateTime::from_timestamp(1_767_225_600, 0).unwrap(),
        );
        MessageParts {
            status: MessageStatus::Complete,
            context_hash: Some(context.hash),
            provider_state: Some(json!({"thinking": "signed"})),
            parts: vec![Part::Text {
                text: "Hi".to_owned(),
            }],
            ..message.to_parts()
        }
    }

    #[test]
    fn replays_an_earlier_message_with_the_same_provider_and_hash() {
        let context = context();
        assert!(replays(
            &Message::restore(answer(&context)),
            false,
            &context
        ));
    }

    #[test]
    fn an_earlier_message_needs_the_same_hash() {
        let context = context();
        let other = Message::restore(MessageParts {
            context_hash: Some(ContextHash::new([2; 32])),
            ..answer(&context)
        });
        assert!(!replays(&other, false, &context));
        assert!(
            replays(&other, true, &context),
            "the current turn ignores the hash"
        );
    }

    #[test]
    fn every_message_needs_the_same_provider() {
        let context = context();
        let moved = Message::restore(MessageParts {
            provider_id: Some(ProviderId::generate()),
            ..answer(&context)
        });
        assert!(!replays(&moved, false, &context));
        assert!(!replays(&moved, true, &context));
        let deleted = Message::restore(MessageParts {
            provider_id: None,
            ..answer(&context)
        });
        assert!(!replays(&deleted, true, &context));
    }

    #[test]
    fn gemini_also_needs_the_same_model() {
        let context = ReplayContext {
            same_model: true,
            ..context()
        };
        let other_model = Message::restore(MessageParts {
            model_id: Some(ModelId::generate()),
            ..answer(&context)
        });
        assert!(!replays(&other_model, false, &context));
        assert!(!replays(&other_model, true, &context));
        let plain = ReplayContext {
            same_model: false,
            ..context
        };
        assert!(replays(&other_model, false, &plain));
    }

    #[test]
    fn never_replays_unfinished_or_stateless_messages() {
        let context = context();
        for status in [
            MessageStatus::Cancelled,
            MessageStatus::Error,
            MessageStatus::Refused,
            MessageStatus::Streaming,
        ] {
            let message = Message::restore(MessageParts {
                status,
                ..answer(&context)
            });
            assert!(!replays(&message, true, &context), "{status}");
        }
        let stateless = Message::restore(MessageParts {
            provider_state: None,
            ..answer(&context)
        });
        assert!(!replays(&stateless, true, &context));
    }

    #[test]
    fn tool_calls_need_tools_in_the_request() {
        let context = context();
        let call = ToolCallPart {
            id: CallId::generate(),
            provider_call_id: Some("toolu_1".to_owned()),
            server_id: None,
            tool_name: "list_schedules".to_owned(),
            alias: ToolAlias::builtin("list_schedules"),
            arguments_raw: "{}".to_owned(),
            arguments: Some(json!({})),
            complete: true,
            approval: Approval::Auto,
            deny_reason: None,
        };
        let calling = Message::restore(MessageParts {
            parts: vec![Part::ToolCall(call)],
            ..answer(&context)
        });
        assert!(replays(&calling, true, &context));
        let without_tools = ReplayContext {
            sends_tools: false,
            ..context
        };
        assert!(!replays(&calling, true, &without_tools));
    }
}
