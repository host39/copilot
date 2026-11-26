//! `conversation`: the conversation engine. It builds each model request from a turn snapshot
//! (`context`), decides which tool calls run, ask, or are refused (`approval`), and runs a
//! prepared turn round by round (`ChatEngine`), reporting live through a `TurnSink`.
//! It owns no repository ports; it uses the chat, tool, file, and schedule domains.

pub mod context;
mod draft;
mod events;
mod hash;
mod lowering;
mod replay;

pub use crate::domains::conversation::context::{
    ContextInputs, DATA_NOTE, SCHEDULE_NOTE, build, output_limit, references, request,
};
pub use crate::domains::conversation::draft::Draft;
pub use crate::domains::conversation::events::TurnEvent;
