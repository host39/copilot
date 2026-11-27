//! Tuning values (`docs/design/copilot.md`, Constants). Session, login-throttle, password, and
//! Argon2 values belong to lib-rustauth.

/// Database reads a use case runs at once (`parallel-db-reads`: the pool holds 4 connections).
pub const MAX_PARALLEL_READS: usize = 4;

/// Default pagination. A request may override limit and offset; these apply when it does not.
pub const DEFAULT_PAGE_SIZE: u32 = 25;
pub const DEFAULT_PAGE_OFFSET: u32 = 0;

/// Connect timeout of every outbound connection (providers and MCP servers). Outbound clients
/// have no total timeout, because it would cut long streams; the idle timeouts below end a
/// response that stops sending.
pub const OUTBOUND_CONNECT_TIMEOUT_SECS: u64 = 10;
/// Longest silence in a provider response. Reasoning models may think for minutes before the
/// first byte.
pub const PROVIDER_IDLE_TIMEOUT_SECS: u64 = 300;
/// Timeout of one MCP tool call.
pub const MCP_CALL_TIMEOUT_SECS: u64 = 60;
/// Longest silence in an MCP response: a call never outlives its timeout anyway.
pub const MCP_IDLE_TIMEOUT_SECS: u64 = MCP_CALL_TIMEOUT_SECS;
/// Tool result text kept for the model and for storage.
pub const MCP_RESULT_MAX_BYTES: usize = 65_536;
/// Largest MCP response (JSON body or SSE event) the client accepts.
pub const MCP_RESPONSE_MAX_BYTES: usize = 4_194_304;
/// One MCP connection attempt, lifecycle included (`server/discover`, `initialize`, or both).
pub const MCP_CONNECT_TIMEOUT_SECS: u64 = 15;
/// A whole MCP tool listing: connecting, every page, and closing.
pub const MCP_LIST_TIMEOUT_SECS: u64 = 30;
/// Closing an MCP connection (a legacy session's `DELETE`).
pub const MCP_CLOSE_TIMEOUT_SECS: u64 = 5;
/// `tools/list` pages read in one listing.
pub const MCP_LIST_MAX_PAGES: usize = 16;
/// Tools one server may list. Providers accept far fewer per request; the cap bounds the work
/// a hostile server's listing can cause.
pub const MCP_TOOLS_MAX: usize = 512;
/// After a failed listing, a turn lists that server again only after this long (its stale list,
/// if any, is used meanwhile), so a server that is down slows at most one turn every few minutes.
pub const TOOL_REFRESH_BACKOFF_SECS: i64 = 300;
/// Tool list cache lifetime when the server sends no `ttlMs`.
pub const TOOL_CACHE_DEFAULT_TTL_SECS: u64 = 3_600;
/// Longest tool list cache lifetime, whatever `ttlMs` the server sends.
pub const TOOL_CACHE_MAX_TTL_SECS: u64 = 86_400;
/// Provider rounds per turn before tool calls get `cancelled` results.
pub const MAX_TOOL_ROUNDS: u32 = 8;
/// An answer may stream text, reasoning, and tool calls of up to its output token limit times
/// `BYTES_PER_TOKEN` times this, and never more than `ANSWER_MAX_BYTES`; past that the engine
/// ends it as a bad response and keeps what streamed. Providers that ignore the limit cannot
/// grow one answer without bound.
pub const ANSWER_SLACK: u64 = 4;
pub const ANSWER_MAX_BYTES: u64 = 8 * 1_024 * 1_024;
/// Parts (text, reasoning, tool calls) of one answer.
pub const ANSWER_MAX_PARTS: usize = 256;
/// Sample rate of spoken reply audio: `pcm16` is 24 kHz mono 16-bit little-endian (OpenAI
/// Chat Completions audio output), which the saved WAV file states.
pub const AUDIO_OUT_SAMPLE_RATE: u32 = 24_000;
/// Spoken reply audio of one answer: five minutes of 24 kHz PCM16 (checked in P2.B against
/// OpenAI's `pcm16` output).
pub const AUDIO_OUT_MAX_BYTES: u64 = AUDIO_CLIP_MAX_SECS as u64 * AUDIO_OUT_SAMPLE_RATE as u64 * 2;
/// Output token limit of a model without one: this, or a quarter of its context window when
/// that is smaller.
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8_192;
/// The same for a model with `reasoning`: its reasoning tokens count against the limit (OpenAI
/// `max_completion_tokens`), so a high-effort answer needs more room.
pub const DEFAULT_REASONING_OUTPUT_TOKENS: u32 = 32_768;
/// Request size limits, as encoded bytes (base64 adds about 33%) and PDF pages. Rough values
/// until P2.B verifies each provider; Gemini's OpenAI-compatible endpoint takes less.
pub const REQUEST_MAX_BYTES: u64 = 32 * 1_024 * 1_024;
pub const GEMINI_REQUEST_MAX_BYTES: u64 = 20 * 1_024 * 1_024;
pub const REQUEST_MAX_PDF_PAGES: u32 = 100;
/// Voice and format of spoken replies (`audio_out` models with Speak replies on).
pub const AUDIO_OUT_VOICE: &str = "alloy";
pub const AUDIO_OUT_FORMAT: &str = "pcm16";
/// Live runs per user.
pub const MAX_ACTIVE_RUNS_PER_USER: usize = 3;
/// Uploads whose bodies are read at once, per user and in all (each is held in memory whole);
/// more wait their turn.
pub const UPLOADS_PER_USER: usize = 2;
pub const UPLOADS_AT_ONCE: usize = 4;
/// An upload body may take `UPLOAD_GRACE_SECS` plus one second per `UPLOAD_MIN_BYTES_PER_SEC`
/// it has sent so far: a client slower than that (or one that stops sending) is refused, so it
/// cannot hold an upload slot for long.
pub const UPLOAD_GRACE_SECS: u64 = 30;
pub const UPLOAD_MIN_BYTES_PER_SEC: u64 = 16 * 1_024;
/// File downloads sent at once, per user and in all; more wait their turn.
pub const DOWNLOADS_PER_USER: usize = 4;
pub const DOWNLOADS_AT_ONCE: usize = 32;
/// A download reads its file in pieces of this size, one query each.
pub const DOWNLOAD_CHUNK_BYTES: u64 = 512 * 1_024;
/// A download whose client takes no piece for this long ends, freeing its slot.
pub const DOWNLOAD_STALL_SECS: u64 = 60;
/// Longest wait for a request's headers once a connection is open.
pub const HEADER_READ_TIMEOUT_SECS: u64 = 30;
/// Scheduled runs that execute at once.
pub const MAX_SCHEDULED_RUNS: usize = 2;
/// Longest scheduled run (a fire or a Run now): past it the run is stopped and recorded as
/// failed, so its slot frees for other schedules.
pub const SCHEDULED_RUN_MAX_SECS: u64 = 30 * 60;
pub const SCHEDULER_TICK_SECS: u64 = 30;
/// Shortest gap between two fires of a schedule.
pub const SCHEDULE_MIN_INTERVAL_MINUTES: u32 = 15;
/// Enabled schedules per user.
pub const MAX_SCHEDULES_PER_USER: u32 = 20;
/// Consecutive failed runs before a schedule disables itself.
pub const SCHEDULE_MAX_FAILURES: u32 = 3;
/// A refresh enables every listed model when the provider lists at most this many.
pub const MODEL_AUTO_ENABLE_MAX: usize = 20;
/// Models a refresh accepts, and the models a connection keeps: past this, a refresh deletes
/// unavailable models that are off and unused, least recently updated first.
pub const MAX_MODELS_PER_PROVIDER: usize = 2_000;
/// Longest model id a refresh accepts (remote display names are cut to `NAME_MAX_CHARS`).
pub const MODEL_ID_MAX_CHARS: usize = 256;
/// Chat titles are the first user message, cut to this many characters.
pub const TITLE_MAX_CHARS: usize = 60;
/// Project files above this share of the context window warn.
pub const CONTEXT_WARN_RATIO: f64 = 0.8;
/// Token estimates for project files and context windows (rough, by design).
pub const TOKENS_PER_PDF_PAGE: u32 = 3_000;
pub const TOKENS_PER_IMAGE: u32 = 1_600;
pub const BYTES_PER_TOKEN: u64 = 4;
/// Audio, by how long it plays: Gemini documents 32 tokens per second, the most of the
/// providers that state a rate, so a clip never counts for less than it costs.
pub const TOKENS_PER_AUDIO_SECOND: u64 = 32;
/// Longest audio in a chat: a push-to-talk clip (the client stops recording there), any audio
/// file uploaded to a chat (refused past it), and a spoken reply (`AUDIO_OUT_MAX_BYTES`).
pub const AUDIO_CLIP_MAX_SECS: u32 = 300;
/// Sample rate the client records push-to-talk clips at (16 kHz mono PCM16 WAV).
pub const AUDIO_SAMPLE_RATE: u32 = 16_000;
pub const SSE_KEEP_ALIVE_SECS: u64 = 15;
/// A stream left open checks its session again this often, and ends once the session has ended
/// (signed out, revoked, expired, or the account disabled or deleted).
pub const SESSION_RECHECK_SECS: u64 = 60;
/// A `GET /api/events` stream ends after this long and the client connects again, so a session
/// signed out elsewhere stops receiving events within this time.
pub const EVENTS_STREAM_MAX_SECS: u64 = 900;
/// Run streams (the turn routes and `GET /api/chats/{id}/stream`) and `GET /api/events` streams
/// a user may have open at once; opening one more ends the user's oldest.
pub const CHAT_STREAMS_PER_USER: usize = 16;
pub const EVENT_STREAMS_PER_USER: usize = 8;
/// What a run's replay buffer may hold across all its answers: their text, the 4 bytes that mark
/// where each merged delta ended, and the other events. Past it the run is stopped, so a provider
/// that streams tiny deltas through a long tool loop cannot grow one run without bound.
pub const RUN_BUFFER_MAX_BYTES: usize = 32 * 1_024 * 1_024;
/// On shutdown, live runs get this long to finish on their own; then they are cancelled and get
/// up to `SHUTDOWN_DRAIN_SECS` more to save their partial answers. Stopping a run (or deleting
/// its chat or project) waits up to `SHUTDOWN_DRAIN_SECS` for it to save too.
pub const SHUTDOWN_GRACE_SECS: u64 = 5;
pub const SHUTDOWN_DRAIN_SECS: u64 = 10;

/// Defaults of `MAX_UPLOAD_MB` and `MAX_STORAGE_MB_PER_USER` (read by `AppConfig::parse`).
pub const DEFAULT_MAX_UPLOAD_MB: u64 = 25;
pub const DEFAULT_MAX_STORAGE_MB_PER_USER: u64 = 1_024;

/// Longest name of a provider, model, project, schedule, or tool server.
pub const NAME_MAX_CHARS: usize = 100;
/// Longest file name (display only).
pub const FILE_NAME_MAX_CHARS: usize = 255;
