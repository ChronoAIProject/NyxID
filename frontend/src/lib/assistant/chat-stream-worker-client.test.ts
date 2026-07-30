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

  it("cancels the matching worker request when its AbortSignal fires", async () => {
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
});
