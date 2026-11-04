import { expect, it, vi } from "vitest";
import {
  mergeCmaEvents,
  normalizeCmaEvent,
  orderCmaMessages,
  readCmaStream,
} from "./cma-events";

it("orders committed history by durable sequence and deduplicates only by identity", () => {
  const messages = orderCmaMessages([
    {
      id: null,
      sequence: 4,
      role: "assistant",
      content: "same text",
      created_at: null,
    },
    {
      id: null,
      sequence: 2,
      role: "user",
      content: "same text",
      created_at: null,
    },
    {
      id: null,
      sequence: 4,
      role: "assistant",
      content: "same text",
      created_at: null,
    },
  ]);
  expect(messages.map((message) => message.sequence)).toEqual([2, 4]);
});
it("keeps Agent and Response event identities independent across replay", () => {
  const agent = normalizeCmaEvent(
    {
      agent_id: "agt",
      cursor: "cursor-one",
      sequence: 1,
      kind: { type: "tool_started", title: "Browser", status: "in_progress" },
    },
    { agentId: "agt" },
  )!;
  const response = normalizeCmaEvent(
    {
      type: "response.output_item.done",
      sequence_number: 1,
      item: {
        id: "msg",
        type: "message",
        role: "assistant",
        content: [{ type: "output_text", text: "Done" }],
      },
    },
    { agentId: "agt", responseId: "rsp" },
    "1",
  )!;
  expect(mergeCmaEvents([agent, response], [agent, response])).toHaveLength(2);
  expect(response.message).toBe("Done");
});
it("rejects mismatched Agent or Response binding and uncommitted deltas", () => {
  expect(
    normalizeCmaEvent(
      { agent_id: "other", cursor: "x", kind: { type: "deleted" } },
      { agentId: "agt" },
    ),
  ).toBeNull();
  expect(
    normalizeCmaEvent(
      {
        type: "response.completed",
        response: { id: "other", conversation: { id: "agt" } },
      },
      { agentId: "agt", responseId: "rsp" },
    ),
  ).toBeNull();
  expect(
    normalizeCmaEvent(
      {
        type: "response.output_text.delta",
        delta: "not committed",
        sequence_number: 1,
      },
      { agentId: "agt", responseId: "rsp" },
    ),
  ).toBeNull();
});
it("reads split UTF-8 and CRLF SSE frames with a separate event cursor", async () => {
  const bytes = new TextEncoder().encode(
    'id: opaque-cursor\r\nevent: cma.agent.event\r\ndata: {"message":"你好"}\r\n\r\n',
  );
  const response = new Response(
    new ReadableStream({
      start(controller) {
        for (const byte of bytes) controller.enqueue(new Uint8Array([byte]));
        controller.close();
      },
    }),
  );
  const frames = [];
  for await (const frame of readCmaStream(response)) frames.push(frame);
  expect(frames).toEqual([
    {
      id: "opaque-cursor",
      event: "cma.agent.event",
      data: { message: "你好" },
    },
  ]);
});
it("reports nonempty transport activity without turning comments into events", async () => {
  const encoder = new TextEncoder();
  const onActivity = vi.fn();
  const response = new Response(
    new ReadableStream({
      start(controller) {
        for (const chunk of [
          new Uint8Array(),
          encoder.encode(": keep-"),
          encoder.encode("alive\n\n"),
          new Uint8Array(),
          encoder.encode('id: committed\ndata: {"message":"Done"}\n\n'),
        ])
          controller.enqueue(chunk);
        controller.close();
      },
    }),
  );
  const frames = [];
  for await (const frame of readCmaStream(response, onActivity))
    frames.push(frame);
  expect(onActivity).toHaveBeenCalledTimes(3);
  expect(frames).toEqual([
    { id: "committed", event: undefined, data: { message: "Done" } },
  ]);
});
it("rejects an oversized event before buffering an unbounded response", async () => {
  const response = new Response(`data: ${"x".repeat(128 * 1024)}\n\n`);
  await expect(async () => {
    for await (const frame of readCmaStream(response)) void frame;
  }).rejects.toThrow(/limit/i);
});
