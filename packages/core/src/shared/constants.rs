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
/// Tool list cache lifetime when the server sends no `ttlMs`.
pub const TOOL_CACHE_DEFAULT_TTL_SECS: u64 = 3_600;
/// Longest tool list cache lifetime, whatever `ttlMs` the server sends.
pub const TOOL_CACHE_MAX_TTL_SECS: u64 = 86_400;
/// Provider rounds per turn before tool calls get `cancelled` results.
pub const MAX_TOOL_ROUNDS: u32 = 8;
/// Output token limit of a model without one: this, or a quarter of its context window when
/// that is smaller.
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 8_192;
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
/// Scheduled runs that execute at once.
pub const MAX_SCHEDULED_RUNS: usize = 2;
pub const SCHEDULER_TICK_SECS: u64 = 30;
/// Shortest gap between two fires of a schedule.
pub const SCHEDULE_MIN_INTERVAL_MINUTES: u32 = 15;
/// Enabled schedules per user.
pub const MAX_SCHEDULES_PER_USER: u32 = 20;
/// Consecutive failed runs before a schedule disables itself.
pub const SCHEDULE_MAX_FAILURES: u32 = 3;
/// A refresh enables every listed model when the provider lists at most this many.
pub const MODEL_AUTO_ENABLE_MAX: usize = 20;
/// Chat titles are the first user message, cut to this many characters.
pub const TITLE_MAX_CHARS: usize = 60;
/// Project files above this share of the context window warn.
pub const CONTEXT_WARN_RATIO: f64 = 0.8;
/// Token estimates for project files and context windows (rough, by design).
pub const TOKENS_PER_PDF_PAGE: u32 = 3_000;
pub const TOKENS_PER_IMAGE: u32 = 1_600;
pub const BYTES_PER_TOKEN: u64 = 4;
/// Push-to-talk clip limits.
pub const AUDIO_CLIP_MAX_SECS: u32 = 300;
pub const AUDIO_SAMPLE_RATE: u32 = 16_000;
pub const SSE_KEEP_ALIVE_SECS: u64 = 15;
pub const SHUTDOWN_DRAIN_SECS: u64 = 10;

/// Defaults of `MAX_UPLOAD_MB` and `MAX_STORAGE_MB_PER_USER` (read by `AppConfig::parse`).
pub const DEFAULT_MAX_UPLOAD_MB: u64 = 25;
pub const DEFAULT_MAX_STORAGE_MB_PER_USER: u64 = 1_024;

/// Longest name of a provider, model, project, schedule, or tool server.
pub const NAME_MAX_CHARS: usize = 100;
/// Longest file name (display only).
pub const FILE_NAME_MAX_CHARS: usize = 255;

/// Common durations.
pub mod time {
    use std::time::Duration;

    pub const SECOND: Duration = Duration::from_secs(1);
    pub const MINUTE: Duration = Duration::from_secs(60);
    pub const HOUR: Duration = Duration::from_secs(60 * 60);
    pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    pub const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);
}
