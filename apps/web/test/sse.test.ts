import { describe, expect, test } from "@rstest/core";
import { readSse, type SseMessage, sseParser } from "@web/api/sse";

const utf8 = new TextEncoder();

/** A body that delivers `chunks` one `read()` at a time. */
function body(chunks: (string | Uint8Array)[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(typeof chunk === "string" ? utf8.encode(chunk) : chunk);
      }
      controller.close();
    },
  });
}

function parse(chunks: string[]): SseMessage[] {
  const messages: SseMessage[] = [];
  const parser = sseParser((message) => messages.push(message));
  for (const chunk of chunks) parser.feed(chunk);
  return messages;
}

describe("sseParser", () => {
  test("dispatches an event at each blank line", () => {
    expect(parse(['data: {"a":1}\n\ndata: {"a":2}\n\n'])).toEqual([
      { event: "message", data: '{"a":1}', id: "" },
      { event: "message", data: '{"a":2}', id: "" },
    ]);
  });

  test("accepts LF, CRLF, and CR line endings", () => {
    expect(
      parse(["data: lf\n\n", "data: crlf\r\n\r\n", "data: cr\r\r"]).map((m) => m.data)
    ).toEqual(["lf", "crlf", "cr"]);
  });

  test("keeps a CRLF split across chunks as one line ending", () => {
    expect(parse(["data: one\r", "\n\r", "\ndata: two\r\n\r\n"]).map((m) => m.data)).toEqual([
      "one",
      "two",
    ]);
  });

  test("handles any split of the text", () => {
    const text = 'event: chat\nid: 7\ndata: {"type":"done","seq":3}\n\n';
    for (let at = 0; at <= text.length; at++) {
      expect(parse([text.slice(0, at), text.slice(at)])).toEqual([
        { event: "chat", data: '{"type":"done","seq":3}', id: "7" },
      ]);
    }
  });

  test("joins multi-line data with LF", () => {
    expect(parse(["data: first\ndata: second\ndata\n\n"])[0]?.data).toBe("first\nsecond\n");
  });

  test("ignores comments such as keep-alives", () => {
    expect(parse([": keep-alive\n\n", ":\n", "data: x\n\n"])).toEqual([
      { event: "message", data: "x", id: "" },
    ]);
  });

  test("removes only one space after the colon", () => {
    expect(parse(["data:no-space\n\n", "data:  two\n\n"]).map((m) => m.data)).toEqual([
      "no-space",
      " two",
    ]);
  });

  test("strips a leading byte order mark", () => {
    expect(parse(["﻿data: x\n\n"])[0]?.data).toBe("x");
  });

  test("skips events without data and unknown fields", () => {
    expect(parse(["event: ping\n\n", "retry: 10\nfoo: bar\ndata: y\n\n"])).toEqual([
      { event: "message", data: "y", id: "" },
    ]);
  });

  test("keeps the last id across events", () => {
    expect(parse(["id: 1\ndata: a\n\ndata: b\n\n"]).map((m) => m.id)).toEqual(["1", "1"]);
  });

  test("drops an event that the stream ends in the middle of", () => {
    expect(parse(["data: complete\n\n", "data: cut off\n"]).map((m) => m.data)).toEqual([
      "complete",
    ]);
  });
});

describe("readSse", () => {
  test("decodes multi-byte characters split across reads", async () => {
    const bytes = utf8.encode("data: héllo ✓\n\n");
    const messages: SseMessage[] = [];
    await readSse(body([bytes.slice(0, 8), bytes.slice(8, 15), bytes.slice(15)]), (message) =>
      messages.push(message)
    );
    expect(messages.map((m) => m.data)).toEqual(["héllo ✓"]);
  });

  test("reads with reader.read(), not async iteration", async () => {
    const source = body(["data: a\n\n", "data: b\n\n"]);
    let reads = 0;
    const reader = source.getReader();
    // Only `getReader` exists: no `Symbol.asyncIterator`, as on older WebKit.
    const bare = {
      getReader: () => ({
        read: () => {
          reads++;
          return reader.read();
        },
        cancel: () => reader.cancel(),
        releaseLock: () => reader.releaseLock(),
      }),
      cancel: () => reader.cancel(),
    } as unknown as ReadableStream<Uint8Array>;
    const messages: SseMessage[] = [];
    await readSse(bare, (message) => messages.push(message));
    expect(messages.map((m) => m.data)).toEqual(["a", "b"]);
    expect(reads).toBe(3);
  });

  test("releases the body when it ends", async () => {
    const source = body(["data: a\n\n"]);
    await readSse(source, () => undefined);
    expect(source.locked).toBe(false);
  });

  test("rejects with AbortError and cancels the body when aborted", async () => {
    let cancelled = false;
    let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
    const source = new ReadableStream<Uint8Array>({
      start(c) {
        controller = c;
        c.enqueue(utf8.encode("data: first\n\n"));
      },
      cancel() {
        cancelled = true;
      },
    });
    const abort = new AbortController();
    const messages: string[] = [];
    const reading = readSse(
      source,
      (message) => {
        messages.push(message.data);
        abort.abort();
      },
      abort.signal
    );
    await expect(reading).rejects.toMatchObject({ name: "AbortError" });
    expect(messages).toEqual(["first"]);
    expect(cancelled).toBe(true);
    expect(controller).toBeDefined();
  });

  test("rejects at once for a signal that is already aborted", async () => {
    let cancelled = false;
    const source = new ReadableStream<Uint8Array>({
      cancel() {
        cancelled = true;
      },
    });
    const abort = new AbortController();
    abort.abort();
    await expect(readSse(source, () => undefined, abort.signal)).rejects.toMatchObject({
      name: "AbortError",
    });
    expect(cancelled).toBe(true);
  });

  test("cancels the body when a handler throws", async () => {
    let cancelled = false;
    const source = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(utf8.encode("data: bad\n\n"));
      },
      cancel() {
        cancelled = true;
      },
    });
    await expect(
      readSse(source, () => {
        throw new Error("bad event");
      })
    ).rejects.toThrow("bad event");
    expect(cancelled).toBe(true);
  });
});
