# fake-llm

A scriptable fake model server for trying Copilot by hand and for the smoke test (`scripts/smoke.sh`). It speaks both wire formats Copilot's adapters use and serves a tiny MCP server, all on one port. Words in the last user message decide what it answers, so every path of the chat view (streaming, reasoning, tool calls with approvals, refusals, provider errors, broken streams, spoken replies) can be reached without a real key.

Dev only: no Copilot crate depends on it (`make boundaries`), the release build never compiles it, and the image never contains it.

## Run It

```bash
make fake-llm       # http://127.0.0.1:4010, key "fake-key"
```

`GET /health` answers `ok`. Each answer logs one line (`fake-llm.openai.answered`, `fake-llm.anthropic.failed`, `fake-llm.mcp.called`, ...).

## Connect Copilot

In Settings, Providers, add one connection per format (or both):

| Kind | Preset | Base URL | API key |
| --- | --- | --- | --- |
| OpenAI-compatible | Custom (or OpenAI, to have Copilot ask for streamed usage) | `http://127.0.0.1:4010/v1` | `fake-key` |
| Anthropic | Custom | `http://127.0.0.1:4010` | `fake-key` |

The server is on loopback, so the connection must be allowed to reach private addresses: with the default `ALLOW_PRIVATE_NETWORK=admins` that works for connections the administrator (the first account) owns; set `ALLOW_PRIVATE_NETWORK=all` to let every account use it. Saving the connection lists three models, enabled automatically:

| Model | OpenAI-compatible reports | Anthropic reports |
| --- | --- | --- |
| `fake-chat` | images, PDFs, tools | images, PDFs |
| `fake-reasoner` | images, PDFs, tools, reasoning | images, PDFs, adaptive thinking with low, medium, and high effort |
| `fake-audio` | text and audio in, text and audio out, tools | images, PDFs (Claude models have no audio, and Copilot locks it) |

Every model reports a 128,000-token context window and 8,192 output tokens. For tools, add a tool server in Settings, Tool servers, with the URL `http://127.0.0.1:4010/mcp`. Test lists `echo` (answers with its `text`) and `add` (answers with the sum of `a` and `b`).

## Keywords

Words match whole and in any case; punctuation around them is ignored. Only the newest message counts: Copilot leaves a refused answer (and a failed one without text) out of the next request, so its prompt merges into the next user message, and the fake reads the last text of the last user message and the attachments after it. Text Copilot writes for a file (an inlined text file, `<file name="...">`, or a notice such as `[audio "clip.wav" omitted: ...]` or `[file "a.txt" was deleted]`) counts as an attachment, not as the message.

| Message | Answer |
| --- | --- |
| anything else | `You said: <message>` in a few chunks, then usage (`(with N attachments)` is added when the message has images, files, or audio). OpenAI-compatible streams send usage only when the request asks for it (`stream_options.include_usage`, which Copilot sends for the OpenAI and Ollama presets), as the real API does |
| `failed <status>` (for example `failed 403`, `failed 429`, `failed 500`) | That HTTP status (400 to 599) with the format's error body; `429` adds `Retry-After: 2` |
| `midstream error` | A little text, then an error chunk (OpenAI-compatible) or `error` event (Anthropic) instead of the end |
| `slow` | The reply streams with `FAKE_LLM_SLOW_MS` between chunks |
| `long` | A long Markdown reply (a heading, 24 paragraphs, a list, a code block) |
| `think` | Reasoning first: `reasoning_content` deltas, or a `thinking` block with a signature |
| `tool <name> <json>` | A call to that tool with exactly those arguments (`{}` when none; text that is not JSON is sent as it is). The name matches an offered tool by name or by Copilot's alias (`<slug>__<name>`) |
| `refuse` | A little text, then a refusal: finish reason `content_filter`, or stop reason `refusal` with `stop_details` |
| `audio` (on `fake-audio`, OpenAI-compatible) | A one-second 440 Hz tone as 24 kHz PCM16 `audio` deltas with the transcript "Here is a short tone."; without Speak replies (no `audio` in `modalities`) a text reply says to turn it on |

When the request ends with tool results (the turn after an approved call), the answer is `Tool said: <result>` whatever the message was. Otherwise `failed` wins over every other keyword; then a `tool` command, `refuse`, `midstream error`, `audio`, and `long`, in that order. `think` and `slow` combine with any streamed reply, also when they come before `tool` (`think tool add {"a": 1, "b": 2}`). `tool` only counts when the name is the last word or a `{` follows it, so "which tool do you use" is plain text.

`fake-reasoner` also reasons without `think` whenever the request turns reasoning on: on the OpenAI-compatible format unless `reasoning_effort` is `none`, on the Anthropic format when `thinking.type` is `adaptive` or `enabled`.

## Checks

The fake refuses what the real APIs refuse, so a client bug shows up here rather than only against a paid key:

- A missing or wrong key: `401` (`Authorization: Bearer` for the OpenAI-compatible format, `x-api-key` for Anthropic's).
- A model it does not list: `404`.
- OpenAI-compatible: an assistant message with `tool_calls` that is not followed by `tool` messages answering each call (the message lists the ids that got no answer), and a `tool` message that answers no call of the assistant message before its run of `tool` messages: `400`.
- Anthropic: a missing `anthropic-version` header or `max_tokens`, a conversation that ends with an assistant message (prefill), a `tool_use` whose `tool_result` is not at the start of the next message, a `tool_result` that answers no `tool_use` of the message before it, and a replayed `thinking` block whose signature is not the one this server gave it: `400`.

Both formats answer streamed (`"stream": true`) and whole requests; a whole request with `midstream error` gets a `500`.

## Settings

| Variable | Default | Description |
| --- | --- | --- |
| `FAKE_LLM_HOST` | `127.0.0.1` | Bind address |
| `FAKE_LLM_PORT` | `4010` | Port |
| `FAKE_LLM_KEY` | `fake-key` | The one API key both formats accept |
| `FAKE_LLM_CHUNK_MS` | `30` | Pause between streamed chunks |
| `FAKE_LLM_SLOW_MS` | `500` | Pause between chunks of a `slow` reply |
| `FAKE_LLM_MCP_TOKEN` | none | When set, `/mcp` answers `401` without `Authorization: Bearer <token>` (to try a tool server's bearer token) |

## From the Command Line

```bash
curl -N http://127.0.0.1:4010/v1/chat/completions \
  -H 'authorization: Bearer fake-key' -H 'content-type: application/json' \
  -d '{"model":"fake-chat","stream":true,"messages":[{"role":"user","content":"think hello"}]}'

curl -N http://127.0.0.1:4010/v1/messages \
  -H 'x-api-key: fake-key' -H 'anthropic-version: 2026-10-21' -H 'content-type: application/json' \
  -d '{"model":"fake-chat","max_tokens":256,"stream":true,"messages":[{"role":"user","content":"refuse"}]}'
```
