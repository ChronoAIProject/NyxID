import assert from "node:assert/strict";
import test from "node:test";
import {
  runAssistantLive,
  type AssistantLiveEvent,
  type AssistantLiveStatus,
} from "./assistantLiveTransport";

const encoder = new TextEncoder();
const frame = (event: object) => `data: ${JSON.stringify(event)}\n\n`;

test("a notification frame reaches subscribers with its identifier only, and unknown frames are ignored", async () => {
  const events: AssistantLiveEvent[] = [];
  const statuses: AssistantLiveStatus[] = [];
  const controller = new AbortController();
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(encoder.encode(frame({ type: "ready" })));
      stream.enqueue(encoder.encode(frame({ type: "notification", id: "n-1" })));
      stream.enqueue(encoder.encode(frame({ type: "notification" })));
      stream.enqueue(encoder.encode(frame({ type: "mystery", id: "x" })));
      stream.enqueue(encoder.encode(frame({ type: "resync" })));
    },
  });
  const run = runAssistantLive({
    connect: async () => ({
      headers: { get: (name: string) => (name === "content-type" ? "text/event-stream" : null) },
      body,
    }),
    signal: controller.signal,
    onStatus: (status) => statuses.push(status),
    onEvent: (event) => {
      events.push(event);
      if (event.type === "resync") controller.abort();
    },
  });
  await run;
  assert.deepEqual(events, [
    { type: "ready" },
    { type: "notification", id: "n-1" },
    { type: "resync" },
  ]);
  assert.ok(statuses.includes("live"));
});
