# Track B: Providers

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other track checkout; merged before each push). Base: `p1-frozen`. Roadmap step: P2.B. Owns `packages/providers/**`.

## Status

In progress: part 2 (P2.B.4 Anthropic adapter, P2.B.5 Gemini spike). Part 1 (the OpenAI-compatible adapter beyond text) is done. Last green CI: see the latest `p2.b` commit.

## Sub-Steps

- [x] P2.B.1 OpenAI-compatible requests beyond text: tools as `function` definitions with object schemas (`strict: false`), `tool_choice: "auto"` except on Ollama, assistant `tool_calls` (`content: null` without text), one `role: tool` message per result (text only; images, audio, and binary content become a notice, links `name: uri`), lowered result text as user text, adjacent user messages merged; images as base64 data URLs, PDFs as `file` parts with a `data:application/pdf;base64,` URL, `input_audio` (`wav`, `mp3`; anything else is `PROVIDER_UNSUPPORTED`); spoken replies (`modalities`, `audio: {voice, format}`); `reasoning_effort` per level (`none` for off), OpenRouter's `reasoning: {effort}`; replay of the stored state (reasoning text under the key it came under, OpenRouter `reasoning_details` instead when present, Gemini extra call fields that never replace `id`, `type`, or `function`). Tests: `src/openai/request/tests.rs` (every dialect, part, and replay case), wiremock tool round, dialect, binary part, and replay fixtures in `tests/openai.rs`.
- [x] P2.B.2 OpenAI-compatible stream decoding: text and reasoning runs as parts (a new part when the kind changes or a tool call comes between); reasoning under `reasoning` or `reasoning_content` (first key kept; `reasoning` wins when both come), OpenRouter `reasoning_details` merged by `index` (text, summary, and data appended; signatures filled in) and shown only when the chunk has no plain text; tool calls reassembled by `index`, by `id` without one, or continuing the last call, with another `id` under a known `index` starting a new call (servers that number whole calls `0`); calls end at the finish reason; `stop` (or no reason) with calls reports `tool_calls`; arguments after a call ended are dropped; `delta.audio` data (base64 PCM) and transcript pieces; usage including OpenRouter's repeated finish frame and Together's flat `cached_tokens`; the replay state (`{reasoning_key, reasoning, reasoning_details, tool_call_extra}`) just before `Finished`, capped at 8 MiB; error chunks (`error`, vLLM `object: "error"`), named `error` events, and JSON replies to a streamed request fail without echoing the provider. Tests: `src/openai/decode/tests.rs`, `src/openai/calls.rs`, `src/openai/replay.rs`, wiremock finish-reason, mid-stream error, JSON-reply, and HTTP status fixtures.
- [x] P2.B.3 Model listing with what the provider reports: `{data: [...]}` or a bare array (Together); display names from `name` or `display_name`; context windows from `context_length`, `context_window` (Groq), `max_model_len` (vLLM), or `top_provider.context_length`; max output from `max_completion_tokens` or `top_provider.max_completion_tokens`; OpenRouter `architecture` modalities and `supported_parameters` as `vision`, `audio_in`, `audio_out`, `pdf` (only when `file` is listed), `tools`, and `reasoning`; the `models/` prefix dropped on the `gemini` preset; on the `ollama` preset each model's `POST /api/show` (`capabilities`, `model_info.*.context_length`) for lists of up to 64 models, four at a time, a failed one keeping the defaults. Core still applies the preset defaults and locks. Tests: `src/openai/models.rs`, wiremock OpenRouter, Ollama, and Gemini listing fixtures.
- [ ] P2.B.4 Anthropic adapter: Messages API with `x-api-key` and `anthropic-version: 2026-10-21`, adaptive thinking with summarized display, signatures and `redacted_thinking` kept as provider state, `cache_control` breakpoints, refusal with `stop_details`, tool use and results, images and PDFs, model listing with reported capabilities. Tests: wiremock fixtures for text, thinking, signatures, tool-call argument deltas, replay state, usage, every stop reason, HTTP and mid-stream errors.
- [ ] P2.B.5 Gemini spike through the OpenAI-compatible endpoint against a Gemini 3 or 4 model (streamed tool calls over two turns with thought signatures as provider state, image and PDF parts, `input_audio`); outcome in ADR-005. Needs a Gemini API key.
- [ ] P2.B.6 Verify the core request limits and `DEFAULT_MAX_OUTPUT_TOKENS` per preset (roadmap P1.3 "verify in P2.B"); see Notes.

## Blockers

- P2.B.5 needs a live Gemini key; the wire shape of thought signatures on the OpenAI-compatible endpoint (`tool_calls[].extra_content.google.thought_signature`) is handled generically (every extra call field is kept on the `gemini` dialect) but is not yet confirmed against a live response. Google's current OpenAI compatibility page only says Gemini 3 "supports OpenAI compatibility for thought signatures".

## Interface-Change Requests

None blocking. Suggestions for a later serial step:

- `Dialect::stream_usage` is true for the `openai` preset only. Ollama and vLLM also document `stream_options.include_usage` and send no usage without it, so their answers save no token counts. Turning it on for the `ollama` dialect would fix that for Ollama; `custom` servers vary.

## Notes

Decisions beyond the spec text (for the integrator to write into `docs/design/copilot.md`, Adapter Rules, when Phase 2 merges):

- **OpenRouter reasoning level** goes in its unified `reasoning: {effort}` parameter (OpenRouter documents `none` to `max` there and does not document a top-level `reasoning_effort`); every other OpenAI-compatible dialect sends `reasoning_effort`.
- **PDF parts** go out as `file` parts on every preset when core sends them (core sends them only to models with `pdf`, which defaults on for `openai` and `openrouter` and is otherwise a user override).
- **Replayed reasoning text** goes back under the key it came under on every dialect, as the Replay Rules say. Risk to verify (unconfirmed here): some servers may reject unknown assistant fields (Groq may reject `reasoning` on input messages, and DeepSeek's own API has rejected `reasoning_content` on input for some models); both would reach Copilot as `custom` connections. If that shows up, limit plain-text replay to the current turn or to the `ollama` and `openrouter` dialects.
- **Assistant text parts** are joined with a blank line (they are separate parts: text before and after a tool call, or a transcript).
- **Tool schemas** that are not objects, or lack `type` or `properties`, are completed to `{type: "object", properties: {}}` because Chat Completions servers reject them otherwise.
- **`<think>` tags** in `content` (servers in raw reasoning mode) are not split out; such text shows as answer text.
- **The deprecated `function_call` delta** is not read (its finish reason still maps to `tool_calls`).

Limits to verify in P2.B.6 (from `docs/research/providers.md`): OpenAI Chat Completions accepts up to 512 MB of images per request and 50 MB of PDFs per file and in total; Anthropic 128 MB per request and 600 PDF pages (100 below a 1M context window); Gemini inline data 20 MB (spike). Core's `REQUEST_MAX_BYTES` (32 MiB), `GEMINI_REQUEST_MAX_BYTES` (20 MiB), and `REQUEST_MAX_PDF_PAGES` (100) sit inside all of them.
