# Provider Chat APIs

Research note for the Copilot provider adapters (Rust, `reqwest` plus SSE parsing). Verified against live primary sources on 2026-11-26. Every load-bearing claim has its source URL next to it. Claims marked **UNVERIFIED** could not be confirmed from a primary source during this research; confirm them with a live request before depending on them. Examples marked **illustrative** were assembled from a schema, not copied from a provider page.

Treat this note as a draft: the provider docs change often, and model names, versions, and limits below are a snapshot.

## Key Findings

1. **Anthropic has a new API version.** The current `anthropic-version` is `2026-10-21`. It removes `temperature`, `top_p`, and `top_k` (any of them returns 400 on every model), rejects `document` blocks placed directly in `assistant` messages, and rejects non-empty `citations` on user `text` blocks. `2023-06-01` is the previous version and stays usable. Source: <https://platform.claude.com/docs/en/api/versioning>, <https://platform.claude.com/docs/en/api/migration-2026-10-21>.
2. **Anthropic thinking is on by default on Claude 5.x models** and its text is hidden by default (`display: "omitted"`). Send `thinking: {"type": "adaptive", "display": "summarized"}` to get visible summaries. `budget_tokens` (`type: "enabled"`) returns 400 on Claude Opus 4.7 and every later numbered model; Claude Mythos Preview is the documented exception (it still accepts `enabled` with `budget_tokens`). Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>.
3. **Anthropic preserved thinking breaks if `system` or `tools` change mid-chat.** On Claude Fable 5.2, Mythos 5.2, Fable 5.1, Mythos 5.1, Opus 5.5, Sonnet 5.5, and Haiku 5.5 (the only models the page lists; Fable 5, Mythos 5, and Opus 5 are not among them, and the page says Opus 5 does not run the check), sending earlier thinking blocks after a change to `system`, `tools`, or an earlier message fails a prefix check (400 on some models, possible 400 on others). Copilot must snapshot a chat's system prompt and tool list at chat creation and resend the identical copy. Source: <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/prefix-checks>.
4. **OpenAI first-party is moving tool calling off Chat Completions.** GPT-6 Astra and GPT-6.1 Sol require the Responses API for tool calling, and from GPT-5.4 on, Chat Completions does not support tool calling with `reasoning_effort` other than `none`. Chat Completions "remains supported". Every self-hosted and aggregator target in scope still serves `/v1/chat/completions`. Source: <https://developers.openai.com/api/docs/guides/function-calling>, <https://developers.openai.com/api/docs/guides/migrate-to-responses>.
5. **Google now calls `generateContent` legacy.** The Interactions API (`POST /v1beta/interactions`) is GA as of June 2026 and recommended for new projects; `generateContent` "remains fully supported". Source: <https://aistudio.google.com/docs/interactions-overview>, <https://aistudio.google.com/llms.txt>.
6. **Gemini 3 and 4 require thought signatures to be returned during function calling**, or the request fails with 400. Source: <https://aistudio.google.com/docs/thought-signatures?generate-content=true>.
7. **Reasoning text field names differ across OpenAI-compatible servers:** `reasoning` (Ollama, vLLM, Groq, OpenRouter, some Together models), `reasoning_content` (llama.cpp, LM Studio per its docs, some Together models), plus `reasoning_details` (OpenRouter). The parser must accept all of them.
8. **Native PDF input:** Anthropic (`document` block), OpenAI first-party Chat Completions (`file` part, PDF only), Gemini (`inlineData` with `application/pdf`), and OpenRouter (any model, it parses when the model cannot). Ollama, llama.cpp, LM Studio, and vLLM document no PDF input for chat. For those, Copilot needs plain text extraction in the backend (a parser, not ML inference) or must refuse the attachment.

## Recommended Adapter Plan

| Adapter | Wire protocol | Covers |
| --- | --- | --- |
| `anthropic` | Messages API, `POST /v1/messages`, named SSE events | Anthropic first-party (also LM Studio, llama.cpp, and vLLM expose `/v1/messages`, see the sections below) |
| `openai_compat` | Chat Completions, `POST {base}/chat/completions`, data-only SSE | Ollama, vLLM, LM Studio, llama.cpp server, OpenRouter, Groq, Together, Gemini OpenAI endpoint, OpenAI models that do not need Responses for tools |
| `gemini` | `generateContent` / `streamGenerateContent?alt=sse` | Google AI Studio Gemini API |
| Later: `openai_responses` | Responses API, `POST {base}/responses`, typed SSE events | OpenAI GPT-6 family tool calling and reasoning summaries; Ollama, vLLM, LM Studio, llama.cpp, OpenRouter, Groq also serve it |
| Later: `gemini_interactions` | Interactions API, `POST /v1beta/interactions` with `store: false` | Gemini once `generateContent` is deprecated |

Reasons for Chat Completions as the generic target: it is the only endpoint every listed server documents (sources in the [OpenAI-Compatible Section](#openai-compatible-chat-completions)), and it is stateless like our storage model. The cost is that OpenAI's newest models need a Responses adapter for tool calling.

## Anthropic Messages API

### Endpoints, Auth, and Versioning

- Base URL `https://api.anthropic.com`. Create a message: `POST /v1/messages`. List models: `GET /v1/models`. Source: <https://platform.claude.com/docs/en/api/messages>, <https://platform.claude.com/docs/en/api/models/list>.
- Required headers: `Authorization: Bearer <key>` (or the legacy `x-api-key: <key>`, "still supported"), `anthropic-version`, `content-type: application/json`. `anthropic-workspace-id` is required only for keys not scoped to a workspace. Source: <https://platform.claude.com/docs/en/api/overview>.
- Current `anthropic-version`: `2026-10-21`. Previous: `2023-06-01`. The SDK major versions that send `2026-10-21` are Python `anthropic` 2.0.0, TypeScript `@anthropic-ai/sdk` 1.0.0, Go `anthropic-sdk-go/v2` 2.0.0. Source: <https://platform.claude.com/docs/en/api/migration-2026-10-21>.
- Beta features use the `anthropic-beta` header, for example `server-side-fallback-2026-07-01`. Source: <https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback>.
- Response header `request-id` identifies the request for support. Source: <https://platform.claude.com/docs/en/api/overview>.
- Request size limit: 128 MB for Messages on the first-party API (413 `request_too_large` above it). Source: <https://platform.claude.com/docs/en/api/overview>.

**Recommendation:** send `anthropic-version: 2026-10-21` and never send `temperature`, `top_p`, or `top_k` to Anthropic. Claude 4.7 and later models accept only the default sampling values anyway. Source: <https://platform.claude.com/docs/en/api/migration-2026-10-21>.

### Listing Models

`GET /v1/models?limit=20&after_id=...` (limit 1 to 1000, default 20; optional `lifecycle` filter `active`, `deprecated`, `retired`). Source: <https://platform.claude.com/docs/en/api/models/list>.

```json
{
  "data": [
    {
      "type": "model",
      "id": "claude-opus-5-5",
      "display_name": "Claude Opus 5.5",
      "created_at": "2026-...",
      "lifecycle": "active",
      "line": "opus",
      "max_input_tokens": 1000000,
      "max_tokens": 128000,
      "deprecated_at": null,
      "retires_at": null,
      "capabilities": {
        "image_input": {"supported": true},
        "pdf_input": {"supported": true},
        "thinking": {
          "supported": true,
          "types": {
            "adaptive": {"supported": true},
            "enabled": {"supported": false},
            "disabled": {"supported": false},
            "between_tools": {"supported": false}
          }
        },
        "effort": {"supported": true, "low": {"supported": true}, "medium": {"supported": true}, "high": {"supported": true}, "xhigh": {"supported": true}, "max": {"supported": true}}
      }
    }
  ],
  "first_id": "claude-opus-5-5",
  "last_id": "claude-opus-5-5",
  "has_more": false
}
```

The field names above are from the reference; the values are illustrative. Use `capabilities.thinking.types.*.supported`, `capabilities.image_input`, `capabilities.pdf_input`, and `max_tokens` to drive the model picker and request building instead of hardcoding per-model rules. Use `line` (not the `id`) to group models. Source: <https://platform.claude.com/docs/en/api/models/list>, <https://platform.claude.com/docs/en/models/overview>.

Current lineup (snapshot): `claude-fable-5-2`, `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`; all 1M context and 128K max output. Source: <https://platform.claude.com/docs/en/models/overview>.

### Request Shape

```json
{
  "model": "claude-opus-5-5",
  "max_tokens": 32000,
  "stream": true,
  "system": "You are a helpful assistant for the Acme project.",
  "thinking": {"type": "adaptive", "display": "summarized"},
  "output_config": {"effort": "medium"},
  "tools": [
    {
      "name": "get_weather",
      "description": "Get the current weather in a given location",
      "input_schema": {
        "type": "object",
        "properties": {"location": {"type": "string"}},
        "required": ["location"]
      }
    }
  ],
  "messages": [
    {"role": "user", "content": "Hello"},
    {"role": "assistant", "content": [{"type": "text", "text": "Hi, how can I help?"}]},
    {"role": "user", "content": [{"type": "text", "text": "What is the weather in Paris?"}]}
  ]
}
```

Rules (source for all: <https://platform.claude.com/docs/en/api/messages> unless noted):

- **`max_tokens` is required** (no "optional" marker; minimum 0). Thinking tokens count toward it. The per-model maximum comes from the Models API `max_tokens`. Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>.
- **System prompt:** top-level `system`, a string or an array of `text` blocks (blocks allow `cache_control`). Not a message.
- **`role: "system"` entries inside `messages` also exist** (`SystemMessageParam`), used for mid-conversation instructions and per-message `output_config.effort`. Some models do not accept them. Do not use them for the base system prompt. Source: <https://platform.claude.com/docs/en/build-with-claude/effort-per-message>, <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/change-tools-or-instructions>.
- **Turns:** alternate `user` and `assistant`. Consecutive same-role turns are merged into one turn. Max 100,000 messages per request.
- **No assistant prefill on new models:** Claude 4.6 and later models, and Claude Mythos Preview, reject a request whose last message is `assistant` with 400 `invalid_request_error`, message `This model does not support assistant message prefill. The conversation must end with a user message.` Source: <https://platform.claude.com/docs/en/api/errors>.
- **`content`** is a string (shorthand for one `text` block) or an array of blocks.
- **Effort:** `output_config.effort` is `low`, `medium`, `high`, `xhigh`, or `max`; defaults differ by model (Opus 5.5 `medium`, Sonnet 5.5 `high`). Source: <https://platform.claude.com/docs/en/api/messages>, <https://platform.claude.com/docs/en/models/overview>.

#### Images

```json
{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "<base64>"}}
{"type": "image", "source": {"type": "url", "url": "https://example.com/a.jpg"}}
{"type": "image", "source": {"type": "file", "file_id": "file_..."}}
```

Formats: `image/jpeg`, `image/png`, `image/gif`, `image/webp` (animations use the first frame only). Up to 600 images per request, but the 128 MB request limit can be hit first. Source: <https://platform.claude.com/docs/en/build-with-claude/vision>.

#### PDFs (Native)

```json
{"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": "<base64>"}}
{"type": "document", "source": {"type": "url", "url": "https://example.com/report.pdf"}}
{"type": "document", "source": {"type": "file", "file_id": "file_..."}}
```

- All active models support PDFs. Limits: 600 pages per request (100 when the request's context window is under 1M tokens), no encrypted PDFs. Each page is sent as extracted text plus a page image (at most 1,568 visual tokens per page; text typically 1,500 to 3,000 tokens per page). Source: <https://platform.claude.com/docs/en/build-with-claude/pdf-support>.
- With `2026-10-21`, `document` blocks belong in `user` messages only. Source: <https://platform.claude.com/docs/en/api/migration-2026-10-21>.
- Files API upload: `POST /v1/files` with multipart `file=@document.pdf`, then reference `file_id`. The PDF page example shows no beta header. Source: <https://platform.claude.com/docs/en/build-with-claude/pdf-support>.

#### Text Files

- Plain text document block: `{"type": "document", "source": {"type": "text", "media_type": "text/plain", "data": "..."}}`. Source: <https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls> (tool result example), <https://platform.claude.com/docs/en/api/messages> (`media_type: "text/plain"` source).
- `.txt`, `.csv`, `.md` can also be uploaded to the Files API as `text/plain`. Binary formats such as `.docx` and `.xlsx` are not supported in document blocks; convert them first. Source: <https://platform.claude.com/docs/en/build-with-claude/pdf-support>.
- Simplest portable option: put the file text in a `text` block wrapped in a delimiter that names the file.

### Tool Calling

- **Definition:** `{"name", "description", "input_schema"}` where `input_schema` is JSON Schema. Source: <https://platform.claude.com/docs/en/api/messages>.
- **`tool_choice`:** `{"type": "auto"}` (default), `{"type": "any"}`, `{"type": "tool", "name": "..."}`, `{"type": "none"}`; `auto`, `any`, and `tool` take `disable_parallel_tool_use: true`. Source: <https://platform.claude.com/docs/en/api/messages>.
- **Forced tool use is rejected (400)** on Claude Opus 5.5, Sonnet 5.5, Fable 5.2, Mythos 5.2, Fable 5.1, Mythos 5.1. Use `auto`. Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>.
- **The model asks** with `stop_reason: "tool_use"` and one or more `tool_use` blocks `{"type": "tool_use", "id": "toolu_...", "name": "...", "input": {...}}`. Source: <https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls>.
- **Send results back:** append the assistant `content` unchanged as an `assistant` message, then one `user` message with one `tool_result` per `tool_use` (even on failure), keeping the same `tools`. Source: <https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls>.

```json
{
  "role": "user",
  "content": [
    {"type": "tool_result", "tool_use_id": "toolu_01A09q90qw90lq917835lq9", "content": "15 degrees"},
    {"type": "tool_result", "tool_use_id": "toolu_01B...", "content": "Tool failed: timeout", "is_error": true},
    {"type": "text", "text": "Optional text, only AFTER all tool_result blocks"}
  ]
}
```

- `tool_result.content` is a string or an array of `text`, `image`, `document`, or `search_result` blocks; `is_error: true` signals failure. `tool_result` blocks must come first in the user message and must immediately follow the assistant tool-use message. Source: <https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls>.
- **Parallel calls:** every result goes in a single user message, with no text before the results. Source: <https://platform.claude.com/docs/en/agents-and-tools/tool-use/parallel-tool-use>.

### Streaming

Set `"stream": true`. Every event is a named SSE event whose `data` JSON repeats the name in `type`. Source: <https://platform.claude.com/docs/en/build-with-claude/streaming>.

Event flow:

1. `message_start`: a `Message` with empty `content` and initial `usage` (input tokens, cache tokens).
2. Per content block: `content_block_start` (with `index` and the empty block), any number of `content_block_delta`, then `content_block_stop`. `redacted_thinking` blocks and server tool results arrive whole, with no deltas.
3. One or more `message_delta` events with `delta.stop_reason`, `delta.stop_sequence`, and `usage`. **Usage in `message_delta` is cumulative.**
4. `message_stop`.
5. `ping` events can appear anywhere. `error` events can appear mid-stream. New event types may be added; ignore unknown ones.

Delta types inside `content_block_delta.delta`:

| `delta.type` | Field | Applies to |
| --- | --- | --- |
| `text_delta` | `text` | `text` blocks |
| `input_json_delta` | `partial_json` (string fragment) | `tool_use` and `server_tool_use` blocks |
| `thinking_delta` | `thinking` | `thinking` blocks |
| `signature_delta` | `signature` (sent once, just before `content_block_stop`) | `thinking` blocks |

Tool argument reassembly: concatenate `partial_json` strings for the block `index` and parse once at `content_block_stop`; the final `tool_use.input` is always an object. By default the API buffers each key/value until complete, so there can be pauses; per-tool `eager_input_streaming` enables fine-grained streaming. Source: <https://platform.claude.com/docs/en/build-with-claude/streaming>.

Minimal tool-call stream (verbatim from <https://platform.claude.com/docs/en/build-with-claude/streaming>, text deltas trimmed):

```text
event: message_start
data: {"type":"message_start","message":{"id":"msg_014p7gG3wDgGV9EUtLvnow3U","type":"message","role":"assistant","model":"claude-opus-5","stop_sequence":null,"usage":{"input_tokens":472,"output_tokens":2},"content":[],"stop_reason":null}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: ping
data: {"type": "ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Okay"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01T1x1fJ34qAmk2tNTrN7Up6","name":"get_weather","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"location\":"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"San"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" Francisc"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"o,"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" CA\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}

event: message_stop
data: {"type":"message_stop"}
```

#### Stop Reasons

| `stop_reason` | Meaning | Copilot action |
| --- | --- | --- |
| `end_turn` | Natural completion | Done |
| `max_tokens` | Hit `max_tokens` | Mark truncated, offer "continue" |
| `stop_sequence` | Hit a stop sequence (`stop_sequence` names it) | Done |
| `tool_use` | Wants client tools run | Run tools, send results |
| `pause_turn` | Server-tool loop hit its iteration limit | Send assistant content back unchanged to continue (only with server tools) |
| `refusal` | Declined (HTTP 200) | Show `stop_details.explanation`; discard partial output |
| `model_context_window_exceeded` | Filled the context window | Mark truncated |

Source: <https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons>. In streams, `stop_reason` is `null` in `message_start` and arrives in `message_delta`. Source: same page.

Refusal shape: `"stop_reason": "refusal"` with `"stop_details": {"type": "refusal", "category": "cyber" | "bio" | "frontier_llm" | "reasoning_extraction" | "general_harms" | null, "explanation": "..." | null}`. `stop_details` is `null` for other stop reasons. A refusal can arrive before any output or mid-stream after partial output; treat partial output as incomplete and discard it. The `explanation` text is not stable: display it, do not parse it. Source: <https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback>.

#### Errors

HTTP error body: `{"type": "error", "error": {"type": "<error_type>", "message": "..."}}`. Types: 400 `invalid_request_error`, 401 `authentication_error`, 402 `billing_error`, 403 `permission_error`, 404 `not_found_error`, 409 `conflict_error`, 413 `request_too_large`, 429 `rate_limit_error`, 500 `api_error`, 504 `timeout_error`, 529 `overloaded_error`. Retry 500 only when `x-should-retry` is not `false`. Source: <https://platform.claude.com/docs/en/api/errors>.

In-stream error event (HTTP status already 200): Source: <https://platform.claude.com/docs/en/build-with-claude/streaming>.

```text
event: error
data: {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}
```

Resuming an interrupted stream on Claude 4.6 and later: do not prefill; send a new user message containing the partial response plus an instruction to continue. Thinking and tool-use blocks cannot be partially recovered. Source: <https://platform.claude.com/docs/en/build-with-claude/streaming>.

### Thinking

Per-model behavior (source: <https://platform.claude.com/docs/en/build-with-claude/thinking>):

| Model | No `thinking` field | `adaptive` | `enabled` + `budget_tokens` | `disabled` |
| --- | --- | --- | --- | --- |
| Opus 5.5, Fable 5.x, Mythos 5.x | Adaptive | Adaptive | 400 | 400 |
| Sonnet 5.5 | Adaptive | Adaptive | 400 | 400 (use `between_tools` to turn off up-front thinking, `high` effort or below) |
| Haiku 5.5, Opus 5 | Adaptive | Adaptive | 400 | Off at `high` effort or below |
| Sonnet 5 | Adaptive | Adaptive | 400 | Off |
| Opus 4.8, Opus 4.7 | Off | Adaptive | 400 | Off |
| Mythos Preview | Adaptive | Adaptive | Extended | 400 |
| Opus 4.6, Sonnet 4.6 | Off | Adaptive | Deprecated | Off |
| Opus 4.5, Sonnet 4.5, Haiku 4.5 | Off | 400 | Extended | Off |

- `display`: `"summarized"` returns summary text; `"omitted"` returns `thinking` blocks with an empty `thinking` string (default on Claude 5.x, Opus 4.8, Opus 4.7, and Mythos Preview). `"updates"` is beta (`thinking-display-updates-2026-08-18`) and returns only short progress updates between tool calls. Billing is the same either way. Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>.
- Streaming with `omitted`: the block opens, one `thinking_delta` with an empty string, one `signature_delta`, then stop. Source: same page.
- `redacted_thinking` blocks: `{"type": "redacted_thinking", "data": "..."}`, opaque, arrive whole. Source: same page.
- **What must be passed back:** within a tool-use turn, every `thinking` and `redacted_thinking` block must go back complete and unmodified with the `tool_use` it accompanied; across turns, passing everything back is recommended and the API filters what it does not need. Modified blocks return 400 (``...`thinking` or `redacted_thinking` blocks in the latest assistant message cannot be modified...``). A filter on `type == "thinking"` alone silently drops `redacted_thinking` and breaks the protocol. Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>, <https://platform.claude.com/docs/en/api/errors>.
- **Do not strip thinking blocks when switching between Claude models**; the API drops blocks the target model cannot read without billing them. Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>.
- **Prefix checks (preserved-thinking models):** a change to `system`, `tools`, or any earlier message invalidates thinking that follows it; the result is a 400 on Fable 5.2, Mythos 5.2, Mythos 5.1 and a possible 400 on Fable 5.1, Opus 5.5, Sonnet 5.5, Haiku 5.5. The documented fix is to build `system` and `tools` once per conversation, send that copy every time, and append later instructions as a `role: "system"` message (and tool changes as `tool_addition` / `tool_removal` blocks under beta headers). Source: <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/prefix-checks>, <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/change-tools-or-instructions>.
- Asking the model to write its reasoning into the answer text can produce a `refusal` with category `reasoning_extraction` on some models; read thinking blocks instead. Source: <https://platform.claude.com/docs/en/build-with-claude/thinking>.

**Chat UI:** show non-empty `thinking` text in a collapsed "Thinking" section; render nothing for empty blocks; never show `signature` or `redacted_thinking.data`; store all of them verbatim for replay.

## OpenAI-Compatible (Chat Completions)

### Chat Completions vs Responses

- OpenAI: "While Chat Completions remains supported, Responses is recommended for all new projects." Reasoning summaries are a Responses-only feature. Source: <https://developers.openai.com/api/docs/guides/migrate-to-responses>, <https://developers.openai.com/api/docs/guides/reasoning>.
- "GPT-6 Astra and GPT-6.1 Sol require the Responses API for tool calling." Source: <https://developers.openai.com/api/docs/guides/function-calling>.
- "Starting with GPT-5.4, Chat Completions does not support tool calling with `reasoning_effort` values other than `none`." GPT-6.2 Astra and GPT-6.2 Sol reject `reasoning_effort: "none"`. Source: <https://developers.openai.com/api/docs/guides/migrate-to-responses>, <https://developers.openai.com/api/docs/guides/reasoning>.

Endpoint support across the targets:

| Server | `/chat/completions` | `/responses` | `/models` | Source |
| --- | --- | --- | --- | --- |
| OpenAI | Yes | Yes | Yes | <https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create> |
| Ollama | Yes | Yes, stateless only (since v0.13.3) | Yes, plus native `/api/tags` | <https://docs.ollama.com/api/openai-compatibility> |
| vLLM | Yes | Yes | Yes | <https://docs.vllm.ai/en/latest/serving/online_serving/> |
| LM Studio | Yes | Yes | Yes | <https://lmstudio.ai/docs/developer/openai-compat> |
| llama.cpp server | Yes | Yes | Yes | <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md> |
| OpenRouter | Yes | Yes | Yes (UNVERIFIED path `/api/v1/models`) | <https://openrouter.ai/docs/api/reference/streaming> |
| Groq | Yes | Yes | UNVERIFIED | <https://console.groq.com/docs/openai> |
| Together | Yes | UNVERIFIED | UNVERIFIED | <https://docs.together.ai/docs/openai-api-compatibility> |
| Gemini OpenAI endpoint | Yes | UNVERIFIED | UNVERIFIED | <https://aistudio.google.com/docs/openai> |

**Decision:** the `openai_compat` adapter targets Chat Completions. Mark OpenAI first-party GPT-6 family models as "tools unavailable via this adapter" until an `openai_responses` adapter exists.

### Base URLs and Auth

| Provider | Base URL | Auth |
| --- | --- | --- |
| OpenAI | `https://api.openai.com/v1` | `Authorization: Bearer <key>`; optional `OpenAI-Organization`, `OpenAI-Project` |
| Ollama | `http://localhost:11434/v1` | Key required by clients but ignored locally |
| LM Studio | `http://localhost:1234/v1` | None by default |
| llama.cpp server | `http://localhost:9931/v1` (default `--port 9931`) | `--api-key` enables bearer auth |
| vLLM | `http://localhost:8000/v1` (UNVERIFIED default port) | `--api-key` (UNVERIFIED header details) |
| OpenRouter | `https://openrouter.ai/api/v1` | `Authorization: Bearer <key>` |
| Groq | `https://api.groq.com/openai/v1` | `Authorization: Bearer <key>` |
| Together | `https://api.together.ai/v1` (US-only: `https://us-api.together.ai/v1`) | `Authorization: Bearer <key>` |
| Gemini (OpenAI mode) | `https://generativelanguage.googleapis.com/v1beta/openai/` | `Authorization: Bearer <gemini key>` (UNVERIFIED, from SDK usage) |

Sources: <https://developers.openai.com/api/reference/overview>, <https://docs.ollama.com/api/openai-compatibility>, <https://lmstudio.ai/docs/developer/openai-compat>, <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md>, <https://openrouter.ai/docs/api/reference/streaming>, <https://console.groq.com/docs/openai>, <https://docs.together.ai/docs/openai-api-compatibility>, <https://aistudio.google.com/docs/openai>.

### Listing Models

`GET {base}/models`: Source: <https://developers.openai.com/api/reference/resources/models/methods/list>.

```json
{
  "object": "list",
  "data": [
    {"id": "model-id-0", "object": "model", "created": 1686935002, "owned_by": "organization-owner", "shutdown_date": null}
  ]
}
```

The response carries no capability info (vision, tools, context window). Copilot needs per-model overrides in its provider config.

Ollama native `GET http://localhost:11434/api/tags`: Source: <https://docs.ollama.com/api/tags>.

```json
{
  "models": [
    {
      "name": "gemma4",
      "model": "gemma4",
      "modified_at": "2025-10-03T23:34:03.409490317-07:00",
      "size": 9608350245,
      "digest": "c6eb396d...",
      "details": {"format": "gguf", "family": "gemma4", "families": ["gemma4"], "parameter_size": "8.0B", "quantization_level": "Q4_K_M"}
    }
  ]
}
```

`/v1/models` on Ollama: `created` is the last-modified time and `owned_by` is the Ollama username, default `"library"`. Source: <https://docs.ollama.com/api/openai-compatibility>. Use `/api/tags` when the user picks "Ollama" so the picker can show size and quantization.

### Request Shape

```json
{
  "model": "gpt-5.6-terra",
  "stream": true,
  "stream_options": {"include_usage": true},
  "max_tokens": 4096,
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": [
      {"type": "text", "text": "Summarize this PDF and describe the image."},
      {"type": "file", "file": {"filename": "report.pdf", "file_data": "data:application/pdf;base64,<base64>"}},
      {"type": "image_url", "image_url": {"url": "data:image/png;base64,<base64>", "detail": "auto"}}
    ]},
    {"role": "assistant", "content": null, "tool_calls": [
      {"id": "call_abc", "type": "function", "function": {"name": "get_weather", "arguments": "{\"location\":\"Paris\"}"}}
    ]},
    {"role": "tool", "tool_call_id": "call_abc", "content": "15 degrees"}
  ],
  "tools": [
    {"type": "function", "function": {"name": "get_weather", "description": "Get weather", "parameters": {"type": "object", "properties": {"location": {"type": "string"}}, "required": ["location"]}}}
  ],
  "tool_choice": "auto"
}
```

Field rules (source: <https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create> unless noted):

- **System prompt:** a first message with `role: "system"`. OpenAI says "With o1 models and newer, `developer` messages replace the previous `system` messages." Send `developer` to `api.openai.com` and `system` to everything else. Whether the newest OpenAI models still accept `system`: UNVERIFIED.
- **User content parts:** `text`; `image_url` (`url` is an https URL or base64 data, `detail` is `auto`, `low`, `high`, or `original`); `input_audio`; `file` (`file_data` base64, `file_id`, `filename`).
- **Assistant:** `content` (required unless `tool_calls` present), `tool_calls` (array of `{id, type: "function", function: {name, arguments}}`, where `arguments` is a JSON string), `refusal`.
- **Tool result:** `{"role": "tool", "tool_call_id": "...", "content": "..."}`; content is a string or `text` parts only. One tool message per call.
- **Output limit:** OpenAI deprecates `max_tokens` in favor of `max_completion_tokens` (which includes reasoning tokens), and `max_tokens` "is not compatible with o-series models". Ollama and LM Studio document only `max_tokens`. Make the parameter name a per-provider setting: `max_completion_tokens` for OpenAI, `max_tokens` elsewhere. Source: <https://docs.ollama.com/api/openai-compatibility>, <https://lmstudio.ai/docs/developer/openai-compat/chat-completions>.
- **`reasoning_effort`:** `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` (model dependent). Ollama maps it to model thinking levels; llama.cpp disables thinking on `none`; Gemini's OpenAI endpoint maps it to `thinking_level`. Sources: <https://docs.ollama.com/api/openai-compatibility>, <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md>, <https://aistudio.google.com/docs/openai>.
- **Images:** OpenAI accepts PNG, JPEG, WEBP, non-animated GIF, up to 1,500 images and 512 MB per request. Source: <https://developers.openai.com/api/docs/guides/images-vision>. **Ollama accepts base64 images only, not image URLs.** Source: <https://docs.ollama.com/api/openai-compatibility>. Always send data URLs.
- **PDFs:** Chat Completions accepts **PDF only** in `file` parts (`file_data` or `file_id`; no `file_url`), each file under 50 MB and 50 MB combined. "To use text from a file in Chat Completions, read the file in your application and send its contents as a `text` content part." Source: <https://developers.openai.com/api/docs/guides/file-inputs>. The OpenAI reference only says `file_data` is "base64 encoded file data", but the Chat Completions examples on <https://developers.openai.com/api/docs/guides/file-inputs> (JavaScript, Python, Go, Java, Ruby) send `file_data` as `data:application/pdf;base64,<base64>`, and OpenRouter's Chat Completions examples do the same (<https://openrouter.ai/docs/features/multimodal/pdfs>). Send the data-URL form. Whether raw base64 without the prefix is also accepted is UNVERIFIED (the curl example only shows a placeholder).
- **OpenRouter PDFs** work on any model: native when supported, otherwise a `file-parser` plugin (`cloudflare-ai`, `mistral-ocr`, or `native` engines) parses it. Source: <https://openrouter.ai/docs/features/multimodal/pdfs>.
- **Ollama unsupported fields:** `tool_choice`, `logit_bias`, `user`, `n`. Source: <https://docs.ollama.com/api/openai-compatibility>. Omit `tool_choice` for Ollama.
- **Groq unsupported:** `logprobs`, `logit_bias`, `top_logprobs`, `messages[].name`, `n` other than 1 (400 if sent). Source: <https://console.groq.com/docs/openai>.

### Tool Calling

- **Definition:** `{"type": "function", "function": {"name", "description", "parameters", "strict"}}`. Name: a-z, A-Z, 0-9, underscores, dashes, max 64 chars. `strict` defaults to `false` in Chat Completions. Source: <https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create>, <https://developers.openai.com/api/docs/guides/function-calling>.
- **`tool_choice`:** `"none"`, `"auto"`, `"required"`, or `{"type": "function", "function": {"name": "..."}}`. Default `none` without tools and `auto` with tools. Source: same reference.
- **The model asks** with `finish_reason: "tool_calls"` and `message.tool_calls`. Results go back as `role: "tool"` messages with matching `tool_call_id`, after the assistant message that holds `tool_calls`. Source: same reference.
- **Parallel calls:** on by default (`parallel_tool_calls: true`); set `false` for zero or one call. Source: <https://developers.openai.com/api/docs/guides/function-calling>.

### Streaming

Data-only SSE: each event is `data: <chat.completion.chunk JSON>`, terminated by `data: [DONE]`. Source: <https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create> (streaming example).

```text
data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-6.2-astra","system_fingerprint":"fp_44709d6fcb","choices":[{"index":0,"delta":{"role":"assistant","content":""},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-6.2-astra","system_fingerprint":"fp_44709d6fcb","choices":[{"index":0,"delta":{"content":"Hello"},"logprobs":null,"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-6.2-astra","system_fingerprint":"fp_44709d6fcb","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"stop"}]}

data: [DONE]
```

Chunk schema (source: <https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events>):

- `choices[].delta`: `role`, `content`, `refusal`, `tool_calls`, `audio`, deprecated `function_call`.
- `choices[].delta.tool_calls[]`: `{index, id?, type?: "function", function?: {name?, arguments?}}`.
- `choices[].finish_reason`: `stop`, `length`, `tool_calls`, `content_filter`, `function_call` (deprecated), or `null`.
- `choices` can be empty on the final chunk when `stream_options.include_usage` is true; that chunk carries `usage`, and every other chunk has `usage: null`. If the stream is interrupted, the usage chunk may never arrive. Source: <https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create>.
- An `obfuscation` string field is added by default on OpenAI (disable with `stream_options.include_obfuscation: false`); ignore it. Source: same.
- Usage fields: `prompt_tokens`, `completion_tokens`, `total_tokens`, `completion_tokens_details.reasoning_tokens`, `prompt_tokens_details.cached_tokens`, `prompt_tokens_details.cache_write_tokens`. Source: same. Together: some non-reasoning models return `cached_tokens` flat at the top of `usage`. Source: <https://docs.together.ai/docs/openai-api-compatibility>.

**Tool-call reassembly:** keep a map keyed by `tool_calls[].index`. The first fragment for an index carries `id`, `type`, and `function.name`; later fragments carry `function.arguments` substrings to concatenate. Parse the JSON after `finish_reason: "tool_calls"` (or at stream end). Ollama appears to emit each tool call whole in one chunk (inferred from the `ToolCall` struct with `index` in its source, UNVERIFIED by a live request), which the same accumulator handles. Source: <https://github.com/ollama/ollama/blob/main/openai/openai.go>.

Minimal tool-call stream (**illustrative**, assembled from the chunk schema above):

```text
data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"locat"}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ion\":\"Paris\"}"}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":82,"completion_tokens":18,"total_tokens":100}}

data: [DONE]
```

**OpenRouter deviations** (source: <https://openrouter.ai/docs/api/reference/streaming>):

- Sends SSE comments such as `: OPENROUTER PROCESSING`; skip lines starting with `:`.
- The final usage chunk contains one choice with an empty delta that repeats `finish_reason` and `native_finish_reason` (OpenAI's spec uses an empty `choices`). Treat it as an accounting frame, not a second end.
- Mid-stream errors arrive as a chunk with a top-level `error` and `finish_reason: "error"`, for example `{"error":{"code":502,"message":"Provider disconnected unexpectedly"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}`. A `200` whose only event is such a chunk is a failure.
- Pre-stream errors are JSON `{"error": {"code": 400, "message": "..."}}` with 400, 401, 402, 429, 502, or 503.
- Closing the connection cancels generation for supported providers (not Groq or Google, per the list).

**OpenAI HTTP errors:** status codes and causes are listed at <https://developers.openai.com/api/docs/guides/error-codes> (for example 429 with codes `credit_balance_exhausted`, `slow_down`; 503 `server_is_overloaded`; follow `Retry-After`). The body shape `{"error": {"message", "type", "param", "code"}}` is UNVERIFIED for this session (long-standing behavior). Response header `x-request-id` identifies the request. Source: <https://developers.openai.com/api/reference/overview>.

### Reasoning Output

| Server | Field in `message` / `delta` | Notes | Source |
| --- | --- | --- | --- |
| OpenAI first-party | None | Only `usage.completion_tokens_details.reasoning_tokens`; summaries need the Responses API | <https://developers.openai.com/api/docs/guides/reasoning> |
| Ollama | `reasoning` | Also accepted on input assistant messages (mapped to the model's thinking) | <https://github.com/ollama/ollama/blob/main/openai/openai.go> |
| vLLM | `reasoning` | Renamed from `reasoning_content`; old clients "could silently read an empty `reasoning_content`" | <https://github.com/vllm-project/vllm/blob/main/docs/features/reasoning_outputs.md> |
| llama.cpp | `reasoning_content` | `--reasoning-format` `none` keeps tags in `content`; `deepseek` fills `reasoning_content` | <https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md> |
| LM Studio | `reasoning_content` | Documented since 0.3.9 for DeepSeek R1; current behavior UNVERIFIED | <https://lmstudio.ai/llms.txt> |
| Groq | `reasoning` | `reasoning_format`: `parsed` (separate field), `raw` (`<think>` tags in `content`), `hidden`; tools or JSON mode need `parsed` or `hidden` | <https://console.groq.com/docs/reasoning> |
| Together | `reasoning` or `reasoning_content` | Depends on the model; pass it back under the same key for multi-turn tool calls | <https://docs.together.ai/docs/openai-api-compatibility> |
| OpenRouter | `reasoning` (string) and `reasoning_details` (array) | `reasoning_content` is accepted as an alias on input | <https://openrouter.ai/docs/use-cases/reasoning-tokens> |

OpenRouter `reasoning_details` items are `{"type": "reasoning.summary" | "reasoning.encrypted" | "reasoning.text", "summary"?, "data"?, "text"?, "signature"?, "id", "format", "index"}`; in streams they arrive in `choices[].delta.reasoning_details`. For tool calling, send them back on the assistant message unchanged; "you cannot rearrange or modify the sequence of these blocks." Source: <https://openrouter.ai/docs/use-cases/reasoning-tokens>.

**Parser rule:** reasoning text = `delta.reasoning` or else `delta.reasoning_content`; also collect `delta.reasoning_details`. Optionally split `<think>...</think>` out of `content` for servers in raw mode.

**Chat UI:** show reasoning text collapsed; store `reasoning_details` (OpenRouter) and the raw reasoning string per assistant message so it can be replayed for the same provider. Replaying plain reasoning strings is optional for local servers.

## Gemini (`generateContent`)

### Status

"While it is now considered legacy, the original `generateContent` API remains fully supported." The Interactions API "is Generally Available and the recommended interface for all new projects" as of June 2026. Source: <https://aistudio.google.com/docs/interactions-overview>. `generateContent` is stateless and maps cleanly onto our stored history; build it first. See [Interactions API Notes](#interactions-api-notes) for the migration target.

### Endpoints, Auth, and Models

- Base: `https://generativelanguage.googleapis.com/v1beta`.
- Non-streaming: `POST /v1beta/models/{model}:generateContent`. Streaming: `POST /v1beta/models/{model}:streamGenerateContent?alt=sse`. The request body is identical for both. Source: <https://ai.google.dev/api/generate-content>, <https://aistudio.google.com/docs/text-generation?generate-content=true>.
- Auth header: `x-goog-api-key: <key>` (all REST examples). The list-models example uses the `?key=` query parameter instead. Prefer the header so keys stay out of logs. Source: <https://aistudio.google.com/docs/text-generation?generate-content=true>, <https://aistudio.google.com/api-reference/models/list-models>.
- No version header; the version is in the path (`v1beta`, with a stable `v1` reference also published). Source: <https://aistudio.google.com/llms.txt>.
- List models: `GET /v1beta/models?pageSize=50&pageToken=...` (default 50, max 1000) returns `{"models": [Model], "nextPageToken": "..."}`. Source: <https://aistudio.google.com/api-reference/models/list-models>.

```json
{
  "models": [
    {
      "name": "models/gemini-4-hydrogen",
      "baseModelId": "gemini-4-hydrogen",
      "version": "...",
      "displayName": "Gemini 4 Hydrogen",
      "description": "...",
      "inputTokenLimit": 1048576,
      "outputTokenLimit": 65536,
      "supportedGenerationMethods": ["generateContent", "countTokens"],
      "thinking": true
    }
  ],
  "nextPageToken": "..."
}
```

Field names are from <https://ai.google.dev/api/models>; values are illustrative. Filter on `supportedGenerationMethods` containing `generateContent` and strip the `models/` prefix for display.

Current models (snapshot): `gemini-4-carbon`, `gemini-4-argon`, `gemini-4-neon`, `gemini-4-hydrogen`, `gemini-3.8-flash`, `gemini-3.6-flash`, `gemini-3.5-flash-lite`. Source: <https://aistudio.google.com/docs/thinking?generate-content=true>.

### Request Shape

```json
{
  "systemInstruction": {"parts": [{"text": "You are a cat. Your name is Neko."}]},
  "contents": [
    {"role": "user", "parts": [{"text": "Hello"}]},
    {"role": "model", "parts": [{"text": "Great to meet you. What would you like to know?"}]},
    {"role": "user", "parts": [
      {"text": "Summarize this file and this image."},
      {"inlineData": {"mimeType": "application/pdf", "data": "<base64>"}},
      {"inlineData": {"mimeType": "image/jpeg", "data": "<base64>"}}
    ]}
  ],
  "generationConfig": {
    "maxOutputTokens": 8192,
    "stopSequences": ["Title"],
    "thinkingConfig": {"thinkingLevel": "high", "includeThoughts": true}
  }
}
```

- **Roles:** `user` and `model` only, alternating. Source: <https://ai.google.dev/api/generate-content> (`Content.role`: "Must be either 'user' or 'model'").
- **System prompt:** top-level `systemInstruction` (a `Content`, text only). REST examples write it `system_instruction`, and both `inline_data` and `inlineData` appear in official examples, so the endpoint accepts snake_case and camelCase. Use camelCase consistently. Source: <https://aistudio.google.com/api-reference/generate-content/generate-content>, <https://aistudio.google.com/docs/text-generation?generate-content=true>.
- **`Part` union:** `text`, `inlineData {mimeType, data}`, `fileData {fileUri, mimeType}`, `functionCall {id?, name, args}`, `functionResponse {id?, name, response, parts?}`, plus `thought` (bool) and `thoughtSignature` (string). Source: <https://ai.google.dev/api/generate-content>.
- **Sampling:** "Gemini 4 and Gemini 3.6+ models ignore custom values for `temperature`, `top_p`, and `top_k` and return deprecation warning headers (and will return `400` errors in a future release)." Do not send them. Source: <https://aistudio.google.com/docs/text-generation?generate-content=true>.
- **Output limit:** `maxOutputTokens` includes thought tokens; hitting it while thinking returns `MAX_TOKENS` with truncated or empty output. Gemini 4 shares a 1,048,576-token window, and a single request returns at most 65,536 tokens (32,768 for non-streaming REST on Carbon and Argon). Source: <https://aistudio.google.com/docs/thinking?generate-content=true>.
- **Images:** `image/png`, `image/jpeg`, `image/webp`, `image/heic`, `image/heif`, `image/gif`, `image/avif` in `inlineData`. Source: <https://ai.google.dev/api/generate-content>.
- **PDFs (native):** `inlineData` with `application/pdf`, or Files API `fileData`. Up to 1,000 PDF pages (2,000 Office/OpenDocument pages), 50 MB per file. Inline data is limited to 100 MB per request, 50 MB for PDFs. Source: <https://aistudio.google.com/docs/document-processing?generate-content=true>, <https://aistudio.google.com/docs/file-input-methods>.
- **Text files:** `inlineData` accepts `text/plain`, `text/markdown`, `text/csv`, `text/html`, `application/json`, code types and more; plain-text formats are "read as text only". Office documents and spreadsheets are also accepted. Source: <https://ai.google.dev/api/generate-content>, <https://aistudio.google.com/docs/document-processing?generate-content=true>.

### Tool Calling

```json
{
  "tools": [{
    "functionDeclarations": [{
      "name": "get_current_temperature",
      "description": "Gets the current temperature for a given location.",
      "parametersJsonSchema": {
        "type": "object",
        "properties": {"location": {"type": "string"}},
        "required": ["location"]
      }
    }]
  }],
  "toolConfig": {"functionCallingConfig": {"mode": "AUTO"}}
}
```

- `FunctionDeclaration`: `name` (a-z, A-Z, 0-9, underscores, colons, dots, dashes, max 128), `description` (required), and either `parameters` (OpenAPI-style `Schema`) or `parametersJsonSchema` (JSON Schema, mutually exclusive with `parameters`). Use `parametersJsonSchema` so MCP tool schemas pass through unchanged. Source: <https://ai.google.dev/api/generate-content>.
- `functionCallingConfig.mode`: `AUTO` (default), `ANY` (must call; `allowedFunctionNames` limits choices), `NONE`, `VALIDATED` (constrained decoding). Source: <https://aistudio.google.com/api-reference/generate-content/generate-content>.
- **The model asks** with `functionCall` parts in the `model` content. `functionCall.id` is optional; "If populated, the client [should] execute the `function_call` and return the response with the matching `id`." Source: <https://ai.google.dev/api/generate-content>.
- **Send results back** as `functionResponse` parts in a following `user` content: `{"functionResponse": {"name": "...", "id": "<same id if given>", "response": {"result": ...}}}`. Use an `"error"` key in `response` for failures. Results can include media via `parts`. Source: <https://ai.google.dev/api/generate-content>, <https://aistudio.google.com/docs/function-calling?generate-content=true>.
- **Parallel calls:** results need not be in call order, but must not be interleaved: send `FC1 + signature, FC2` in the model turn, then `FR1, FR2` in the next user turn; "FC1 + signature, FR1, FC2, FR2" returns 400. Source: <https://aistudio.google.com/docs/thought-signatures?generate-content=true>, <https://aistudio.google.com/docs/function-calling?generate-content=true>.

### Streaming

`streamGenerateContent?alt=sse` returns SSE `data:` lines, each a full `GenerateContentResponse` holding only the new parts, with a shared `responseId`. No `event:` names. Source: <https://aistudio.google.com/docs/text-generation?generate-content=true>. Whether a terminating `[DONE]` line is ever sent: none is shown (UNVERIFIED); end on stream close.

```text
data: {"candidates":[{"content":{"parts":[{"text":"The image displays"}],"role":"model"},"index":0}],"usageMetadata":{"promptTokenCount":12},"modelVersion":"gemini-4-hydrogen","responseId":"mAitaLmkHPPlz7IPvtfUqQ4"}

data: {"candidates":[{"content":{"parts":[{"text":" the following materials"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":40,"thoughtsTokenCount":120,"totalTokenCount":172},"modelVersion":"gemini-4-hydrogen","responseId":"mAitaLmkHPPlz7IPvtfUqQ4"}
```

The structure follows the documented streaming responses; values are **illustrative**.

- **Text deltas:** append `candidates[0].content.parts[].text` where `thought` is not true.
- **Thought summaries:** parts with `"thought": true` (only with `thinkingConfig.includeThoughts: true`). Streaming returns "rolling, incremental summaries". Source: <https://aistudio.google.com/docs/thinking?generate-content=true>.
- **Tool calls:** `functionCall` parts. The `FunctionCall` schema has no partial-argument field, so each call should arrive complete in one chunk (inferred from <https://ai.google.dev/api/generate-content>; verify in a test). Detect tool calls by the presence of `functionCall` parts, not by `finishReason`.
- **Signatures in streams:** for a response without a function call, the signature "may [be returned] in a part with an empty text content"; "parse the entire request until the `finish_reason` is returned". Source: <https://aistudio.google.com/docs/thought-signatures?generate-content=true>.
- **Usage:** `usageMetadata` with `promptTokenCount`, `cachedContentTokenCount`, `candidatesTokenCount`, `thoughtsTokenCount`, `toolUsePromptTokenCount`, `totalTokenCount`. Keep the last value seen. Source: <https://ai.google.dev/api/generate-content>.
- **Finish reasons** (`candidates[].finishReason`): `STOP`, `MAX_TOKENS`, `SAFETY`, `RECITATION`, `LANGUAGE`, `OTHER`, `BLOCKLIST`, `PROHIBITED_CONTENT`, `SPII`, `MALFORMED_FUNCTION_CALL`, `IMAGE_SAFETY`, `IMAGE_PROHIBITED_CONTENT`, `IMAGE_OTHER`, `NO_IMAGE`, `IMAGE_RECITATION`, `UNEXPECTED_TOOL_CALL`, `TOO_MANY_TOOL_CALLS`, `MISSING_THOUGHT_SIGNATURE`, `MALFORMED_RESPONSE`, `ESCALATION`, `PUP_LIMITED_DISABLED`, `CONTINUATION` (per-request output cap reached; continue with the returned `continuation_token`). `finishMessage` explains it. Source: <https://ai.google.dev/api/generate-content>.
- **Blocked prompt:** no candidates, and `promptFeedback.blockReason` is `SAFETY`, `OTHER`, `BLOCKLIST`, `PROHIBITED_CONTENT`, or `IMAGE_SAFETY`. Source: same.
- **Errors:** HTTP status plus `{"error": {"code": 400, "message": "...", "status": "INVALID_ARGUMENT", "details": [...]}}`. Common: 400 `INVALID_ARGUMENT` / `FAILED_PRECONDITION`, 402 and 429 `RESOURCE_EXHAUSTED` (402 is depleted prepay credit; do not retry), 403 `PERMISSION_DENIED`, 404 `NOT_FOUND`, 499 `CANCELLED`, 500 `INTERNAL`, 503 `UNAVAILABLE`, 504 `DEADLINE_EXCEEDED`. Source: <https://aistudio.google.com/docs/api-errors?generate-content=true>. The shape of an error that occurs mid-stream is UNVERIFIED.

### Thinking and Thought Signatures

- `thinkingConfig.thinkingLevel` is required for Gemini 4 and recommended for Gemini 3. Carbon and Argon: `low`, `medium` (default), `high`. Neon and Hydrogen: `low`, `medium`, `high` (default), `xhigh`. Thinking cannot be disabled on Gemini 4. `thinkingBudget` is deprecated on Gemini 4 (alone it maps to a level with a warning; with `thinkingLevel` it returns 400); Gemini 2.5 uses `thinkingBudget` (0 disables where allowed, -1 dynamic). Source: <https://aistudio.google.com/docs/thinking?generate-content=true>.
- **Signatures are mandatory in function-calling turns on Gemini 3 and 4.** The first `functionCall` part of each step in the current turn carries `thoughtSignature`; with parallel calls only the first part has it. It must be returned "in the exact part where it was received". Missing it returns 400 (for example "Function call `FC1` in the `1.` content block is missing a `thought_signature`"). Signatures on the final non-function part are recommended but not validated. Source: <https://aistudio.google.com/docs/thought-signatures?generate-content=true>.
- For function-call history that the API did not generate (another provider's history, or injected calls), put the dummy value `"skip_thought_signature_validator"` (or `"context_engineering_is_the_way_to_go"`) in `thoughtSignature`. Source: same page.

**Chat UI:** show `thought: true` text collapsed as "Thinking"; never show `thoughtSignature`. Store the model's `parts` verbatim (including thought parts and signatures) and replay them unchanged for the same model family. Whether thought-summary parts must be replayed is UNVERIFIED; replaying the full model content, as the SDKs do, is the documented safe path.

### Interactions API Notes

For the future `gemini_interactions` adapter (source: <https://aistudio.google.com/docs/streaming>, <https://aistudio.google.com/docs/interactions-overview>):

- `POST /v1beta/interactions` with `{"model", "input", "stream": true, "tools": [{"type": "function", "name", "description", "parameters"}], "store": false}`. State is stored by default (`store=true`); `store=false` gives stateless mode with full history sent each time.
- SSE events: `interaction.created`, `interaction.status_update`, `step.start` (step `type` is `model_output`, `thought`, `function_call`, or a server tool), `step.delta` (`text`, `image`, `audio`, `thought_summary`, `thought_signature`, `arguments_delta`), `step.stop`, `interaction.completed` (with `usage`), `error`, then `event: done` / `data: [DONE]`.
- Function calls stream their arguments as `arguments_delta` strings to accumulate; the interaction ends with status `requires_action`; results go back as `{"type": "function_result", "name", "call_id", "result": {...}}` input items.

## Cross-Provider Design Notes

### Normalized Stream Events

One internal enum keeps the HTTP layer and the UI provider-agnostic (suggested names):

| Event | Anthropic | OpenAI-compatible | Gemini |
| --- | --- | --- | --- |
| `TextDelta(text)` | `text_delta` | `delta.content` | `parts[].text` (not thought) |
| `ReasoningDelta(text)` | `thinking_delta` (non-empty) | `delta.reasoning` / `delta.reasoning_content` | `parts[].text` with `thought: true` |
| `ReplayState(blob)` | `thinking` / `redacted_thinking` block with `signature` | `delta.reasoning_details` (OpenRouter) | `thoughtSignature` on parts |
| `ToolCallStart{id, name}` | `content_block_start` `tool_use` | first `tool_calls[i]` fragment | whole `functionCall` part |
| `ToolCallArgsDelta{id, json}` | `input_json_delta` | `function.arguments` fragment | not used |
| `ToolCallEnd{id}` | `content_block_stop` | `finish_reason: "tool_calls"` | same chunk |
| `Usage{...}` | `message_start` + cumulative `message_delta.usage` | final `usage` chunk | last `usageMetadata` |
| `Finish(reason)` | `message_delta.delta.stop_reason` | `finish_reason` | `finishReason` |
| `Error{...}` | `event: error` | OpenRouter error chunk | HTTP error body |

Finish reason mapping:

| Normalized | Anthropic | OpenAI-compatible | Gemini |
| --- | --- | --- | --- |
| `stop` | `end_turn`, `stop_sequence` | `stop` | `STOP` (without `functionCall` parts) |
| `tool_calls` | `tool_use` | `tool_calls`, `function_call` | `STOP` with `functionCall` parts |
| `length` | `max_tokens`, `model_context_window_exceeded` | `length` | `MAX_TOKENS`, `CONTINUATION` |
| `refusal` | `refusal` | `content_filter` (or `delta.refusal` text) | `SAFETY`, `RECITATION`, `BLOCKLIST`, `PROHIBITED_CONTENT`, `SPII`, `IMAGE_*`, prompt `blockReason` |
| `error` | stream `error` event | OpenRouter `error` | `MALFORMED_FUNCTION_CALL`, `UNEXPECTED_TOOL_CALL`, `TOO_MANY_TOOL_CALLS`, `MISSING_THOUGHT_SIGNATURE`, `MALFORMED_RESPONSE` |
| `other` | `pause_turn` | anything else | `OTHER`, `LANGUAGE`, `ESCALATION`, `PUP_LIMITED_DISABLED` |

### Storage Implications

- **Store each assistant message twice:** provider-neutral parts (text, reasoning text, tool calls, tool results) for display and cross-provider replay, plus an opaque `provider_state` JSON (Anthropic content blocks verbatim, OpenRouter `reasoning_details`, Gemini `parts` with `thoughtSignature`) tagged with provider and model. Replay `provider_state` only to the same provider, and for Anthropic across Claude models (the API drops unreadable blocks). For Gemini, foreign function-call history needs the dummy signature.
- **Tool call ids:** Anthropic `toolu_...`, OpenAI `call_...`, Gemini optional. Synthesize an id when Gemini omits it, but do not send a synthesized id back to Gemini.
- **Snapshot the system prompt and tool list per chat.** Projects that carry a system prompt should copy it onto each chat at creation; editing the project should not rewrite running chats. This avoids Anthropic prefix-check 400s and keeps provider prompt caches warm. If a running chat must change instructions, append them (Anthropic `role: "system"` message where supported; a user-visible note elsewhere). Source: <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/prefix-checks>.
- **MCP tool lists:** connecting or disconnecting an MCP server mid-chat changes `tools`, which has the same prefix-check effect on Anthropic. Freeze the tool list per chat, or use the beta `tool_addition` / `tool_removal` system blocks. Source: <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/change-tools-or-instructions>.
- **Do not send assistant-last histories** to Anthropic (prefill 400 on Claude 4.6 and later). "Continue" after truncation means a new user message. Source: <https://platform.claude.com/docs/en/api/errors>.

### File Attachment Matrix

| Input | Anthropic | OpenAI Chat Completions | Gemini | Ollama / llama.cpp / LM Studio / vLLM |
| --- | --- | --- | --- | --- |
| Images | `image` block, base64 / url / file | `image_url` data URL or https | `inlineData` | Data URL (Ollama: base64 only); model must support vision |
| PDF | `document` block (native) | `file` part, PDF only | `inlineData` `application/pdf` (native) | Not documented: extract text in the backend or reject |
| Text and code files | `text` block or `document` text source | `text` part (file text inlined) | `inlineData` `text/*` or `text` part | `text` part |
| Office files | Convert first | Not in Chat Completions | `inlineData` (native) | Convert first |

Plain-text extraction from PDF or DOCX in Rust is parsing, not ML inference, so it fits the "no local ML" rule. Scanned PDFs without a text layer would need OCR, which is out of scope; send those only to providers with native PDF vision.

### SSE Parsing in Rust

- Formats to support: named events with `event:` + `data:` (Anthropic, Gemini Interactions, OpenAI Responses), data-only events (OpenAI Chat Completions, Gemini `alt=sse`), comment lines starting with `:` (OpenRouter keep-alives), the `data: [DONE]` sentinel (OpenAI-compatible, Gemini Interactions), and multi-line `data:` fields per the SSE spec. Source for the comment rule: <https://openrouter.ai/docs/api/reference/streaming>.
- Crates (crates.io API, checked 2026-11-26): `reqwest` 0.13.5 (updated 2026-09-08); `eventsource-stream` 0.2.3 (last release 2022-02-17); `sse-stream` 0.3.0 (updated 2026-09-18); `reqwest-eventsource` 0.6.0 (2024-03-29). Sources: <https://crates.io/api/v1/crates/reqwest>, <https://crates.io/api/v1/crates/eventsource-stream>, <https://crates.io/api/v1/crates/sse-stream>, <https://crates.io/api/v1/crates/reqwest-eventsource>. A small in-house parser over `reqwest::Response::bytes_stream()` is also reasonable (about 100 lines) and avoids the unmaintained-looking options; choose after checking `sse-stream`'s API and license.
- Always stream from the backend to providers, even for one-shot calls: Anthropic SDKs require streaming for large `max_tokens` to avoid HTTP timeouts, and streaming lets Copilot cancel by dropping the connection. Source: <https://platform.claude.com/docs/en/build-with-claude/streaming>.

## Open Questions

- OpenAI Chat Completions `file_data`: OpenAI's own examples use the data-URL prefix; whether raw base64 is also accepted is UNVERIFIED.
- Whether the newest OpenAI models still accept `role: "system"` in Chat Completions (UNVERIFIED).
- Gemini mid-stream error payload shape, and whether `alt=sse` ever sends a final sentinel (UNVERIFIED).
- vLLM default port and `--api-key` header handling; Groq and Together model-list endpoints (UNVERIFIED).
- LM Studio's current reasoning field name in Chat Completions (`reasoning_content` per its 0.3.9 notes; may have changed).

## Verification

An independent reviewer re-opened the primary sources on 2026-11-26 and checked the load-bearing claims. All eight held; the edits below tighten scope or resolve an open item.

| Claim | Result | Source re-read |
| --- | --- | --- |
| `anthropic-version` `2026-10-21` is current; `temperature` / `top_p` / `top_k` return 400 "on every model"; `document` blocks directly in `assistant` messages return 400; `2023-06-01` is the prior version | Confirmed (versioning page lists `2026-10-21` then `2023-06-01`; migration page also confirms the SDK versions) | <https://platform.claude.com/docs/en/api/versioning>, <https://platform.claude.com/docs/en/api/migration-2026-10-21> |
| Claude 5.x thinking on by default, `display` defaults to `omitted`; `enabled` + `budget_tokens` is 400 on 4.7 and later; `thinking` and `redacted_thinking` blocks go back unmodified within tool-use turns | Confirmed. Added: Claude Mythos Preview also defaults to `omitted` but still accepts `enabled` + `budget_tokens`, so "4.7 and later" means numbered models only | <https://platform.claude.com/docs/en/build-with-claude/thinking> |
| Prefix checks: a change to `system`, `tools`, or an earlier message invalidates later thinking; fix is the same `system` and `tools` for the whole conversation | Confirmed. Corrected Key Finding 3: the page lists Fable 5.2, Mythos 5.2, Fable 5.1, Mythos 5.1, Opus 5.5, Sonnet 5.5, Haiku 5.5, not every Fable or Mythos 5.x model | <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/prefix-checks>, <https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/change-tools-or-instructions> |
| Claude 4.6 and later reject a prefilled final assistant message with 400 `invalid_request_error` | Confirmed, with the exact message. Added: Claude Mythos Preview rejects it too | <https://platform.claude.com/docs/en/api/errors> |
| GPT-6 Astra and GPT-6.1 Sol require Responses for tool calling; Chat Completions remains supported, Responses recommended | Confirmed verbatim on the function-calling and migration pages; the reasoning page adds that GPT-6.2 Astra and Sol reject `reasoning_effort: "none"` | <https://developers.openai.com/api/docs/guides/function-calling>, <https://developers.openai.com/api/docs/guides/migrate-to-responses>, <https://developers.openai.com/api/docs/guides/reasoning> |
| Chat Completions accepts only PDF as `file` parts (`file_data` or `file_id`, no `file_url`); other file text goes in `text` parts | Confirmed verbatim. Resolved part of an open question: OpenAI's own Chat Completions examples send `file_data` as a `data:application/pdf;base64,` URL | <https://developers.openai.com/api/docs/guides/file-inputs> |
| `generateContent` is legacy but fully supported; Interactions API GA and recommended as of June 2026 | Confirmed verbatim | <https://aistudio.google.com/docs/interactions-overview> |
| Gemini 3 and 4 require thought signatures on function-call parts in the current turn, or the request fails with 400 | Confirmed (first `functionCall` part of each step; dummy signatures skip validation for foreign history) | <https://aistudio.google.com/docs/thought-signatures?generate-content=true> |

Everything else in this note was not re-checked by the reviewer and keeps its original sourcing and UNVERIFIED marks.
