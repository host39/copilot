// Parses chat, message, search, and stream event payloads with the Zod schemas. The event
// samples are the JSON the Rust tests in `packages/contracts/src/events.rs` pin.

import {
  type ChatStreamEvent,
  chatPageBodySchema,
  chatStreamEventSchema,
  decisionsRequestSchema,
  type MessagePartDto,
  messageListBodySchema,
  type messagePartSchema,
  type ResultContent,
  type resultContentSchema,
  searchListBodySchema,
  sendMessageRequestSchema,
  type UserEvent,
  updateChatRequestSchema,
  userEventSchema,
} from "@contracts/index";
import { describe, expect, test } from "@rstest/core";
import type { z } from "zod";

/** `true` only when `A` and `B` are the same type. */
type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

// `satisfies` proves each schema's output fits its type; these prove every variant has a schema.
const everyStreamEvent: Equal<
  z.output<typeof chatStreamEventSchema>["type"],
  ChatStreamEvent["type"]
> = true;
const everyUserEvent: Equal<z.output<typeof userEventSchema>["type"], UserEvent["type"]> = true;
const everyPart: Equal<z.output<typeof messagePartSchema>["type"], MessagePartDto["type"]> = true;
const everyContent: Equal<z.output<typeof resultContentSchema>["type"], ResultContent["type"]> =
  true;

const ID = "019abf6c-2f6e-7a51-8c3e-6d1e2f3a4b5c";
const AT = "2026-11-26T12:00:00.123456Z";

describe("stream events", () => {
  test("cover every variant", () => {
    expect([everyStreamEvent, everyUserEvent, everyPart, everyContent]).toEqual([
      true,
      true,
      true,
      true,
    ]);
  });

  test("parse a run from start to done", () => {
    const run = [
      { type: "message.started", seq: 1, messageId: ID, role: "assistant", windowStartId: null },
      { type: "part.started", seq: 2, messageId: ID, index: 0, kind: "toolCall" },
      { type: "text.delta", seq: 3, messageId: ID, index: 0, text: "Hi" },
      { type: "reasoning.delta", seq: 4, messageId: ID, index: 1, text: "Thinking" },
      { type: "audio.delta", seq: 5, messageId: ID, data: "AAA=", format: "pcm16" },
      {
        type: "tool.requested",
        seq: 6,
        messageId: ID,
        callId: ID,
        serverId: null,
        toolName: "schedule_prompt",
        title: "Schedule a prompt",
        arguments: { name: "News" },
        needsApproval: true,
      },
      {
        type: "tool.finished",
        seq: 7,
        messageId: ID,
        callId: ID,
        status: "ok",
        output: [{ type: "text", text: "done" }],
        truncated: false,
      },
      {
        type: "message.finished",
        seq: 8,
        messageId: ID,
        status: "refused",
        finishReason: "refusal",
        refusal: { category: null, explanation: "The model declined to answer." },
        usage: {
          inputTokens: 10,
          outputTokens: null,
          reasoningTokens: null,
          cacheReadTokens: null,
          cacheWriteTokens: null,
        },
      },
      {
        type: "error",
        seq: 9,
        message: "Key rejected: open provider settings.",
        code: "PROVIDER_AUTH",
      },
      { type: "done", seq: 10 },
    ];
    const events = run.map((event) => chatStreamEventSchema.parse(event));
    expect(events.map((event) => event.seq)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    expect(events.at(-1)?.type).toBe("done");
  });

  test("reject events without seq or of an unknown type", () => {
    expect(chatStreamEventSchema.safeParse({ type: "done" }).success).toBe(false);
    expect(chatStreamEventSchema.safeParse({ type: "message.ended", seq: 1 }).success).toBe(false);
  });

  test("parse every user event", () => {
    const events = [
      { type: "chat.updated", chatId: ID },
      { type: "run.started", chatId: ID },
      { type: "run.finished", chatId: ID, status: "awaiting_approval" },
      { type: "approval.pending", chatId: ID, messageId: ID },
      { type: "schedule.updated", scheduleId: ID },
    ];
    for (const event of events) {
      expect(userEventSchema.parse(event)).toEqual(event);
    }
  });
});

describe("chats and messages", () => {
  test("a page of chats ends with a null cursor", () => {
    const body = chatPageBodySchema.parse({
      items: [
        {
          id: ID,
          projectId: null,
          modelId: ID,
          scheduleId: ID,
          title: "Morning news · 26.11.2026",
          pinned: false,
          unread: true,
          awaitingApproval: false,
          createdAt: AT,
          updatedAt: AT,
        },
      ],
      next: null,
    });
    expect(body.next).toBeNull();
    expect(body.items[0]?.unread).toBe(true);
  });

  test("messages carry tool calls and results without provider data", () => {
    const message = {
      id: ID,
      chatId: ID,
      role: "assistant",
      status: "awaiting_approval",
      finishReason: "tool_calls",
      modelId: ID,
      parts: [
        { type: "text", text: "Let me check." },
        {
          type: "toolCall",
          callId: ID,
          serverId: ID,
          toolName: "search_docs",
          arguments: { query: "cron" },
          complete: true,
          approval: "pending",
          denyReason: null,
        },
      ],
      usage: null,
      error: null,
      createdAt: AT,
      updatedAt: AT,
    };
    const results = {
      ...message,
      role: "tool",
      status: "complete",
      finishReason: null,
      modelId: null,
      parts: [
        {
          type: "toolResult",
          callId: ID,
          status: "denied",
          isError: true,
          content: [{ type: "text", text: "Denied by the user." }],
          truncated: false,
        },
      ],
    };
    const body = messageListBodySchema.parse({ items: [message, results], total: 2 });
    expect(body.items.map((item) => item.parts[0]?.type)).toEqual(["text", "toolResult"]);
    const withState = { ...message, providerState: { secret: true } };
    expect(
      messageListBodySchema.parse({ items: [withState], total: 1 }).items[0]
    ).not.toHaveProperty("providerState");
  });

  test("search hits", () => {
    const body = searchListBodySchema.parse({
      items: [{ chatId: ID, chatTitle: "Cron help", messageId: null, snippet: "cron rules" }],
      total: 1,
    });
    expect(body.items[0]?.messageId).toBeNull();
  });
});

describe("requests", () => {
  test("a message without files sends none", () => {
    expect(sendMessageRequestSchema.parse({ text: "Hello" })).toEqual({
      text: "Hello",
      fileIds: [],
    });
  });

  test("moving a chat out of its project sends null", () => {
    expect(updateChatRequestSchema.parse({ projectId: null })).toEqual({ projectId: null });
  });

  test("decisions name the call and default their scope on the server", () => {
    const request = decisionsRequestSchema.parse({
      decisions: [
        { callId: ID, decision: "approve", scope: "chat" },
        { callId: ID, decision: "deny", reason: "Not now." },
      ],
    });
    expect(request.decisions[1]?.scope).toBeUndefined();
    expect(decisionsRequestSchema.safeParse({ decisions: [{ callId: ID }] }).success).toBe(false);
  });
});
