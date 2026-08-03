import { afterEach, describe, expect, it, vi } from "vitest";
import { ChatStreamWorkerClient } from "./chat-stream-worker-client";
import type {
  ChatStreamWorkerCommand,
  ChatStreamWorkerMessage,
} from "./chat-stream-worker-protocol";

class FakeWorker {
  onmessage: ((event: MessageEvent<ChatStreamWorkerMessage>) => void) | null =
    null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  onmessageerror: ((event: MessageEvent) => void) | null = null;
  readonly messages: ChatStreamWorkerCommand[] = [];
  readonly terminate = vi.fn();

  postMessage(message: ChatStreamWorkerCommand): void {
    this.messages.push(message);
  }

  emit(message: ChatStreamWorkerMessage): void {
    this.onmessage?.({
      data: message,
    } as MessageEvent<ChatStreamWorkerMessage>);
  }

  crash(): void {
    this.onerror?.({ preventDefault: vi.fn() } as unknown as ErrorEvent);
  }
}

function asWorker(worker: FakeWorker): Worker {
  return worker as unknown as Worker;
}

function requestId(worker: FakeWorker): string {
  const start = worker.messages.find(
    (message) => message.type === "stream.start",
  );
  if (!start) throw new Error("Worker did not receive stream.start");
  return start.requestId;
}

describe("ChatStreamWorkerClient", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("delivers ordered frame batches and completion from one worker request", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const controller = new AbortController();
    const delivered: unknown[] = [];

    const stream = client.start({
      url: "/api/v1/assistant/conversations/chat-1/stream",
      bodyText: '{"prompt":"hello"}',
      signal: controller.signal,
      onFrames: (frames) => delivered.push(...frames),
    });
    const id = requestId(worker);
    worker.emit({
      type: "stream.response",
      requestId: id,
      status: 200,
      contentType: "text/event-stream",
    });
    worker.emit({
      type: "stream.batch",
      requestId: id,
      frames: [{ type: "RUN_STARTED" }, { type: "TEXT_MESSAGE_START" }],
    });
    worker.emit({
      type: "stream.complete",
      requestId: id,
      frames: [{ type: "RUN_FINISHED" }],
    });

    await expect(stream.headers).resolves.toEqual({
      kind: "response",
      status: 200,
      contentType: "text/event-stream",
    });
    await expect(stream.completion).resolves.toEqual({ kind: "complete" });
    expect(delivered).toEqual([
      { type: "RUN_STARTED" },
      { type: "TEXT_MESSAGE_START" },
      { type: "RUN_FINISHED" },
    ]);
  });

  it("settles capture-off cancellation without waiting for a worker acknowledgement", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const controller = new AbortController();
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: controller.signal,
      onFrames: () => {},
    });
    const id = requestId(worker);

    controller.abort();

    expect(worker.messages.at(-1)).toEqual({
      type: "stream.cancel",
      requestId: id,
    });
    await expect(stream.headers).resolves.toEqual({ kind: "cancelled" });
    await expect(stream.completion).resolves.toEqual({ kind: "cancelled" });
  });

  it("retains capture-on cancellation until flushed wire data is acknowledged", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const controller = new AbortController();
    const wireEvents: unknown[] = [];
    const flushedLine = 'data: {"type":"TEXT_MESSAGE_CONTENT"}';
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: controller.signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => wireEvents.push(event),
    });
    const id = requestId(worker);
    worker.emit({
      type: "stream.response",
      requestId: id,
      status: 200,
      contentType: "text/event-stream",
    });

    controller.abort();

    let completionSettled = false;
    void stream.completion.then(() => {
      completionSettled = true;
    });
    await Promise.resolve();
    expect(completionSettled).toBe(false);

    worker.emit({
      type: "stream.wire_batch",
      requestId: id,
      fragments: [{ text: flushedLine, ending: "\n", fragment: false }],
      bytes: 38,
      truncated: false,
    });
    worker.emit({
      type: "stream.wire_end",
      requestId: id,
      outcome: "cancelled",
    });
    await Promise.resolve();
    expect(completionSettled).toBe(false);

    worker.emit({ type: "stream.cancelled", requestId: id });

    await expect(stream.completion).resolves.toEqual({ kind: "cancelled" });
    expect(wireEvents).toEqual([
      {
        type: "lines",
        requestId: id,
        lines: [
          {
            text: flushedLine,
            ending: "\n",
          },
        ],
        bytes: 38,
        truncated: false,
      },
      { type: "end", requestId: id, outcome: "cancelled" },
    ]);
  });

  it("surfaces debug metadata from worker HTTP errors", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: () => {},
    });
    const id = requestId(worker);

    worker.emit({
      type: "stream.http_error",
      requestId: id,
      status: 401,
      body: '{"message":"unauthorized"}',
      debugUpstream: "encoded-error-envelope-array",
    });

    const expected = {
      kind: "http_error",
      status: 401,
      body: '{"message":"unauthorized"}',
      debugUpstream: "encoded-error-envelope-array",
    } as const;
    await expect(stream.headers).resolves.toEqual(expected);
    await expect(stream.completion).resolves.toEqual(expected);
  });

  it("falls back inline after a worker fails before its first message", async () => {
    const worker = new FakeWorker();
    const factory = vi
      .fn<() => Worker | null>()
      .mockReturnValue(asWorker(worker));
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response('data: {"type":"RUN_FINISHED"}\n\n', {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
      ),
    );
    const client = new ChatStreamWorkerClient(factory);
    const firstStream = client.start({
      url: "/stream/worker",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: () => {},
    });

    worker.crash();
    await expect(firstStream.completion).resolves.toMatchObject({
      kind: "network_error",
      code: "worker_error",
    });

    const delivered: unknown[] = [];
    const fallbackStream = client.start({
      url: "/stream/inline",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: (frames) => delivered.push(...frames),
    });
    await expect(fallbackStream.completion).resolves.toEqual({
      kind: "complete",
    });
    expect(factory).toHaveBeenCalledOnce();
    expect(delivered).toEqual([{ type: "RUN_FINISHED" }]);
  });

  it("creates a fresh worker after an established worker crashes", async () => {
    const first = new FakeWorker();
    const second = new FakeWorker();
    const factory = vi
      .fn<() => Worker | null>()
      .mockReturnValueOnce(asWorker(first))
      .mockReturnValueOnce(asWorker(second));
    const client = new ChatStreamWorkerClient(factory);
    const firstController = new AbortController();
    const firstStream = client.start({
      url: "/stream/one",
      bodyText: "{}",
      signal: firstController.signal,
      onFrames: () => {},
    });

    first.emit({
      type: "stream.response",
      requestId: requestId(first),
      status: 200,
      contentType: "text/event-stream",
    });
    first.crash();

    await expect(firstStream.completion).resolves.toMatchObject({
      kind: "network_error",
      code: "worker_error",
    });
    expect(first.terminate).toHaveBeenCalledOnce();

    const secondController = new AbortController();
    const secondStream = client.start({
      url: "/stream/two",
      bodyText: "{}",
      signal: secondController.signal,
      onFrames: () => {},
    });
    expect(factory).toHaveBeenCalledTimes(2);
    expect(second.messages[0]).toMatchObject({
      type: "stream.start",
      url: "/stream/two",
    });
    // A duplicate late error from the retired worker must not tear down its
    // replacement or settle the replacement's requests.
    first.crash();
    expect(second.terminate).not.toHaveBeenCalled();
    secondController.abort();
    const secondId = requestId(second);
    second.emit({
      type: "stream.wire_end",
      requestId: secondId,
      outcome: "cancelled",
    });
    second.emit({ type: "stream.cancelled", requestId: secondId });
    await expect(secondStream.completion).resolves.toEqual({
      kind: "cancelled",
    });
  });

  it("cancels the inline reader when frame delivery throws", async () => {
    const cancel = vi.fn();
    const encoder = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode('data: {"type":"RUN_STARTED"}\n\n'));
      },
      cancel,
    });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(body, {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
      ),
    );
    const client = new ChatStreamWorkerClient(() => null);
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: () => {
        throw new Error("adapter failed");
      },
    });

    await expect(stream.completion).resolves.toMatchObject({
      kind: "network_error",
      code: "worker_error",
    });
    expect(cancel).toHaveBeenCalledOnce();
  });

  it("sends optional headers and surfaces debug metadata outside frame batches", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response('data: {"type":"RUN_FINISHED"}\n\n', {
        status: 200,
        headers: {
          "Content-Type": "text/event-stream",
          "X-NyxID-Debug-Upstream-Log": "encoded-envelope-array",
        },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const delivered: unknown[] = [];
    const client = new ChatStreamWorkerClient(() => null);

    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: (frames) => delivered.push(...frames),
    });

    await expect(stream.headers).resolves.toEqual({
      kind: "response",
      status: 200,
      contentType: "text/event-stream",
      debugUpstream: "encoded-envelope-array",
    });
    await expect(stream.completion).resolves.toEqual({ kind: "complete" });
    expect(delivered).toEqual([{ type: "RUN_FINISHED" }]);
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit;
    expect(init.headers).toMatchObject({ "X-NyxID-Debug-Upstream": "1" });
  });

  it("surfaces debug metadata from inline HTTP errors", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response('{"message":"unauthorized"}', {
          status: 401,
          headers: {
            "X-NyxID-Debug-Upstream-Log": "encoded-error-envelope-array",
          },
        }),
      ),
    );
    const client = new ChatStreamWorkerClient(() => null);

    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: () => {},
    });

    const expected = {
      kind: "http_error",
      status: 401,
      body: '{"message":"unauthorized"}',
      debugUpstream: "encoded-error-envelope-array",
    } as const;
    await expect(stream.headers).resolves.toEqual(expected);
    await expect(stream.completion).resolves.toEqual(expected);
  });

  it("preserves capture-off character truncation and drops partially read HTTP errors inline", async () => {
    const multibyteBody = "界".repeat(40_000);
    const brokenBody = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("partial"));
        controller.error(new Error("body read failed"));
      },
    });
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValueOnce(new Response(multibyteBody, { status: 502 }))
        .mockResolvedValueOnce(new Response(brokenBody, { status: 503 })),
    );
    const client = new ChatStreamWorkerClient(() => null);

    const multibyte = client.start({
      url: "/multibyte-error",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: () => {},
    });
    await expect(multibyte.completion).resolves.toMatchObject({
      kind: "http_error",
      body: multibyteBody,
    });

    const broken = client.start({
      url: "/broken-error",
      bodyText: "{}",
      signal: new AbortController().signal,
      onFrames: () => {},
    });
    await expect(broken.completion).resolves.toMatchObject({
      kind: "http_error",
      body: "",
    });
  });

  it("reassembles wire fragments and waits for wire end before deleting pending state", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const wireEvents: unknown[] = [];
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => wireEvents.push(event),
    });
    const id = requestId(worker);
    worker.emit({
      type: "stream.response",
      requestId: id,
      status: 200,
      contentType: "text/event-stream",
    });
    worker.emit({
      type: "stream.wire_batch",
      requestId: id,
      fragments: [{ text: "data: long-", fragment: true }],
      bytes: 12,
      truncated: false,
    });
    worker.emit({
      type: "stream.wire_batch",
      requestId: id,
      fragments: [{ text: "line", ending: "\r\n", fragment: false }],
      bytes: 4,
      truncated: false,
    });
    worker.emit({ type: "stream.complete", requestId: id, frames: [] });
    let completionSettled = false;
    void stream.completion.then(() => {
      completionSettled = true;
    });
    await Promise.resolve();
    expect(completionSettled).toBe(false);

    worker.emit({
      type: "stream.wire_end",
      requestId: id,
      outcome: "complete",
    });

    await expect(stream.completion).resolves.toEqual({ kind: "complete" });
    expect(wireEvents).toEqual([
      {
        type: "lines",
        requestId: id,
        lines: [],
        bytes: 12,
        truncated: false,
      },
      {
        type: "lines",
        requestId: id,
        lines: [{ text: "data: long-line", ending: "\r\n" }],
        bytes: 4,
        truncated: false,
      },
      { type: "end", requestId: id, outcome: "complete" },
    ]);
  });

  it("isolates a throwing wire consumer from worker frame delivery", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const delivered: unknown[] = [];
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: (frames) => delivered.push(...frames),
      onWire: () => {
        throw new Error("diagnostic consumer failed");
      },
    });
    const id = requestId(worker);
    worker.emit({
      type: "stream.response",
      requestId: id,
      status: 200,
      contentType: "text/event-stream",
    });
    worker.emit({
      type: "stream.wire_batch",
      requestId: id,
      fragments: [{ text: "data: raw", ending: "\n", fragment: false }],
      bytes: 10,
      truncated: false,
    });
    worker.emit({
      type: "stream.batch",
      requestId: id,
      frames: [{ type: "RUN_STARTED" }],
    });
    worker.emit({ type: "stream.complete", requestId: id, frames: [] });
    worker.emit({
      type: "stream.wire_end",
      requestId: id,
      outcome: "complete",
    });

    await expect(stream.completion).resolves.toEqual({ kind: "complete" });
    expect(delivered).toEqual([{ type: "RUN_STARTED" }]);
  });

  it("settles open wire captures as worker_error when the worker crashes", async () => {
    const worker = new FakeWorker();
    const client = new ChatStreamWorkerClient(() => asWorker(worker));
    const wireEvents: unknown[] = [];
    const stream = client.start({
      url: "/stream",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => wireEvents.push(event),
    });
    const id = requestId(worker);
    worker.emit({
      type: "stream.response",
      requestId: id,
      status: 200,
      contentType: "text/event-stream",
    });

    worker.crash();

    await expect(stream.completion).resolves.toMatchObject({
      kind: "network_error",
      code: "worker_error",
    });
    expect(wireEvents.at(-1)).toEqual({
      type: "end",
      requestId: id,
      outcome: "worker_error",
    });
  });

  it("mirrors SSE and non-SSE wire capture in the inline fallback", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(': ping\ndata: {"type":"RUN_FINISHED"}\n\n', {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
      )
      .mockResolvedValueOnce(
        new Response('{"ok":true}', {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    vi.stubGlobal("fetch", fetchMock);
    const client = new ChatStreamWorkerClient(() => null);
    const sseWire: unknown[] = [];
    const sse = client.start({
      url: "/sse",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => sseWire.push(event),
    });
    await expect(sse.completion).resolves.toEqual({ kind: "complete" });
    expect(sseWire).toEqual([
      {
        type: "lines",
        requestId: expect.any(String),
        lines: [
          { text: ": ping", ending: "\n" },
          { text: 'data: {"type":"RUN_FINISHED"}', ending: "\n" },
          { text: "", ending: "\n" },
        ],
        bytes: 38,
        truncated: false,
      },
      {
        type: "end",
        requestId: expect.any(String),
        outcome: "complete",
      },
    ]);

    const bodyWire: unknown[] = [];
    const json = client.start({
      url: "/json",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => bodyWire.push(event),
    });
    await expect(json.completion).resolves.toEqual({ kind: "complete" });
    expect(bodyWire).toEqual([
      {
        type: "body",
        requestId: expect.any(String),
        text: '{"ok":true}',
        bytes: 11,
        truncated: false,
      },
      {
        type: "end",
        requestId: expect.any(String),
        outcome: "complete",
      },
    ]);
  });

  it("captures inline HTTP errors and 204 responses without double-reading", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response("failure", {
          status: 500,
          headers: { "X-NyxID-Debug-Upstream-Log": "encoded" },
        }),
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    const client = new ChatStreamWorkerClient(() => null);
    const errorWire: unknown[] = [];
    const error = client.start({
      url: "/error",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => errorWire.push(event),
    });
    await expect(error.completion).resolves.toMatchObject({
      kind: "http_error",
      body: "failure",
    });
    expect(errorWire).toEqual([
      {
        type: "body",
        requestId: expect.any(String),
        text: "failure",
        bytes: 7,
        truncated: false,
      },
      {
        type: "end",
        requestId: expect.any(String),
        outcome: "complete",
      },
    ]);

    const emptyWire: unknown[] = [];
    const empty = client.start({
      url: "/empty",
      bodyText: "{}",
      signal: new AbortController().signal,
      headers: { "X-NyxID-Debug-Upstream": "1" },
      onFrames: () => {},
      onWire: (event) => emptyWire.push(event),
    });
    await expect(empty.completion).resolves.toMatchObject({
      kind: "network_error",
      code: "stream_closed",
    });
    expect(emptyWire).toEqual([
      {
        type: "body",
        requestId: expect.any(String),
        text: "",
        bytes: 0,
        truncated: false,
      },
      {
        type: "end",
        requestId: expect.any(String),
        outcome: "complete",
      },
    ]);
  });
});
