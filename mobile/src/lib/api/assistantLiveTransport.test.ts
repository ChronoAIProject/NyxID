import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "./ApiError";
import {
  ASSISTANT_LIVE_MAX_CHUNK_BYTES,
  ASSISTANT_LIVE_MAX_FRAME_CHARS,
  livePollInterval,
  runAssistantLive,
  type AssistantLiveClock,
  type AssistantLiveEvent,
  type AssistantLiveStatus,
} from "./assistantLiveTransport";

const encoder = new TextEncoder();
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));
const frame = (event: object) => `data: ${JSON.stringify(event)}\n\n`;

class FakeClock implements AssistantLiveClock {
  private time = 0;
  private sequence = 0;
  readonly jobs = new Map<unknown, { at: number; callback: () => void }>();
  now = () => this.time;
  setTimeout = (callback: () => void, milliseconds: number): unknown => {
    const id = ++this.sequence;
    this.jobs.set(id, { at: this.time + milliseconds, callback });
    return id;
  };
  clearTimeout = (handle: unknown) => {
    this.jobs.delete(handle);
  };
  nextDelay() {
    const next = Math.min(...[...this.jobs.values()].map((job) => job.at));
    return Number.isFinite(next) ? next - this.time : undefined;
  }
  async advance(milliseconds: number) {
    const target = this.time + milliseconds;
    for (;;) {
      const entry = [...this.jobs].sort(([, a], [, b]) => a.at - b.at)[0];
      if (!entry || entry[1].at > target) break;
      this.time = entry[1].at;
      this.jobs.delete(entry[0]);
      entry[1].callback();
      await flush();
    }
    this.time = target;
    await flush();
  }
}

function stream(
  signal: AbortSignal,
  chunks: readonly Uint8Array[] = [],
  options: {
    close?: boolean;
    ignoreAbort?: boolean;
    contentType?: string;
  } = {},
) {
  let source!: ReadableStreamDefaultController<Uint8Array>;
  let ended = false;
  let cancelled = 0;
  let aborted = 0;
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      source = controller;
      chunks.forEach((chunk) => controller.enqueue(chunk));
      if (options.close) {
        ended = true;
        controller.close();
      }
    },
    cancel() {
      cancelled++;
    },
  });
  signal.addEventListener(
    "abort",
    () => {
      aborted++;
      if (!ended && !options.ignoreAbort) {
        ended = true;
        source.error(new Error("Native request aborted"));
      }
    },
    { once: true },
  );
  return {
    response: {
      headers: {
        get: () => options.contentType ?? "text/event-stream; charset=utf-8",
      },
      body,
    },
    body,
    push: (text: string) => source.enqueue(encoder.encode(text)),
    close: () => {
      ended = true;
      source.close();
    },
    fail: () => {
      ended = true;
      source.error(new Error("Connection lost"));
    },
    cancelled: () => cancelled,
    aborted: () => aborted,
  };
}
type Connection = ReturnType<typeof stream>["response"];
function start(connect: (signal: AbortSignal) => Promise<Connection>) {
  const controller = new AbortController();
  const clock = new FakeClock();
  const statuses: AssistantLiveStatus[] = [];
  const events: AssistantLiveEvent[] = [];
  const running = runAssistantLive({
    connect,
    signal: controller.signal,
    clock,
    onStatus: (status) => statuses.push(status),
    onEvent: (event) => events.push(event),
  });
  return {
    controller,
    clock,
    statuses,
    events,
    running,
    stop: async () => {
      controller.abort();
      await running;
      assert.equal(clock.jobs.size, 0);
    },
  };
}

test("chunked SSE handles UTF-8, comments, CRLF, multiline data and only known valid events", async () => {
  const input =
    ": heartbeat\r\nevent: ignored\r\ndata: not-json\r\n\r\n" +
    'data: {"type":"future"}\r\n\r\n' +
    'data: {"type":"conversation","id":"bad","messages":-1}\r\n\r\n' +
    frame({ type: "ready" }) +
    'data: {"type":"conversation",\r\ndata: "id":"你好","group_id":"nyxg-1"}\r\n\r\n' +
    'data: {"type":"group","id":"nyxg-1"}\r\r' +
    frame({ type: "channels" }) +
    frame({
      type: "channel_thread",
      id: "thread",
      channel_id: "channel",
      parent_id: null,
      conversation_id: null,
    }) +
    frame({ type: "resync" });
  let live!: ReturnType<typeof stream>;
  const run = start(async (signal) => {
    live = stream(
      signal,
      [...encoder.encode(input)].map((byte) => Uint8Array.of(byte)),
    );
    return live.response;
  });
  await flush();
  assert.deepEqual(run.statuses, ["connecting", "live"]);
  assert.deepEqual(run.events, [
    { type: "ready" },
    {
      type: "conversation",
      id: "你好",
      group_id: "nyxg-1",
      turn_id: null,
      messages: 0,
    },
    { type: "group", id: "nyxg-1" },
    { type: "channels" },
    {
      type: "channel_thread",
      id: "thread",
      channel_id: "channel",
      parent_id: null,
      conversation_id: null,
    },
    { type: "resync" },
  ]);
  await run.stop();
  assert.equal(live.cancelled(), 0);
  assert.equal(live.aborted(), 1);
  assert.equal(live.body.locked, false);
});

test("only ready enables live; normal EOF immediately restores polling then reopens in 250 ms", async () => {
  let calls = 0;
  const sources: ReturnType<typeof stream>[] = [];
  const run = start(async (signal) => {
    const source = stream(signal, [encoder.encode(frame({ type: "ready" }))], {
      close: calls++ === 0,
    });
    sources.push(source);
    return source.response;
  });
  await flush();
  assert.deepEqual(run.statuses, ["connecting", "live", "connecting"]);
  assert.equal(run.clock.nextDelay(), 250);
  assert.equal(sources[0]?.aborted(), 0);
  assert.equal(sources[0]?.body.locked, false);
  await run.clock.advance(249);
  assert.equal(calls, 1);
  await run.clock.advance(1);
  assert.equal(calls, 2);
  assert.equal(run.statuses.at(-1), "live");
  await run.stop();
});

test("unsupported HTTP statusCode and wrong content type fall back for five minutes", async () => {
  for (const statusCode of [404, 405, 501, 200]) {
    let calls = 0;
    let rejected: ReturnType<typeof stream> | undefined;
    const run = start(async (signal) => {
      calls++;
      if (calls === 1 && statusCode !== 200)
        throw new ApiError({
          statusCode,
          errorKey: "unsupported",
          errorCode: 0,
          message: "Unsupported",
        });
      if (calls === 1) {
        rejected = stream(signal, [], { contentType: "application/json" });
        return rejected.response;
      }
      return stream(signal, [encoder.encode(frame({ type: "ready" }))])
        .response;
    });
    await flush();
    assert.deepEqual(run.statuses, ["connecting", "unsupported"]);
    assert.equal(run.clock.nextDelay(), 300_000);
    await run.clock.advance(299_999);
    assert.equal(calls, 1);
    await run.clock.advance(1);
    assert.equal(calls, 2);
    assert.equal(run.statuses.at(-1), "live");
    assert.equal(rejected?.cancelled() ?? 0, 0);
    await run.stop();
  }
});

test("transient failures use capped backoff and receiving ready resets it", async () => {
  let calls = 0;
  let recovered!: ReturnType<typeof stream>;
  const run = start(async (signal) => {
    if (++calls <= 6) throw new Error("Network unavailable");
    recovered = stream(signal, [encoder.encode(frame({ type: "ready" }))]);
    return recovered.response;
  });
  await flush();
  for (const delay of [1000, 2000, 5000, 10_000, 30_000, 30_000]) {
    assert.equal(run.clock.nextDelay(), delay);
    await run.clock.advance(delay);
  }
  assert.equal(calls, 7);
  assert.equal(run.statuses.at(-1), "live");
  recovered.fail();
  await flush();
  assert.equal(run.statuses.at(-1), "connecting");
  assert.equal(run.clock.nextDelay(), 1000);
  await run.stop();
});

test("EOF without ready backs off and does not slow fallback polling", async () => {
  const run = start(
    async (signal) =>
      stream(signal, [encoder.encode(frame({ type: "group", id: "group" }))], {
        close: true,
      }).response,
  );
  await flush();
  assert.deepEqual(run.statuses, ["connecting"]);
  assert.equal(run.clock.nextDelay(), 1000);
  assert.equal(livePollInterval(1500, false), 1500);
  assert.equal(livePollInterval(1500, true), 30_000);
  assert.equal(livePollInterval(60_000, true), 60_000);
  assert.equal(livePollInterval(false, true), false);
  await run.stop();
});

test("header deadline aborts a connect that ignores cancellation without leaking timers", async () => {
  let requestSignal!: AbortSignal;
  let resolve!: (response: Connection) => void;
  const run = start((signal) => {
    requestSignal = signal;
    return new Promise((yes) => {
      resolve = yes;
    });
  });
  await flush();
  assert.equal(run.clock.nextDelay(), 15_000);
  await run.clock.advance(15_000);
  assert.equal(requestSignal.aborted, true);
  assert.equal(run.clock.nextDelay(), 1000);
  await run.stop();
  const before = [...run.statuses];
  resolve(
    stream(requestSignal, [encoder.encode(frame({ type: "ready" }))], {
      close: true,
    }).response,
  );
  await flush();
  assert.deepEqual(run.events, []);
  assert.deepEqual(run.statuses, before);
});

test("ready deadline is absolute even when comments continue arriving", async () => {
  let source!: ReturnType<typeof stream>;
  const run = start(async (signal) => {
    source = stream(signal);
    return source.response;
  });
  await flush();
  await run.clock.advance(10_000);
  source.push(": heartbeat\n\n");
  await flush();
  await run.clock.advance(5000);
  assert.equal(source.aborted(), 1);
  assert.equal(source.cancelled(), 0);
  assert.equal(source.body.locked, false);
  assert.deepEqual(run.statuses, ["connecting"]);
  assert.equal(run.clock.nextDelay(), 1000);
  await run.stop();
});

test("heartbeats extend a ready stream but a silent stream times out and resumes polling", async () => {
  let source!: ReturnType<typeof stream>;
  const run = start(async (signal) => {
    source = stream(signal, [encoder.encode(frame({ type: "ready" }))]);
    return source.response;
  });
  await flush();
  assert.equal(run.clock.nextDelay(), 45_000);
  await run.clock.advance(30_000);
  source.push(": heartbeat\r\n\r\n");
  await flush();
  assert.equal(run.clock.nextDelay(), 45_000);
  await run.clock.advance(45_000);
  assert.deepEqual(run.statuses, ["connecting", "live", "connecting"]);
  assert.equal(source.aborted(), 1);
  assert.equal(source.cancelled(), 0);
  assert.equal(source.body.locked, false);
  assert.equal(run.clock.nextDelay(), 1000);
  await run.stop();
});

test("oversized frames/chunks and endless empty chunks are bounded without reader.cancel", async () => {
  const oversizedFrame = encoder.encode(
    `data: ${"x".repeat(ASSISTANT_LIVE_MAX_FRAME_CHARS + 1)}`,
  );
  const frameChunks: Uint8Array[] = [];
  for (let i = 0; i < oversizedFrame.length; i += 8192)
    frameChunks.push(oversizedFrame.slice(i, i + 8192));
  for (const chunks of [
    frameChunks,
    [new Uint8Array(ASSISTANT_LIVE_MAX_CHUNK_BYTES + 1)],
    Array.from({ length: 65 }, () => new Uint8Array()),
  ]) {
    let source!: ReturnType<typeof stream>;
    const run = start(async (signal) => {
      source = stream(signal, chunks);
      return source.response;
    });
    await flush();
    assert.equal(source.aborted(), 1);
    assert.equal(source.cancelled(), 0);
    assert.equal(source.body.locked, false);
    assert.equal(run.clock.nextDelay(), 1000);
    await run.stop();
  }
});

test("outer abort stops a stuck reader and blocks late events without canceling its body", async () => {
  let source!: ReturnType<typeof stream>;
  const run = start(async (signal) => {
    source = stream(signal, [], { ignoreAbort: true });
    return source.response;
  });
  await flush();
  await run.stop();
  assert.equal(source.aborted(), 1);
  assert.equal(source.cancelled(), 0);
  assert.equal(source.body.locked, false);
  source.push(frame({ type: "ready" }));
  source.close();
  await flush();
  assert.deepEqual(run.events, []);
  assert.deepEqual(run.statuses, ["connecting"]);
});

test("aborting while delivering a frame blocks remaining events in the same chunk", async () => {
  const controller = new AbortController();
  const clock = new FakeClock();
  const events: AssistantLiveEvent[] = [];
  await runAssistantLive({
    connect: async (signal) =>
      stream(signal, [
        encoder.encode(
          frame({ type: "ready" }) + frame({ type: "group", id: "late" }),
        ),
      ]).response,
    signal: controller.signal,
    clock,
    onStatus: () => undefined,
    onEvent: (event) => {
      events.push(event);
      controller.abort();
    },
  });
  assert.deepEqual(events, [{ type: "ready" }]);
  assert.equal(clock.jobs.size, 0);
});
