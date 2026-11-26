# Voice Research

Status: research note, checked against live primary sources on 2026-11-26. Treat it as a draft: provider APIs in this area change monthly, so re-check the linked pages before building against them. Claims I could not confirm from a primary source are marked **UNVERIFIED**.

Scope: what Copilot can do with voice while honoring hard constraint 1 in `docs/design/copilot.md` (no local speech-to-text, text-to-speech, or inference; forward to providers only).

## Summary

| Provider | Audio in (inside a chat request) | Audio out (model speaks) | Standalone speech endpoints | Realtime / live voice |
| --- | --- | --- | --- | --- |
| Anthropic (Claude API) | **No** | **No** | **No** | **No** |
| OpenAI | Yes: Chat Completions `input_audio` (wav, mp3) on `gpt-audio-1.5` | Yes: `modalities: ["text","audio"]`, streamed as `delta.audio` chunks | `/v1/audio/speech` (models shut down 2027-01-06), `/v1/audio/transcriptions` (`gpt-transcribe`) | Realtime API (`gpt-realtime-2.1`, `-mini`) and GPT-Live (`gpt-live-1.1`), WebRTC or WebSocket, ephemeral client secrets |
| Google Gemini | Yes: `inlineData` with `audio/*` on Gemini 4 models | Only on dedicated TTS models and Live models | TTS models `gemini-3.8-flash-tts`, `gemini-3.8-flash-lite-tts`; STT `gemini-3.5-transcribe` | Live API (`gemini-3.8-live`), WebSocket, ephemeral tokens (preview) |
| Ollama | No `input_audio` in chat; WAV upload to `/v1/audio/transcriptions` | No | `/v1/audio/speech`, `/v1/audio/transcriptions`, `/v1/audio/translations` | No |
| vLLM | Yes (`input_audio` for audio models) | No (TTS lives in vLLM-Omni) | `/v1/audio/transcriptions`; vLLM-Omni adds `/v1/audio/speech` | WebSocket `/v1/realtime` transcription only |
| llama.cpp server | Yes (`input_audio`, mp3/wav/flac) | No in chat | `/v1/audio/speech`, `/tts` | WebSocket `/v1/realtime` (transcription and conversation) |
| LM Studio | Not documented | No | None listed | No |
| OpenRouter | Yes (`input_audio`) | Yes (Chat Completions, streaming required) | `/api/v1/audio/speech`, `/api/v1/audio/transcriptions` | No (not found in docs) |

Bottom line for the owner: **Anthropic's API cannot speak and cannot listen.** Claude's voice mode is a feature of Anthropic's own apps, not of the API. Under the stated rule ("if not supported, do not add a translation layer"), Copilot shows no microphone or speaker for Claude models by default. Voice works natively with OpenAI, Gemini, OpenRouter audio models, and self-hosted servers that expose audio endpoints.

## 1. Anthropic

### Messages API: No Audio Content Block

- The `POST /v1/messages` reference lists content block params `TextBlockParam`, `ImageBlockParam`, `DocumentBlockParam`, `SearchResultBlockParam`, tool and thinking blocks, and server-tool result blocks. There is no audio block. The only `media_type` values are `image/jpeg`, `image/png`, `image/gif`, `image/webp`, `application/pdf`, and `text/plain`. ([Create a Message](https://platform.claude.com/docs/en/api/messages/create))
- The models overview states: "All current models support text and image input, text output, multilingual capabilities, vision, and tool use." Current models: `claude-fable-5-2`, `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5` (plus limited `claude-mythos-5-2`). ([Models overview](https://platform.claude.com/docs/en/about-claude/models/overview))
- The Models API `capabilities` object reports `image_input` and `pdf_input`, and has no audio field. ([List Models](https://platform.claude.com/docs/en/api/models/list))
- The OpenAI SDK compatibility layer says: "Audio and file inputs are not supported. A request that includes an `input_audio` or `file` content part returns a 400 error." The `audio` and `modalities` request fields are "Ignored" and `choices[].message.audio` is "Always empty". ([OpenAI SDK compatibility](https://platform.claude.com/docs/en/cli-sdks-libraries/libraries/openai-sdk))

### No Speech Output, No Realtime Endpoint

- The Claude Platform release notes (read in full, entries from 2024 through 2026-11-23) contain no audio input, audio output, speech, transcription, or realtime feature. The only audio mention (2026-09-01) is that image, video, and audio **files** Claude produces through the code execution tool carry C2PA Content Credentials. That is file provenance, not text-to-speech. ([Release notes](https://platform.claude.com/docs/en/release-notes/overview), [Files API](https://platform.claude.com/docs/en/build-with-claude/files))
- The docs index (`llms.txt`, about 970 links) has no page about voice, audio, speech, or realtime sessions. ([platform.claude.com/llms.txt](https://platform.claude.com/llms.txt))

### Claude App Voice Mode Is a Product Feature

- Voice mode is a beta feature of Claude Mobile, Claude Desktop, and claude.ai, with hands-free and push-to-talk modes and a fixed set of voices. "Your speech is converted to text, the recording is deleted, and the transcript is saved in your chat history." Claude Fable is not available in voice mode. In Claude Cowork and Claude Code, only dictation is available. ([Use voice mode](https://support.claude.com/en/articles/11101966-use-voice-mode))
- So voice mode is a speech-to-text, then Claude, then text-to-speech pipeline inside Anthropic's apps. No API reference documents it.
- Anthropic's Transparency Hub lists "voice dictation" as an input and, for older models only (Claude Sonnet 4, 4.5, 4.6 and 5, Haiku 4.5, Opus 4.5 to 4.8, Fable 5), "audio via text-to-speech" as an output. The current model cards (Opus 5.5, Sonnet 5.5, Haiku 5.5, Opus 5, Fable 5.1, Mythos 5.1) list text, artifacts, and diagrams only. I read this as describing Anthropic's product surfaces (voice mode), because no Messages API parameter produces audio. ([Transparency Hub](https://www.anthropic.com/transparency)) The interpretation is mine; the API facts above are what Copilot depends on.
- Press reports that voice mode uses ElevenLabs for speech output: **UNVERIFIED** (Anthropic has not confirmed it in any page I found).

### What to Tell the Owner About Anthropic

1. Anthropic's API does not accept audio and does not return audio, and there is no realtime voice endpoint, as of 2026-11-26.
2. The Claude app's voice mode is not exposed to developers. It transcribes speech, sends text to Claude, and speaks the reply with a text-to-speech engine Anthropic runs. Copilot cannot call it.
3. Following your rule, Copilot will not show voice controls for Claude models. The capability flags make this a data change later: if Anthropic adds audio, it shows up in the release notes and likely in the Models API `capabilities` object, and Copilot needs only a new adapter branch.
4. The open "something else" choice is yours (see Section 6, Option C): reading Claude's text aloud with another provider's cloud TTS or with the device's own speech engine. Neither runs a model inside Copilot, but both are outside "Anthropic natively", so they stay off unless you opt in.

## 2. OpenAI

### Current Audio Models

From the model catalog ([All models](https://developers.openai.com/api/docs/models/all)) and per-model endpoint tables:

| Model | Use | Endpoint |
| --- | --- | --- |
| `gpt-audio-1.5` | Audio in and out in chat | `/v1/chat/completions` only (not Responses, not Realtime) ([model page](https://developers.openai.com/api/docs/models/gpt-audio-1.5)) |
| `gpt-realtime-2.1`, `gpt-realtime-2.1-mini` | Speech-to-speech with reasoning and tools | `/v1/realtime` ([model page](https://developers.openai.com/api/docs/models/gpt-realtime-2.1)) |
| `gpt-live-1.1` | Full-duplex voice with delegation to a backend | `/v1/live/sessions` ([model page](https://developers.openai.com/api/docs/models/gpt-live-1.1)) |
| `gpt-transcribe` | File transcription | `/v1/audio/transcriptions`, realtime transcription sessions ([model page](https://developers.openai.com/api/docs/models/gpt-transcribe)) |
| `gpt-live-transcribe` | Live captions | realtime transcription sessions ([model page](https://developers.openai.com/api/docs/models/gpt-live-transcribe)) |

Deprecations that matter ([Deprecations](https://developers.openai.com/api/docs/deprecations)):

- `tts-1`, `tts-1-hd`, `gpt-4o-mini-tts` (all snapshots): shut down **2027-01-06**, replacement `gpt-realtime-2.1-mini`.
- `gpt-realtime`, `gpt-audio`, `gpt-audio-mini`, `gpt-realtime-mini`, `gpt-4o-audio`, `gpt-4o-realtime` and minis: shut down **2027-01-20**, replacements `gpt-audio-1.5` or `gpt-realtime-2.1(-mini)`.
- `whisper-1`, `gpt-4o-transcribe`, `gpt-4o-mini-transcribe`, `gpt-4o-transcribe-diarize`: shut down **2027-02-26**, replacements `gpt-transcribe` or `gpt-live-transcribe`. `/v1/audio/translations` is deprecated in favor of `/v1/realtime/translations`.

### Chat Completions Audio

([Audio in Chat Completions](https://developers.openai.com/api/docs/guides/audio-chat-completions), [Create chat completion](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create), [Streaming events](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events))

- Input: a user content part `{"type": "input_audio", "input_audio": {"data": "<base64>", "format": "wav" | "mp3"}}`. Only wav and mp3 are accepted.
- Output: `modalities: ["text", "audio"]` plus `audio: {voice, format}`. Formats: `wav`, `aac`, `mp3`, `flac`, `opus`, `pcm16`. Built-in voices: `alloy`, `ash`, `ballad`, `coral`, `echo`, `fable`, `nova`, `onyx`, `sage`, `shimmer`, `marin`, `cedar`, or a custom voice `{ "id": "voice_..." }`. The reference is internally inconsistent: the prose lists the voices above, while the enum lists `verse` and omits `fable`, `nova`, and `onyx`; the format prose omits `aac` while the enum includes it. Test a voice before making it a default.
- Response: `choices[0].message.audio = {id, data (base64), expires_at, transcript}`.
- Streaming: with `stream: true`, `delta.audio` chunks carry `id`, base64 `data`, or `transcript` text; the final audio update carries only `expires_at`.
- Multi-turn: a previous assistant audio reply is referenced by `audio: {id}` on the assistant message.
- The guide says the Responses API currently covers text and image input with text output, so audio chat must use Chat Completions.

### Standalone Speech Endpoints

- `POST /v1/audio/speech`: `input` up to 4096 chars, `model` one of `tts-1`, `tts-1-hd`, `gpt-4o-mini-tts`, `gpt-4o-mini-tts-2025-12-15`; `response_format` `mp3` (default), `opus`, `aac`, `flac`, `wav`, `pcm`; `stream_format` `audio` or `sse`; `instructions` for style. ([Create speech](https://developers.openai.com/api/reference/resources/audio/subresources/speech/methods/create))
- Every model this endpoint accepts shuts down on 2027-01-06, and the TTS guide says the replacement "`gpt-realtime-2.1-mini` isn't a supported model for `/v1/audio/speech`". After that date, OpenAI read-aloud needs the Realtime API. ([Text to speech](https://developers.openai.com/api/docs/guides/text-to-speech))
- Realtime can recite text: send an out-of-band response with an empty `input` and instructions such as "Say exactly the following: ...". ([Realtime conversations](https://developers.openai.com/api/docs/guides/realtime-conversations))
- `POST /v1/audio/transcriptions`: multipart `file` in flac, mp3, mp4, mpeg, mpga, m4a, ogg, wav, or webm; models include `gpt-transcribe` (recommended), `whisper-1` and `gpt-4o-*-transcribe` (deprecated); can stream transcript events. ([Create transcription](https://developers.openai.com/api/reference/resources/audio/subresources/transcriptions/methods/create), [Transcription](https://developers.openai.com/api/docs/guides/transcription))

### Realtime API and GPT-Live

- Realtime flow: the app server creates an ephemeral client secret, the client connects over WebRTC (browser) or WebSocket (server). GA changes: no `OpenAI-Beta: realtime=v1` header, `POST /v1/realtime/client_secrets` for ephemeral keys, `/v1/realtime/calls` for WebRTC, event names such as `response.output_audio.delta`. ([Realtime guide](https://developers.openai.com/api/docs/guides/realtime))
- Two browser connection modes: the "unified interface" (your server forwards the browser's SDP offer to `/v1/realtime/calls` with the real key) or an ephemeral key used by the browser directly. ([WebRTC](https://developers.openai.com/api/docs/guides/voice-webrtc))
- Client secrets look like `ek_...`; `expires_after.seconds` defaults to 600 (range 10 to 7200); a secret can carry the session config (instructions, voice, audio formats), but the reference says that config "can also be overridden by the client connection", so it is a default, not a lock. Expiry stops new sessions only; "the session itself may continue". Audio formats: `audio/pcm`, `audio/pcmu`, `audio/pcma`. `output_modalities` defaults to `["audio"]`. ([Create client secret](https://developers.openai.com/api/reference/resources/realtime/subresources/client_secrets/methods/create))
- WebSocket URL: `wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1`; live transcription uses `wss://api.openai.com/v1/realtime?intent=transcription` with `gpt-live-transcribe`. ([WebSockets](https://developers.openai.com/api/docs/guides/voice-websockets), [Realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription))
- GPT-Live (`gpt-live-1.1`) is OpenAI's recommended start for new voice apps: full duplex, sessions at `POST /v1/live/sessions` (WebRTC) or `wss://api.openai.com/v1/live/sessions`, billed by session duration, and it delegates reasoning to a backend. With "client delegation" the backend can be "any model, agent harness, or service your application runs". ([GPT-Live](https://developers.openai.com/api/docs/guides/live), [Audio and voice](https://developers.openai.com/api/docs/guides/audio))
- Note: GPT-Live client delegation could put an OpenAI voice in front of a Claude backend. That is exactly a cloud translation layer around a non-audio model, so it falls under the owner's "do not add" rule unless the owner chooses it.

## 3. Google Gemini

API surface note: the Gemini docs now lead with the **Interactions API** (`POST https://generativelanguage.googleapis.com/v1beta/interactions`, audio content blocks `{"type": "audio", "data", "mime_type"}`). `generateContent` and `streamGenerateContent` remain supported (the 2026-11-05 and 2026-10-26 changelog entries still change them). ([Audio understanding](https://ai.google.dev/gemini-api/docs/audio), [Changelog](https://ai.google.dev/gemini-api/docs/changelog))

### Audio Input

- `generateContent`: a `Part` with `inlineData: {mimeType, data}`; the reference lists audio types as `audio/*`. ([generateContent reference](https://ai.google.dev/api/generate-content))
- Supported audio MIME types: `audio/wav`, `audio/mp3`, `audio/aiff`, `audio/aac`, `audio/ogg`, `audio/flac`, `audio/mpeg`, `audio/m4a`, `audio/l16`, `audio/opus`, `audio/alaw`, `audio/mulaw`, `audio/webm`. Inline when the whole request is under 20 MB, Files API above that. 32 tokens per second of audio, up to 9.5 hours per prompt, downsampled to 16 Kbps and mixed to mono. ([Audio understanding](https://ai.google.dev/gemini-api/docs/audio))
- Current general models `gemini-4-carbon`, `gemini-4-neon`, `gemini-4-hydrogen` take text, image, audio, video, and PDF, and output **text only**. ([Models](https://ai.google.dev/gemini-api/docs/models), [Gemini 4 Hydrogen](https://ai.google.dev/gemini-api/docs/models/gemini-4-hydrogen))
- Gemini's OpenAI-compatible endpoint accepts `input_audio` parts (example uses `gemini-4-hydrogen` with `format: "wav"`). ([OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai))

### Text-to-Speech Models

- `gemini-3.8-flash-tts` and `gemini-3.8-flash-lite-tts` went GA on 2026-09-22 with a `/v1beta/voices` endpoint (150+ prebuilt and custom voices, voice design). TTS models take text only and return audio only. ([Changelog](https://ai.google.dev/gemini-api/docs/changelog), [Text-to-speech](https://ai.google.dev/gemini-api/docs/speech-generation))
- Interactions API: `generation_config.speech_config: [{"voice": "Kore"}]`, `response_format: {"type": "audio"}`. `generateContent`: `generationConfig.responseModalities: ["AUDIO"]` and `generationConfig.speechConfig.voiceConfig.prebuiltVoiceConfig.voiceName`, with `audio.mimeType` options `AUDIO_MP3`, `AUDIO_OGG_OPUS`, `AUDIO_L16`, `AUDIO_WAV`, `AUDIO_ALAW`, `AUDIO_MULAW`. ([Text-to-speech](https://ai.google.dev/gemini-api/docs/speech-generation), [generateContent reference](https://ai.google.dev/api/generate-content))
- Unary responses are WAV (24 kHz, mono, 16-bit). Streaming (`stream: true`) returns headerless `audio/l16` chunks at 24 kHz, so playback can start early. `sample_rate` can be 24000, 16000, or 8000. ([Text-to-speech](https://ai.google.dev/gemini-api/docs/speech-generation))
- Gemini 3.8 Flash TTS input limit is 8,192 tokens. ([Gemini 3.8 Flash TTS](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash-tts))

### Live API

- Models: `gemini-3.8-live` and `gemini-3.8-live-extended-thinking` (GA 2026-09-15); inputs text, images, audio; outputs text and audio. ([Gemini 3.8 Live](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-live), [Changelog](https://ai.google.dev/gemini-api/docs/changelog))
- Transport: stateful WebSocket at `/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent`. Input audio raw 16-bit PCM 16 kHz little-endian (the API resamples other rates when the MIME type states the rate, for example `audio/pcm;rate=48000`); output always raw 16-bit PCM 24 kHz. ([Live API overview](https://ai.google.dev/gemini-api/docs/live), [Live WebSockets reference](https://ai.google.dev/api/live))
- Ephemeral tokens (preview, Live API only): `POST /v1beta/auth_tokens`; by default 1 minute to start a session and 30 minutes of messaging; the client connects to `BidiGenerateContentConstrained` with `access_token`; tokens can be locked to a config so system instructions stay server-side. ([Ephemeral tokens](https://ai.google.dev/gemini-api/docs/ephemeral-tokens), [Live WebSockets reference](https://ai.google.dev/api/live), [WebSocket tutorial](https://ai.google.dev/gemini-api/docs/live-api/get-started-websocket) for the `BidiGenerateContentConstrained?access_token=` URL)
- Limits: audio-only sessions last 15 minutes unless session resumption is used; context window 128k tokens for native audio models. ([Live guide](https://ai.google.dev/gemini-api/docs/live-guide))
- Transcription: `gemini-3.5-transcribe` (files) and `gemini-3.5-transcribe-live` (WebSocket). ([Models](https://ai.google.dev/gemini-api/docs/models))

## 4. Ollama and Other OpenAI-Compatible Servers

These run outside Copilot, so forwarding to them is allowed.

| Server | Audio in chat | Speech endpoint | Transcription endpoint | Realtime | Source |
| --- | --- | --- | --- | --- | --- |
| Ollama | Not listed for `/v1/chat/completions` (text and image only) | `/v1/audio/speech` (added v0.40.8 per docs): `wav` (default) or `pcm`, 16-bit mono 24 kHz, streams while generating, `speed` 1 only | `/v1/audio/transcriptions` (WAV only, streaming deltas); `/v1/audio/translations` (v0.40.12) | No | [OpenAI compatibility](https://github.com/ollama/ollama/blob/main/docs/api/openai-compatibility.mdx), [Speech](https://docs.ollama.com/capabilities/speech) |
| vLLM | Yes, `input_audio` for audio models | Not in core vLLM | `/v1/audio/transcriptions`, `/v1/audio/translations` (ASR models) | `ws://host/v1/realtime` transcription | [Server](https://github.com/vllm-project/vllm/blob/main/docs/serving/online_serving/openai_compatible_server.md), [Speech to text](https://github.com/vllm-project/vllm/blob/main/docs/serving/online_serving/speech_to_text.md), [Multimodal inputs](https://github.com/vllm-project/vllm/blob/main/docs/features/multimodal_inputs.md) |
| vLLM-Omni | Omni models | `/v1/audio/speech` (Qwen3-TTS, Voxtral TTS, CosyVoice3, others) | n/a | Full-duplex realtime serving | [Speech API](https://github.com/vllm-project/vllm-omni/blob/main/docs/serving/speech_api.md), [README](https://github.com/vllm-project/vllm-omni/blob/main/README.md) |
| llama.cpp server | Yes, `input_audio` (mp3, wav, flac; `format` ignored) | `/v1/audio/speech` (`wav` or `pcm`, model sample rate, `stream_format: audio`) and `/tts` | Via `/v1/realtime?intent=transcription` | WebSocket `/v1/realtime` (transcription and conversation, PCM16 mono) | [Server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) |
| LM Studio | Not documented (chat params list has no audio) | None listed | None listed | No | [OpenAI compat](https://lmstudio.ai/docs/developer/openai-compat), [Chat completions](https://lmstudio.ai/docs/developer/openai-compat/chat-completions) |
| LocalAI | Model dependent | Text to audio | Audio to text | Realtime API for speech-to-speech | [README](https://github.com/mudler/LocalAI/blob/master/README.md) |
| OpenRouter | Yes, `input_audio` on Responses (wav, mp3) and Chat Completions (provider formats) | `/api/v1/audio/speech` (`mp3` or `pcm`) | `/api/v1/audio/transcriptions` | Not found | [Audio](https://openrouter.ai/docs/guides/overview/multimodal/audio) |

Useful detail: OpenRouter's models API returns `architecture.input_modalities` and `architecture.output_modalities` per model, and filters with `?output_modalities=...`, so Copilot can auto-fill audio flags for OpenRouter. Audio output from OpenRouter chat models requires `stream: true`. ([Models](https://openrouter.ai/docs/guides/overview/models), [Audio](https://openrouter.ai/docs/guides/overview/multimodal/audio))

Third-party posts describe Ollama audio-capable chat models (for example LFM3-Audio, gemma4 audio on MLX): **UNVERIFIED** against Ollama's own docs, which list no `input_audio` support in chat.

## 5. Zero-Weight Client-Side Options

These ship no model inside Copilot. They use the browser or OS engine, which may itself run an on-device model or send audio to a vendor cloud. Whether that counts as "running a model locally" is a judgment for the owner; it does not add weight to Copilot.

### Web Speech API Support

Data from MDN browser-compat-data ([SpeechSynthesis.json](https://github.com/mdn/browser-compat-data/blob/main/api/SpeechSynthesis.json), [SpeechRecognition.json](https://github.com/mdn/browser-compat-data/blob/main/api/SpeechRecognition.json)) unless noted.

| Engine | `speechSynthesis` (TTS) | `SpeechRecognition` (STT) | Where audio goes |
| --- | --- | --- | --- |
| Chromium (Chrome 139+) | Yes (since 33) | Yes, unprefixed since 139 (`webkitSpeechRecognition` since 33); `processLocally`, `available()`, `install()` since 139 | Recognition defaults to a server-based service unless `processLocally = true` and a language pack is installed ([MDN guide](https://developer.mozilla.org/en-US/docs/Web/API/Web_Speech_API/Using_the_Web_Speech_API)). TTS uses OS voices plus possibly remote voices (`SpeechSynthesisVoice.localService === false`) |
| Firefox desktop 160+ | Yes (since 49) | Yes since 160, **on-device only**, 40 languages; no cloud recognition, not on Android | On device ([Firefox 160 notes](https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Releases/160)) |
| WebKitGTK (Tauri on Linux) | Yes: Flite-based synthesis since WebKitGTK 2.39.6 (2.40 series), `ENABLE_SPEECH_SYNTHESIS` on by default; libspiel backend is an off-by-default option | No: `OptionsGTK.cmake` has no speech recognition option (WebKit's recognizer is Apple-only). **UNVERIFIED** at runtime | On device (Flite) ([WebKitGTK NEWS](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/gtk/NEWS), [OptionsGTK.cmake](https://github.com/WebKit/WebKit/blob/main/Source/cmake/OptionsGTK.cmake)) |
| Android System WebView (Tauri on Android) | **No** (BCD `version_added: false`, crbug 40417848) | BCD lists a prefixed interface, but a Chromium bug says WebView does not expose the speech APIs because they are unsupported; treat as unavailable (**UNVERIFIED** on current WebView) | n/a ([Chromium issue](https://issues.chromium.org/41172064)) |

Distro packages can change WebKitGTK build flags, so whether a given Linux install has `speechSynthesis` is **UNVERIFIED** per distro.

### Tauri v2 Plugins

- `tauri-plugin-tts` 0.2.1 (2026-09-08): delegates to the OS synthesizer: WinRT, AVSpeechSynthesizer, speech-dispatcher on Linux (needs the daemon installed), Android `TextToSpeech`. No bundled model. ([README](https://github.com/brenogonzaga/tauri-plugin-tts), [crates.io](https://crates.io/crates/tauri-plugin-tts)) On Android the default engine is usually Google's; whether a voice is network-backed depends on the engine: **UNVERIFIED**.
- `tauri-plugin-stt` 0.2.2 (2026-07-07): on desktop it runs **whisper.cpp with a downloaded GGML model**, which violates constraint 1. On Android it uses the OS `SpeechRecognizer`. Do not use it on desktop. ([README](https://github.com/brenogonzaga/tauri-plugin-stt))
- Android `SpeechRecognizer`: "The implementation of this API is likely to stream audio to remote servers". `createOnDeviceSpeechRecognizer` (API 31+) returns "a new on-device SpeechRecognizer" and throws `UnsupportedOperationException` when `isOnDeviceRecognitionAvailable` is false. ([SpeechRecognizer](https://developer.android.com/reference/android/speech/SpeechRecognizer)) Below API 31 the only option is `RecognizerIntent.EXTRA_PREFER_OFFLINE` (API 23), a hint that "may have no effect" depending on the recognizer, so it does not guarantee on-device processing. ([RecognizerIntent](https://developer.android.com/reference/android/speech/RecognizerIntent))
- No official Tauri plugin for speech exists in the v2 plugin index I checked (**UNVERIFIED**: the `llms.txt` index at v2.tauri.app was short and listed none).

### Microphone Capture in the Native Shells

- WebKitGTK builds MediaStream (`getUserMedia`) and MediaRecorder on by default, but WebRTC is on only with experimental features. ([OptionsGTK.cmake](https://github.com/WebKit/WebKit/blob/main/Source/cmake/OptionsGTK.cmake)) Plan on **WebSocket, not WebRTC**, for live sessions in the Linux client.
- WebKitGTK denies `getUserMedia` silently unless the host app enables media streams and handles the permission request; community Tauri fixes do both. **UNVERIFIED** against current wry or Tauri APIs ([Tauri discussion 8426](https://github.com/tauri-apps/tauri/discussions/8426)).
- Android needs `RECORD_AUDIO` and a WebView permission grant; Tauri's handling of WebView media permission prompts is **UNVERIFIED** ([Tauri discussion 12732](https://github.com/orgs/tauri-apps/discussions/12732)).

## 6. Recommended Design for Copilot

### Capability Model

Keep audio as data on the enabled model and on the provider connection, not as code paths per vendor.

Per enabled model (already planned in `docs/design/copilot.md`):

- `audio_in`: the chat request may carry an audio part. Forwarded as `input_audio` (OpenAI-compatible, OpenRouter, vLLM, llama.cpp) or `inlineData` (Gemini).
- `audio_out`: the chat response may carry audio. Today only OpenAI `gpt-audio-1.5` (Chat Completions) and OpenRouter audio chat models.
- `live`: the model runs a realtime session: `openai_realtime`, `openai_live`, `gemini_live`, or none.

Per provider connection (new):

- `speech_model` and `speech_voice`: a text-to-speech endpoint the connection offers (OpenAI `/v1/audio/speech` until 2027-01-06, Gemini TTS, OpenRouter, Ollama, llama.cpp, vLLM-Omni).
- `transcription_model`: an STT endpoint the connection offers (OpenAI `gpt-transcribe`, Gemini `gemini-3.5-transcribe`, OpenRouter, Ollama, vLLM, llama.cpp).

Defaults:

| Provider kind | `audio_in` | `audio_out` | `live` | Source of default |
| --- | --- | --- | --- | --- |
| Anthropic | false, locked | false, locked | none | Models API has no audio capability |
| OpenAI | true for `gpt-audio-*` | true for `gpt-audio-*` | realtime for `gpt-realtime-*`, live for `gpt-live-*` | Curated table by model prefix |
| Gemini | true for Gemini 4 models | false (TTS models are separate) | `gemini_live` for `*-live` | Curated table |
| OpenRouter | from `architecture.input_modalities` | from `architecture.output_modalities` | none | Models API |
| Ollama, vLLM, llama.cpp, LM Studio, LocalAI, other | false | false | none | User toggles per model |

"Locked" means the UI does not offer the toggle, so nobody sends audio to an API that rejects it.

### Feature 1: Push-to-Talk Audio Input (Default Voice Input)

- Shown only when the selected model has `audio_in`.
- Record in the client and send one bounded clip with the message. This works on every client, needs no realtime connection, and keeps the backend a plain forwarder.
- Encode **WAV (PCM16 mono)** in the client from an AudioWorklet. Reason: OpenAI accepts only `wav` or `mp3` for `input_audio`, and Chromium's MediaRecorder produces WebM/Opus. Gemini, OpenRouter Chat Completions, and llama.cpp accept WAV too. WAV encoding is a header plus samples in TypeScript, no codec library, no model. This changes the `MediaRecorder` line in `docs/design/copilot.md`.
- Size guard: Gemini inline audio needs the whole request under 20 MB; larger clips go through the provider's file upload or are refused with a clear message.
- Store the clip as a message attachment (it is the user's message) and show the provider's transcript when one comes back (OpenAI audio replies include `transcript`).

### Feature 2: Read Aloud (Audio Output)

Order of preference for an assistant message's speaker button:

1. The model itself returned audio (`audio_out` models): play the streamed `delta.audio` chunks as they arrive and keep the audio id for multi-turn.
2. The connection that served the message has a `speech_model`: the API calls the provider's speech endpoint and streams the bytes back to the client (`pcm` or `l16` chunks for low latency, `wav` for download). This is the same provider speaking its own model's words, so it is not a cross-provider translation layer.
3. Otherwise: no speaker button (this is the Anthropic case by default).

Plan for the OpenAI cutoff: on 2027-01-06 `/v1/audio/speech` loses all OpenAI models. The OpenAI adapter then needs a Realtime-based speech path (an out-of-band response with "Say exactly the following") or the owner switches read-aloud to Gemini TTS.

### Feature 3: Live Voice (Streaming Both Ways)

- Offered only for models with `live` set (OpenAI Realtime or GPT-Live, Gemini Live).
- The API mints a short-lived credential bound to a server-built session config: OpenAI `POST /v1/realtime/client_secrets` (or `/v1/live/sessions` for GPT-Live), Gemini `POST /v1beta/auth_tokens` with locked config. The project's instructions go into that config, so the system prompt never travels from the client. Gemini tokens can lock those fields; an OpenAI client secret's config is only a default that the client connection can override, so treat OpenAI live sessions as client-controllable (acceptable for Copilot's own signed-in clients, but do not rely on it as an enforcement point).
- The client connects straight to the provider. Use WebRTC on web and Android and **WebSocket on the Linux (WebKitGTK) client**. The Copilot API stays out of the audio path, which keeps it light.
- The client posts the session's input and output transcripts (provider transcription events) back to the Copilot API, so the chat history stays complete and searchable.
- Push-to-talk is the default inside Live mode too (more reliable in noise, as Anthropic's own help page also notes); hands-free with provider voice activity detection is a toggle.
- Watch limits: Gemini audio sessions end at 15 minutes without session resumption; OpenAI client secrets expire after 600 seconds by default (the session itself continues).

### Option C: Voice for Models Without Audio (Owner Decision, Off by Default)

These would let Claude chats use voice, but each is the "translation" the owner said not to build unless chosen. None runs a model inside Copilot.

| Option | Speech in | Speech out | Privacy | Weight added to Copilot |
| --- | --- | --- | --- | --- |
| C1. Cloud speech connection | Another provider's transcription endpoint (OpenAI `gpt-transcribe`, Gemini `gemini-3.5-transcribe`, OpenRouter, or a self-hosted Ollama, vLLM, or llama.cpp) | That provider's TTS | Audio goes to the chosen provider or the owner's own server | None (HTTP calls only) |
| C2. Device speech | Firefox desktop on-device recognition; Chrome with `processLocally`; Android on-device `SpeechRecognizer` via native code. No zero-weight option on Linux desktop (WebKitGTK has no recognizer) | `speechSynthesis` on web and WebKitGTK; `tauri-plugin-tts` on Android | On device if configured; Chrome default and Android default may use vendor cloud | None in Copilot; the OS or browser engine does the work |
| C3. GPT-Live with client delegation | OpenAI voice front end | OpenAI voice | Audio to OpenAI, text to Anthropic | Moderate (session plumbing) |

Recommendation for v1: build Features 1 to 3 behind the flags, ship Anthropic with voice off, and ask the owner whether to add C1 or C2 later.

### Backend Footprint

No audio codecs or ML crates are needed. The API needs only what it already uses for forwarding: an HTTP client with multipart upload (transcription), base64, and streaming response bodies (speech). It needs a WebSocket client only if Copilot ever proxies live audio, which this design avoids.

## Open Questions

1. Should Copilot keep recorded audio clips after the provider responds, or store only the transcript? (Storage and privacy policy.)
2. Does the owner accept C1 or C2 for Claude chats, or keep voice strictly provider-native?
3. Which voice should be the default per provider (OpenAI `marin` or `cedar`, Gemini `Kore`)? This is taste, not research.

## Unverified Items

- ElevenLabs as Claude voice mode's TTS vendor.
- Ollama chat models with native audio input.
- WebKitGTK `SpeechRecognition` absent at runtime, and per-distro `speechSynthesis` availability.
- Android WebView `SpeechRecognition` on current WebView versions.
- Network versus on-device voices in Android's default TTS engine.
- Current Tauri and wry APIs for granting microphone permission on Linux and Android.
- Existence of an official Tauri speech plugin.

## Verification

An independent reviewer re-opened the primary sources on 2026-11-26 (Markdown versions of the docs pages where offered) and checked the eight load-bearing claims:

| # | Claim | Result | Source opened |
| --- | --- | --- | --- |
| 1 | Messages API has no audio block; media types are only JPEG, PNG, GIF, WebP, PDF, plain text | Confirmed (no `audio` string anywhere in the reference; only those six `media_type` values) | [Create a Message](https://platform.claude.com/docs/en/api/messages/create) |
| 2 | OpenAI compatibility layer: `input_audio` returns 400, `audio` and `modalities` ignored, `message.audio` always empty | Confirmed (exact wording) | [OpenAI SDK compatibility](https://platform.claude.com/docs/en/cli-sdks-libraries/libraries/openai-sdk) |
| 3 | Voice mode is an app feature; speech converted to text, recording deleted, transcript saved; hands-free and push-to-talk; Fable not available | Confirmed; the page adds "By default" before "we don't keep the audio", and users on some plans can opt in to share recordings | [Use voice mode](https://support.claude.com/en/articles/11101966-use-voice-mode) |
| 4 | Chat Completions `input_audio` is wav or mp3; audio output formats; `gpt-audio-1.5` Chat Completions only | Confirmed; added a note on the reference's inconsistent voice and format lists | [Create chat completion](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create), [gpt-audio-1.5](https://developers.openai.com/api/docs/models/gpt-audio-1.5) |
| 5 | `tts-1`, `tts-1-hd`, `gpt-4o-mini-tts` shut down 2027-01-06; `gpt-realtime-2.1-mini` not supported on `/v1/audio/speech` | Confirmed (guide banner, deprecations table, and speech `model` enum all agree) | [Text to speech](https://developers.openai.com/api/docs/guides/text-to-speech), [Deprecations](https://developers.openai.com/api/docs/deprecations), [Create speech](https://developers.openai.com/api/reference/resources/audio/subresources/speech/methods/create) |
| 6 | `POST /v1/realtime/client_secrets`, default 600 s, range 10 to 7200 s, carries session config | Confirmed; corrected the implication that the config is enforced: the client connection can override it | [Create client secret](https://developers.openai.com/api/reference/resources/realtime/subresources/client_secrets/methods/create) |
| 7 | Gemini Live is WebSocket, 16 kHz PCM in, 24 kHz PCM out; ephemeral tokens preview, Live only, `POST /v1beta/auth_tokens`; 3.8 TTS is text in, audio out | Confirmed; added that Live resamples other input rates | [Ephemeral tokens](https://ai.google.dev/gemini-api/docs/ephemeral-tokens), [Live API](https://ai.google.dev/gemini-api/docs/live), [Live guide](https://ai.google.dev/gemini-api/docs/live-guide), [Text-to-speech](https://ai.google.dev/gemini-api/docs/speech-generation), [Gemini 3.8 Flash TTS](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash-tts) |
| 8 | Android `SpeechRecognizer` "is likely to stream audio to remote servers"; on-device needs `createOnDeviceSpeechRecognizer` (API 31+) | Confirmed for the quote and API level; softened "requires": `EXTRA_PREFER_OFFLINE` (API 23) exists but is only a hint | [SpeechRecognizer](https://developer.android.com/reference/android/speech/SpeechRecognizer), [RecognizerIntent](https://developer.android.com/reference/android/speech/RecognizerIntent) |

Also spot-checked: the Models API `capabilities` object has `image_input` and `pdf_input` and no audio field ([List Models](https://platform.claude.com/docs/en/api/models/list)); the release notes' only audio mention is the 2026-09-01 C2PA entry, and `llms.txt` has no voice, speech, or realtime page ([Release notes](https://platform.claude.com/docs/en/release-notes/overview), [llms.txt](https://platform.claude.com/llms.txt)); the Transparency Hub model list was corrected (see Section 1). Not re-checked by the reviewer: Sections 4 and 5 (self-hosted servers, OpenRouter, Web Speech, Tauri plugins) and the GPT-Live details.
