//! `conversation`: the conversation engine. It builds each model request from a turn snapshot
//! (`context`), decides which tool calls run, ask, or are refused (`approval`), and runs a
//! prepared turn round by round (`ChatEngine`), reporting live through a `TurnSink`.
//! It owns no repository ports; it uses the chat, tool, file, and schedule domains.

mod events;

pub use crate::domains::conversation::events::TurnEvent;
