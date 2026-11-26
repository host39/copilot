// Server-sent events over `fetch`. `EventSource` can neither POST nor send an `Authorization`
// header, so the chat routes (`POST /chats/{id}/messages` and the others) are read from the
// response body instead. The parser follows the `text/event-stream` rules of the HTML standard:
// LF, CRLF, or CR line endings, `:` comments (the server's keep-alives), multi-line `data`, and an
// event at the end of the stream without its blank line is dropped.

export type SseMessage = {
  /** The `event` field, `"message"` when the server sent none. */
  event: string;
  data: string;
  /** The last `id` field seen on the stream, `""` before any. */
  id: string;
};

export type SseParser = {
  /** Takes the next piece of decoded text, split anywhere. */
  feed(text: string): void;
};

const LINE_END = /\r\n|\r|\n/;

/** An incremental `text/event-stream` parser that calls `onMessage` for every complete event. */
export function sseParser(onMessage: (message: SseMessage) => void): SseParser {
  let pending = "";
  let started = false;
  let skipLf = false;
  let event = "";
  let data: string[] = [];
  let id = "";

  function dispatch(): void {
    if (data.length > 0) onMessage({ event: event || "message", data: data.join("\n"), id });
    event = "";
    data = [];
  }

  function field(name: string, value: string): void {
    if (name === "event") event = value;
    else if (name === "data") data.push(value);
    else if (name === "id" && !value.includes("\0")) id = value;
  }

  function line(text: string): void {
    if (text === "") {
      dispatch();
    } else if (!text.startsWith(":")) {
      const colon = text.indexOf(":");
      const value = colon === -1 ? "" : text.slice(colon + 1);
      field(
        colon === -1 ? text : text.slice(0, colon),
        value.startsWith(" ") ? value.slice(1) : value
      );
    }
  }

  return {
    feed(text) {
      if (text === "") return;
      let chunk = text;
      if (!started) {
        started = true;
        if (chunk.startsWith("\uFEFF")) chunk = chunk.slice(1);
      }
      // A CR that ended the previous chunk may be the first half of a CRLF.
      if (skipLf && chunk.startsWith("\n")) chunk = chunk.slice(1);
      const buffer = pending + chunk;
      skipLf = buffer.endsWith("\r");
      const lines = buffer.split(LINE_END);
      pending = lines.pop() ?? "";
      for (const next of lines) line(next);
    },
  };
}

/** The error a read rejects with once `signal` aborts, like `fetch` does. */
function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException("The operation was aborted.", "AbortError");
}

/**
 * Reads an event stream body to its end, calling `onMessage` for each event. Uses `reader.read()`
 * in a loop rather than `for await`: async iteration of `ReadableStream` is missing from the
 * WebKit versions the Linux app may run on. Aborting `signal` cancels the body and rejects.
 */
export async function readSse(
  body: ReadableStream<Uint8Array>,
  onMessage: (message: SseMessage) => void,
  signal?: AbortSignal
): Promise<void> {
  if (signal?.aborted) {
    await body.cancel().catch(() => undefined);
    throw abortReason(signal);
  }
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const parser = sseParser(onMessage);
  const cancel = () => {
    reader.cancel().catch(() => undefined);
  };
  signal?.addEventListener("abort", cancel, { once: true });
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (signal?.aborted) throw abortReason(signal);
      if (done) break;
      parser.feed(decoder.decode(value, { stream: true }));
    }
    parser.feed(decoder.decode());
  } catch (error) {
    // Stop the download too: the caller gave up on this stream.
    await reader.cancel().catch(() => undefined);
    throw error;
  } finally {
    signal?.removeEventListener("abort", cancel);
    reader.releaseLock();
  }
}
