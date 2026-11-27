// Voice in the chat screens: the microphone's states and timer, how a clip ends, the controls
// each model gets, recordings and spoken replies in the conversation, the Speak replies setting,
// and which stream events play.

import type { Capabilities, ChatStreamEvent, MessagePartDto } from "@copilot/contracts";
import { describe, expect, test } from "@rstest/core";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Composer } from "@web/components/Chat/Composer";
import { refocus } from "@web/components/Chat/composer";
import { MessageList } from "@web/components/Chat/MessageList";
import { VOICE_MESSAGE } from "@web/components/Chat/messages";
import { draftBody } from "@web/components/Chat/StartChat";
import { AudioPart, RecordingBar, recordingTime, SpeakToggle } from "@web/components/Chat/Voice";
import { clipOutcome, MIN_CLIP_MS, type MicState, mic, micHost } from "@web/hooks/useRecorder";
import type { Speaker } from "@web/hooks/useSpeaker";
import { messageFiles, speaksReplies } from "@web/routes/chat/page";
import type { ViewMessage } from "@web/routes/chat/stream";
import { audioFormat, draftMessage, speaks, turnVoice } from "@web/routes/chat/turn";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";

const MODEL = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708100";
const FILE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708210";
const OTHER = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708211";
const MESSAGE = "0196f6a4-7c1e-7d2a-9b3c-4d5e6f708111";

const capabilities: Capabilities = {
  vision: false,
  pdf: false,
  tools: true,
  audioIn: false,
  audioOut: false,
  reasoning: false,
  reasoningLevels: [],
  defaultReasoning: null,
};

function render(node: ReactNode): string {
  return renderToString(
    <QueryClientProvider client={new QueryClient()}>{node}</QueryClientProvider>
  );
}

function message(role: ViewMessage["role"], parts: MessagePartDto[]): ViewMessage {
  return {
    id: MESSAGE,
    role,
    status: "complete",
    finishReason: "stop",
    usage: null,
    error: null,
    parts,
  };
}

const audio = (
  extra: Partial<Extract<MessagePartDto, { type: "audio" }>> = {}
): MessagePartDto => ({
  type: "audio",
  fileId: FILE,
  format: "wav",
  transcript: null,
  durationMs: null,
  ...extra,
});

describe("the microphone's states", () => {
  const idle: MicState = { kind: "idle" };

  test("start waits for the microphone, then records from zero", () => {
    const starting = mic(idle, { type: "start" });
    expect(starting).toEqual({ kind: "starting" });
    expect(mic(starting, { type: "started" })).toEqual({ kind: "recording", ms: 0 });
    // Starting twice changes nothing.
    expect(mic(starting, { type: "start" })).toBe(starting);
    expect(mic(idle, { type: "started" })).toBe(idle);
  });

  test("the timer moves once a second, not with every chunk", () => {
    const recording: MicState = { kind: "recording", ms: 0 };
    expect(mic(recording, { type: "progress", ms: 850 })).toBe(recording);
    const next = mic(recording, { type: "progress", ms: 1_020 });
    expect(next).toEqual({ kind: "recording", ms: 1_020 });
    expect(mic(next, { type: "progress", ms: 1_990 })).toBe(next);
    expect(mic(idle, { type: "progress", ms: 5_000 })).toBe(idle);
  });

  test("stop keeps the time while the last samples arrive; reset ends everything", () => {
    const stopping = mic({ kind: "recording", ms: 12_000 }, { type: "stop" });
    expect(stopping).toEqual({ kind: "stopping", ms: 12_000 });
    expect(mic(stopping, { type: "progress", ms: 13_000 })).toBe(stopping);
    expect(mic(idle, { type: "stop" })).toBe(idle);
    expect(mic(stopping, { type: "reset" })).toEqual({ kind: "idle" });
    expect(mic({ kind: "starting" }, { type: "reset" })).toEqual({ kind: "idle" });
  });

  test("a clip is kept unless it is too short, and its ending is explained", () => {
    expect(clipOutcome(MIN_CLIP_MS - 1, null)).toEqual({
      keep: false,
      note: { text: "The recording was too short.", problem: false },
    });
    expect(clipOutcome(4_000, null)).toEqual({ keep: true, note: null });
    expect(clipOutcome(300_000, "full")).toEqual({
      keep: true,
      note: { text: "Recording stopped at the 5-minute limit.", problem: false },
    });
    expect(clipOutcome(2_000, "lost")).toEqual({
      keep: true,
      note: { text: "The microphone stopped, so the recording ends there.", problem: false },
    });
  });

  test("the timer shows the time recorded against the 5-minute cap", () => {
    expect(recordingTime(0)).toBe("0:00 / 5:00");
    expect(recordingTime(72_500)).toBe("1:12 / 5:00");
  });
});

describe("the recording bar", () => {
  test("shows the timer, Cancel, and Stop while recording", () => {
    const html = render(
      <RecordingBar
        state={{ kind: "recording", ms: 12_000 }}
        onStop={() => undefined}
        onCancel={() => undefined}
      />
    );
    expect(html).toContain("Recording");
    expect(html).toContain("0:12 / 5:00");
    expect(html).toContain('role="timer"');
    expect(html).toContain('aria-label="Cancel recording"');
    expect(html).toMatch(/<button[^>]*aria-label="Stop recording"[^>]*>/);
    expect(html).not.toMatch(/<button[^>]*disabled=""[^>]*aria-label="Stop recording"/);
  });

  test("waits for the microphone without a timer, and Stop waits for the recording", () => {
    const html = render(
      <RecordingBar
        state={{ kind: "starting" }}
        onStop={() => undefined}
        onCancel={() => undefined}
      />
    );
    expect(html).toContain("Waiting for the microphone...");
    expect(html).not.toContain('role="timer"');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Stop recording"/);
    expect(html).toContain('aria-label="Cancel recording"');
  });

  test("closing it puts focus back in the composer", () => {
    // After Stop, the field, to write and send with the clip; after Cancel, the microphone.
    expect(refocus("stop", true, true)).toBe("field");
    expect(refocus("cancel", true, true)).toBe("mic");
    // Touch screens get the microphone, so the keyboard does not open over the clip.
    expect(refocus("stop", true, false)).toBe("mic");
    expect(refocus("cancel", true, false)).toBe("mic");
    // Focus that already moved elsewhere stays there.
    expect(refocus("stop", false, true)).toBeNull();
    expect(refocus("cancel", false, false)).toBeNull();
  });
});

describe("the composer's voice controls", () => {
  const send = async () => true;
  const speak = { on: false, onChange: () => undefined };

  test("the microphone shows only for models that take audio", () => {
    expect(render(<Composer onSend={send} capabilities={capabilities} />)).not.toContain(
      "Record a voice message"
    );
    expect(render(<Composer onSend={send} />)).not.toContain("Record a voice message");
    const html = render(
      <Composer onSend={send} capabilities={{ ...capabilities, audioIn: true }} />
    );
    expect(html).toContain('aria-label="Record a voice message"');
  });

  test("Speak replies shows only for models that answer with audio", () => {
    expect(
      render(<Composer onSend={send} capabilities={capabilities} speak={speak} />)
    ).not.toContain("Speak replies");
    const able = { ...capabilities, audioOut: true };
    expect(render(<Composer onSend={send} capabilities={able} />)).not.toContain("Speak replies");
    const off = render(<Composer onSend={send} capabilities={able} speak={speak} />);
    expect(off).toContain('aria-label="Speak replies"');
    expect(off).toContain('aria-pressed="false"');
    const on = render(
      <Composer onSend={send} capabilities={able} speak={{ ...speak, on: true }} />
    );
    expect(on).toContain('aria-pressed="true"');
    expect(on).toContain("Speak replies: on");
  });

  test("the toggle says its state", () => {
    const html = render(<SpeakToggle on={false} onChange={() => undefined} disabled />);
    expect(html).toContain('aria-pressed="false"');
    expect(html).toContain("Speak replies: off");
    expect(html).toContain("disabled");
  });
});

describe("recordings and spoken replies in the conversation", () => {
  test("a saved recording plays on request, with its length", () => {
    const html = render(<AudioPart fileId={FILE} label={VOICE_MESSAGE} durationMs={12_400} />);
    expect(html).toContain('aria-label="Play voice message"');
    expect(html).toContain("0:12");
    // The bytes come with the token on Play; nothing loads before.
    expect(html).not.toContain("<audio");
  });

  test("a voice message shows its player and the provider's transcript", () => {
    const html = render(
      <MessageList messages={[message("user", [audio({ transcript: "Book a table for two." })])]} />
    );
    expect(html).toContain('aria-label="Play voice message"');
    expect(html).toContain("Book a table for two.");
  });

  test("a spoken reply shows its player; its transcript shows only without answer text", () => {
    const spoken = render(
      <MessageList
        messages={[
          message("assistant", [
            { type: "text", text: "Here is the plan." },
            audio({ transcript: "Here is the plan.", durationMs: 3_000 }),
          ]),
        ]}
      />
    );
    expect(spoken).toContain('aria-label="Play spoken reply"');
    expect(spoken.split("Here is the plan.")).toHaveLength(2);

    const bare = render(
      <MessageList messages={[message("assistant", [audio({ transcript: "Only audio." })])]} />
    );
    expect(bare).toContain("Only audio.");
  });
});

describe("recordings sent with a message", () => {
  test("a recording shows as an audio part until the message is saved", () => {
    const draft = draftMessage("Listen", [
      { fileId: FILE, name: "Voice message.wav", mediaType: "audio/wav" },
      { fileId: OTHER, name: "notes.pdf", mediaType: "application/pdf" },
    ]);
    expect(draft.parts).toEqual([
      { type: "text", text: "Listen" },
      { type: "audio", fileId: FILE, format: "wav", transcript: null, durationMs: null },
      { type: "file", fileId: OTHER, name: "notes.pdf", mediaType: "application/pdf" },
    ]);
    expect(audioFormat("audio/mpeg")).toBe("mp3");
    expect(audioFormat("audio/wav")).toBe("wav");
  });

  test("an edit sends the message's recordings again with its files", () => {
    const sent = message("user", [
      { type: "text", text: "Hi" },
      audio(),
      audio({ fileId: OTHER, format: "mp3" }),
      { type: "file", fileId: MODEL, name: "a.png", mediaType: "image/png" },
    ]);
    expect(messageFiles(sent)).toEqual([
      { fileId: FILE, name: VOICE_MESSAGE, mediaType: "audio/wav" },
      { fileId: OTHER, name: VOICE_MESSAGE, mediaType: "audio/mpeg" },
      { fileId: MODEL, name: "a.png", mediaType: "image/png" },
    ]);
  });
});

describe("Speak replies", () => {
  test("is on only for a chat with it on and a model that answers with audio", () => {
    const able = { capabilities: { ...capabilities, audioOut: true } };
    expect(speaksReplies({ voiceReplies: true }, able)).toBe(true);
    expect(speaksReplies({ voiceReplies: false }, able)).toBe(false);
    expect(speaksReplies({ voiceReplies: true }, { capabilities })).toBe(false);
    expect(speaksReplies(undefined, able)).toBe(false);
    expect(speaksReplies({ voiceReplies: true }, undefined)).toBe(false);
  });

  test("a new chat asks for spoken replies only when the draft turned it on", () => {
    const none = { toolOverrides: [], builtinTools: null };
    expect(draftBody(MODEL, null, null, none)).toEqual({ modelId: MODEL });
    expect(draftBody(MODEL, null, null, none, false)).toEqual({ modelId: MODEL });
    expect(draftBody(MODEL, null, null, none, true)).toEqual({
      modelId: MODEL,
      voiceReplies: true,
    });
  });

  test("plays the audio of turns this page started, once per event", () => {
    const delta: ChatStreamEvent = {
      type: "audio.delta",
      seq: 4,
      messageId: MESSAGE,
      data: "AAA=",
      format: "pcm16",
    };
    expect(speaks(delta, true, 3)).toBe(true);
    // A replay after a dropped connection never plays twice.
    expect(speaks(delta, true, 4)).toBe(false);
    // A run attached to on opening the chat stays quiet.
    expect(speaks(delta, false, 0)).toBe(false);
    expect(speaks({ type: "done", seq: 5 }, true, 0)).toBe(false);
  });

  /** A speaker that records what it was asked to do. */
  function recorded() {
    const calls: string[] = [];
    const speaker: Speaker = {
      begin: () => calls.push("begin"),
      play: (data) => calls.push(`play ${data}`),
      stop: () => calls.push("stop"),
      unlock: () => calls.push("unlock"),
    };
    return { calls, speaker };
  }

  const delta = (seq: number): ChatStreamEvent => ({
    type: "audio.delta",
    seq,
    messageId: MESSAGE,
    data: `d${seq}`,
    format: "pcm16",
  });

  test("Stop silences the turn, and deltas that arrive after it never play", () => {
    const { calls, speaker } = recorded();
    const voice = turnVoice(() => speaker);
    voice.begin();
    voice.hear(delta(1));
    voice.hear(delta(2));
    voice.stop();
    // The stopped run winds down and the stream still carries its deltas.
    voice.hear(delta(3));
    voice.hear(delta(4));
    expect(calls).toEqual(["begin", "play d1", "play d2", "stop"]);

    // The next turn speaks again, from its own first delta.
    voice.begin();
    voice.hear(delta(1));
    expect(calls.slice(4)).toEqual(["begin", "play d1"]);
  });

  test("a turn that ends lets queued audio play out and plays nothing more", () => {
    const { calls, speaker } = recorded();
    const voice = turnVoice(() => speaker);
    voice.begin();
    voice.hear(delta(1));
    voice.hear(delta(1));
    voice.quiet();
    voice.hear(delta(2));
    expect(calls).toEqual(["begin", "play d1"]);
  });

  test("a page without a speaker, or a turn it did not start, stays quiet", () => {
    const { calls, speaker } = recorded();
    let current: Speaker | undefined;
    const voice = turnVoice(() => current);
    voice.hear(delta(1));
    voice.begin();
    voice.hear(delta(2));
    current = speaker;
    voice.hear(delta(3));
    expect(calls).toEqual(["play d3"]);
  });
});

describe("microphone host", () => {
  test("is the browser on the web, Android in the Android app, and the desktop in the Linux app", () => {
    const real = Object.getOwnPropertyDescriptor(globalThis, "navigator");
    const agent = (userAgent: string) =>
      Object.defineProperty(globalThis, "navigator", { configurable: true, value: { userAgent } });
    try {
      agent("Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/142.0");
      expect(micHost()).toBe("browser");
      Object.defineProperty(globalThis, "isTauri", { value: true, configurable: true });
      expect(micHost()).toBe("android");
      agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)");
      expect(micHost()).toBe("desktop");
    } finally {
      Reflect.deleteProperty(globalThis, "isTauri");
      if (real) Object.defineProperty(globalThis, "navigator", real);
    }
  });
});
