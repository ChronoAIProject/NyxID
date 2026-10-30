import assert from "node:assert/strict";
import test from "node:test";
import { readAssistantResponse } from "./assistantStreamReader";
import type { AssistantTurnEvent } from "./assistantTypes";

const conversationId = `nyxa-${"a".repeat(32)}`;
const encoder = new TextEncoder();
const frame = (event: AssistantTurnEvent) =>
  encoder.encode(`data: ${JSON.stringify(event)}\n\n`);
const start: AssistantTurnEvent = {
  event: "turn.status",
  cursor: 1,
  conversation_id: conversationId,
  turn_id: "turn-1",
  status: "running",
};
const terminal: AssistantTurnEvent = {
  event: "turn.completed",
  cursor: 3,
  turn_id: "turn-1",
  status: "completed",
  error: null,
};

test("terminal SSE waits for native completion without canceling the body", async () => {
  let cancelled = 0;
  let closeError: unknown;
  let nativeCompleted = false;
  const events: AssistantTurnEvent[] = [];
  const controller = new AbortController();
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(frame(start));
      stream.enqueue(frame(terminal));
      // Expo emits this separately from the terminal data. Canceling its
      // reader first makes this queued controller.close() throw on native.
      setImmediate(() => {
        try {
          stream.close();
          nativeCompleted = true;
        } catch (cause) {
          closeError = cause;
        }
      });
    },
    cancel() {
      cancelled++;
    },
  });
  await readAssistantResponse(body, (event) => events.push(event), controller);
  assert.deepEqual(events, [start, terminal]);
  assert.equal(nativeCompleted, true);
  assert.equal(closeError, undefined);
  assert.equal(cancelled, 0);
  assert.equal(controller.signal.aborted, false);
  assert.equal(body.locked, false);
});

test("split UTF-8 deltas arrive before EOF and post-terminal frames are ignored", async () => {
  let finish: (() => void) | undefined;
  const events: AssistantTurnEvent[] = [];
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(frame(start));
      const delta = frame({
        event: "block.delta",
        cursor: 2,
        block_id: "block-1",
        text: "你好",
      });
      for (const byte of delta) stream.enqueue(new Uint8Array([byte]));
      finish = () => {
        stream.enqueue(frame(terminal));
        stream.enqueue(
          frame({
            event: "turn.notice",
            cursor: 4,
            code: "ignored-after-terminal",
            message: "Ignored trailing frame",
          }),
        );
        stream.close();
      };
    },
  });
  const reading = readAssistantResponse(
    body,
    (event) => events.push(event),
    new AbortController(),
  );
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(events.length, 2);
  assert.equal(events[1]?.event, "block.delta");
  const delta = events[1];
  assert.ok(delta && "text" in delta);
  assert.equal(delta.text, "你好");
  finish!();
  await reading;
  assert.deepEqual(
    events.map((event) => event.event),
    ["turn.status", "block.delta", "turn.completed"],
  );
});

test("backend Stop's cancelled terminal drains without a local request abort", async () => {
  const events: AssistantTurnEvent[] = [];
  const controller = new AbortController();
  let nativeCompleted = false;
  const cancelled: AssistantTurnEvent = {
    ...terminal,
    status: "cancelled",
    error: { code: "cancelled", message: "Turn cancelled." },
  };
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(frame(start));
      stream.enqueue(frame(cancelled));
      setImmediate(() => {
        stream.close();
        nativeCompleted = true;
      });
    },
  });
  await readAssistantResponse(body, (event) => events.push(event), controller);
  assert.deepEqual(events, [start, cancelled]);
  assert.equal(nativeCompleted, true);
  assert.equal(controller.signal.aborted, false);
  assert.equal(body.locked, false);
});

test("a terminal transport error does not abort an already errored native response", async () => {
  const controller = new AbortController();
  const events: AssistantTurnEvent[] = [];
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(frame(start));
      stream.enqueue(frame(terminal));
      setImmediate(() => stream.error(new Error("Native cleanup failed")));
    },
  });
  await readAssistantResponse(body, (event) => events.push(event), controller);
  assert.deepEqual(events, [start, terminal]);
  assert.equal(controller.signal.aborted, false);
  assert.equal(body.locked, false);
});

test("truncated EOF remains uncertain without a second cancel", async () => {
  let cancelled = 0;
  const controller = new AbortController();
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(frame(start));
      stream.close();
    },
    cancel() {
      cancelled++;
    },
  });
  await assert.rejects(
    readAssistantResponse(body, () => undefined, controller),
    /may still be working/,
  );
  assert.equal(cancelled, 0);
  assert.equal(controller.signal.aborted, false);
  assert.equal(body.locked, false);
});

test("request abort ends unfinished reading without also canceling the body", async () => {
  let cancelled = 0;
  let errors = 0;
  const controller = new AbortController();
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(frame(start));
      controller.signal.addEventListener("abort", () => {
        errors++;
        stream.error(new Error("Native request aborted"));
      });
    },
    cancel() {
      cancelled++;
    },
  });
  await assert.rejects(
    readAssistantResponse(body, () => controller.abort(), controller),
    /Native request aborted/,
  );
  assert.equal(errors, 1);
  assert.equal(cancelled, 0);
  assert.equal(body.locked, false);
});

test("invalid SSE aborts its request once and releases the reader", async () => {
  let aborted = 0;
  let cancelled = 0;
  const controller = new AbortController();
  const body = new ReadableStream<Uint8Array>({
    start(stream) {
      stream.enqueue(
        encoder.encode(
          'data: {"event":"turn.status","cursor":1,"conversation_id":"wrong"}\n\n',
        ),
      );
      controller.signal.addEventListener("abort", () => {
        aborted++;
        stream.error(new Error("Native request aborted"));
      });
    },
    cancel() {
      cancelled++;
    },
  });
  await assert.rejects(
    readAssistantResponse(body, () => undefined, controller),
  );
  assert.equal(aborted, 1);
  assert.equal(cancelled, 0);
  assert.equal(body.locked, false);
});
