import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ChatStreamWireLineFragment,
  ChatStreamWorkerCommand,
  ChatStreamWorkerMessage,
} from "./chat-stream-worker-protocol";
import {
  CHAT_STREAM_MAX_WIRE_BATCH_BYTES,
  CHAT_STREAM_MAX_WIRE_BYTES,
} from "./chat-stream-worker-protocol";

interface TestWorkerScope {
  onmessage: ((event: MessageEvent<ChatStreamWorkerCommand>) => void) | null;
  postMessage: ReturnType<
    typeof vi.fn<(message: ChatStreamWorkerMessage) => void>
  >;
}

async function installWorker(): Promise<TestWorkerScope> {
  const scope: TestWorkerScope = {
    onmessage: null,
    postMessage: vi.fn<(message: ChatStreamWorkerMessage) => void>(),
  };
  vi.stubGlobal("self", scope);
  vi.resetModules();
  await import("./chat-stream.worker");
  return scope;
}

function send(scope: TestWorkerScope, command: ChatStreamWorkerCommand): void {
  scope.onmessage?.({ data: command } as MessageEvent<ChatStreamWorkerCommand>);
}

function messages(scope: TestWorkerScope): ChatStreamWorkerMessage[] {
  return scope.postMessage.mock.calls.map(([message]) => message);
}

function reassembleWireLines(
  fragments: readonly ChatStreamWireLineFragment[],
): Array<{ text: string; ending: string }> {
  const lines: Array<{ text: string; ending: string }> = [];
  let text = "";
  for (const fragment of fragments) {
    text += fragment.text;
    if (fragment.fragment) continue;
    lines.push({ text, ending: fragment.ending ?? "" });
    text = "";
  }
  expect(text).toBe("");
  return lines;
}

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("chat stream worker", () => {
  it("batches ordered frames and flushes them within the bounded interval", async () => {
    const encoder = new TextEncoder();
    let push: (value: Uint8Array) => void = (value) => {
      void value;
    };
    let close: () => void = () => undefined;
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        push = (value) => controller.enqueue(value);
        close = () => controller.close();
      },
    });
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(body, {
        status: 200,
        headers: {
          "Content-Type": "text/event-stream",
          "X-NyxID-Debug-Upstream-Log": "encoded-envelope-array",
        },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-batch",
      url: "/stream",
      bodyText: '{"prompt":"hello"}',
      headers: { "X-NyxID-Debug-Upstream": "1" },
    });
    await vi.waitFor(() => {
      expect(messages(scope)[0]).toMatchObject({ type: "stream.response" });
    });
    expect(messages(scope)[0]).toMatchObject({
      type: "stream.response",
      debugUpstream: "encoded-envelope-array",
    });
    expect((fetchMock.mock.calls[0]?.[1] as RequestInit).headers).toMatchObject(
      {
        "X-NyxID-Debug-Upstream": "1",
      },
    );

    push(
      encoder.encode(
        [
          'data: {"type":"RUN_STARTED","turnId":"turn-1"}',
          "",
          'data: {"type":"TEXT_MESSAGE_START"}',
          "",
          "",
        ].join("\n"),
      ),
    );

    await vi.waitFor(() => {
      const batch = messages(scope).find(
        (message) => message.type === "stream.batch",
      );
      expect(batch?.type === "stream.batch" && batch.frames).toEqual([
        { type: "RUN_STARTED", turnId: "turn-1" },
        { type: "TEXT_MESSAGE_START" },
      ]);
    });
    push(encoder.encode('data: {"type":"RUN_FINISHED"}\n\n'));
    await vi.waitFor(() => {
      const batches = messages(scope).filter(
        (message) => message.type === "stream.batch",
      );
      expect(batches[1]).toMatchObject({
        frames: [{ type: "RUN_FINISHED" }],
      });
    });
    close();
    await vi.waitFor(() => {
      expect(
        messages(scope).some((message) => message.type === "stream.complete"),
      ).toBe(true);
    });
  });

  it("keeps each posted batch within the maximum frame count", async () => {
    const frames = Array.from({ length: 600 }, (_, index) => ({
      type: "TEXT_MESSAGE_CONTENT",
      textMessageContent: { delta: String(index) },
    }));
    const encoder = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            frames
              .map(
                (frame, index) =>
                  `data: ${JSON.stringify(frame)}${index + 1 < frames.length ? "\n\n" : ""}`,
              )
              .join(""),
          ),
        );
        controller.close();
      },
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
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-bounded",
      url: "/stream",
      bodyText: "{}",
    });

    await vi.waitFor(() => {
      expect(
        messages(scope).some((message) => message.type === "stream.complete"),
      ).toBe(true);
    });
    const frameMessages = messages(scope).filter(
      (message) =>
        message.type === "stream.batch" || message.type === "stream.complete",
    );
    expect(frameMessages.every((message) => message.frames.length <= 256)).toBe(
      true,
    );
    expect(
      frameMessages.reduce(
        (total, message) => total + message.frames.length,
        0,
      ),
    ).toBe(600);
  });

  it("preserves a final frame without an SSE separator", async () => {
    const encoder = new TextEncoder();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(
          encoder.encode(
            'data: {"type":"RUN_STARTED","turnId":"turn-2"}\n\n' +
              'data: {"type":"RUN_FINISHED"}',
          ),
        );
        controller.close();
      },
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
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-terminal",
      url: "/stream",
      bodyText: "{}",
    });

    await vi.waitFor(() => {
      const complete = messages(scope).find(
        (message) => message.type === "stream.complete",
      );
      expect(complete?.type === "stream.complete" && complete.frames).toEqual([
        { type: "RUN_STARTED", turnId: "turn-2" },
        { type: "RUN_FINISHED" },
      ]);
    });
    expect(
      messages(scope).some((message) => message.type === "stream.batch"),
    ).toBe(false);
  });

  it("returns typed HTTP and network failures", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response('{"code":"UPSTREAM_DOWN"}', {
          status: 502,
          headers: {
            "X-NyxID-Debug-Upstream-Log": "encoded-error-envelope-array",
          },
        }),
      )
      .mockRejectedValueOnce(new TypeError("connection reset"));
    vi.stubGlobal("fetch", fetchMock);
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-http",
      url: "/stream",
      bodyText: "{}",
    });
    send(scope, {
      type: "stream.start",
      requestId: "request-network",
      url: "/stream",
      bodyText: "{}",
    });

    await vi.waitFor(() => {
      expect(messages(scope)).toEqual(
        expect.arrayContaining([
          {
            type: "stream.http_error",
            requestId: "request-http",
            status: 502,
            body: '{"code":"UPSTREAM_DOWN"}',
            debugUpstream: "encoded-error-envelope-array",
          },
          {
            type: "stream.network_error",
            requestId: "request-network",
            code: "network_error",
            message: "The assistant stream was interrupted. Try again.",
          },
        ]),
      );
    });
  });

  it("preserves capture-off character truncation and drops partially read HTTP errors", async () => {
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
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-multibyte-error",
      url: "/stream",
      bodyText: "{}",
    });
    send(scope, {
      type: "stream.start",
      requestId: "request-broken-error",
      url: "/stream",
      bodyText: "{}",
    });

    await vi.waitFor(() => {
      expect(messages(scope)).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            type: "stream.http_error",
            requestId: "request-multibyte-error",
            body: multibyteBody,
          }),
          expect.objectContaining({
            type: "stream.http_error",
            requestId: "request-broken-error",
            body: "",
          }),
        ]),
      );
    });
  });

  it("aborts only the matching in-flight request", async () => {
    let capturedSignal: AbortSignal | undefined;
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: string, init?: RequestInit) => {
        capturedSignal = init?.signal ?? undefined;
        return new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(new DOMException("aborted", "AbortError"));
          });
        });
      }),
    );
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-cancel",
      url: "/stream",
      bodyText: "{}",
    });
    await vi.waitFor(() => expect(capturedSignal).toBeDefined());

    send(scope, { type: "stream.cancel", requestId: "request-cancel" });

    expect(capturedSignal?.aborted).toBe(true);
    await vi.waitFor(() => {
      expect(messages(scope)).toContainEqual({
        type: "stream.cancelled",
        requestId: "request-cancel",
      });
    });
  });

  it("clears a pending frame batch before acknowledging cancellation", async () => {
    vi.useFakeTimers();
    const encoder = new TextEncoder();
    let push: (value: Uint8Array) => void = () => undefined;
    let fail: (reason: unknown) => void = () => undefined;
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        push = (value) => controller.enqueue(value);
        fail = (reason) => controller.error(reason);
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: string, init?: RequestInit) => {
        init?.signal?.addEventListener("abort", () => {
          fail(new DOMException("aborted", "AbortError"));
        });
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          }),
        );
      }),
    );
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-cancel-timer",
      url: "/stream",
      bodyText: "{}",
      headers: { "X-NyxID-Debug-Upstream": "1" },
    });
    await Promise.resolve();
    push(
      encoder.encode(
        'data: {"type":"RUN_STARTED","turnId":"turn-cancel"}\n\n',
      ),
    );
    for (let index = 0; index < 5; index += 1) await Promise.resolve();
    expect(messages(scope)).toContainEqual(
      expect.objectContaining({
        type: "stream.wire_batch",
        requestId: "request-cancel-timer",
      }),
    );
    expect(
      messages(scope).some((message) => message.type === "stream.batch"),
    ).toBe(false);

    send(scope, { type: "stream.cancel", requestId: "request-cancel-timer" });
    await vi.advanceTimersByTimeAsync(100);

    expect(
      messages(scope).some(
        (message) =>
          message.requestId === "request-cancel-timer" &&
          message.type === "stream.batch",
      ),
    ).toBe(false);
    expect(messages(scope)).toContainEqual({
      type: "stream.cancelled",
      requestId: "request-cancel-timer",
    });
  });

  it("tees exact SSE lines only when the debug gate is present", async () => {
    const source = ': keepalive\r\ndata: {"type":"RUN_STARTED"}\n\nid: final\r';
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(source, {
          status: 200,
          headers: { "Content-Type": "text/event-stream; charset=utf-8" },
        }),
      )
      .mockResolvedValueOnce(
        new Response(source, {
          status: 200,
          headers: { "Content-Type": "text/event-stream; charset=utf-8" },
        }),
      );
    vi.stubGlobal("fetch", fetchMock);
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-gated",
      url: "/stream",
      bodyText: "{}",
      headers: { "X-NyxID-Debug-Upstream": "1" },
    });
    send(scope, {
      type: "stream.start",
      requestId: "request-ungated",
      url: "/stream",
      bodyText: "{}",
    });

    await vi.waitFor(() => {
      expect(
        messages(scope).filter((message) => message.type === "stream.complete"),
      ).toHaveLength(2);
    });
    const gated = messages(scope).filter(
      (
        message,
      ): message is Extract<
        ChatStreamWorkerMessage,
        { type: "stream.wire_batch" }
      > =>
        message.requestId === "request-gated" &&
        message.type === "stream.wire_batch",
    );
    const fragments = gated.flatMap((message) => message.fragments);
    expect(reassembleWireLines(fragments)).toEqual([
      { text: ": keepalive", ending: "\r\n" },
      { text: 'data: {"type":"RUN_STARTED"}', ending: "\n" },
      { text: "", ending: "\n" },
      { text: "id: final", ending: "\r" },
    ]);
    expect(gated.reduce((total, message) => total + message.bytes, 0)).toBe(
      new TextEncoder().encode(source).byteLength,
    );
    expect(messages(scope)).toContainEqual({
      type: "stream.wire_end",
      requestId: "request-gated",
      outcome: "complete",
    });
    expect(
      messages(scope).some(
        (message) =>
          message.requestId === "request-ungated" &&
          message.type.startsWith("stream.wire_"),
      ),
    ).toBe(false);
  });

  it("captures successful non-SSE and null-body responses as delivered bodies", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response('{"ok":true}', {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    const scope = await installWorker();
    for (const requestId of ["request-json", "request-empty"]) {
      send(scope, {
        type: "stream.start",
        requestId,
        url: "/stream",
        bodyText: "{}",
        headers: { "X-NyxID-Debug-Upstream": "1" },
      });
    }

    await vi.waitFor(() => {
      expect(
        messages(scope).filter((message) => message.type === "stream.wire_end"),
      ).toHaveLength(2);
    });
    expect(messages(scope)).toEqual(
      expect.arrayContaining([
        {
          type: "stream.wire_body",
          requestId: "request-json",
          text: '{"ok":true}',
          bytes: 11,
          truncated: false,
        },
        {
          type: "stream.wire_body",
          requestId: "request-empty",
          text: "",
          bytes: 0,
          truncated: false,
        },
        {
          type: "stream.wire_end",
          requestId: "request-empty",
          outcome: "complete",
        },
      ]),
    );
    expect(messages(scope)).toContainEqual({
      type: "stream.network_error",
      requestId: "request-empty",
      code: "stream_closed",
      message: "The assistant stream closed before it started.",
    });
    expect(
      messages(scope).some(
        (message) =>
          message.requestId === "request-empty" &&
          message.type === "stream.complete",
      ),
    ).toBe(false);
  });

  it("reads an HTTP error body once and derives both error text and capture", async () => {
    const pulls = vi.fn();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        pulls();
        controller.enqueue(new TextEncoder().encode('{"code":"DOWN"}'));
        controller.close();
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(body, {
          status: 502,
          headers: {
            "Content-Type": "application/json",
            "X-NyxID-Debug-Upstream-Log": "encoded",
          },
        }),
      ),
    );
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-error-capture",
      url: "/stream",
      bodyText: "{}",
      headers: { "X-NyxID-Debug-Upstream": "1" },
    });

    await vi.waitFor(() => {
      expect(messages(scope)).toContainEqual({
        type: "stream.http_error",
        requestId: "request-error-capture",
        status: 502,
        body: '{"code":"DOWN"}',
        debugUpstream: "encoded",
      });
    });
    expect(pulls).toHaveBeenCalledOnce();
    expect(messages(scope)).toContainEqual({
      type: "stream.wire_body",
      requestId: "request-error-capture",
      text: '{"code":"DOWN"}',
      bytes: 15,
      truncated: false,
    });
  });

  it("fragments oversized lines into byte-bounded posts and keeps truncation orthogonal", async () => {
    const oversizedLine = `data: ${"x".repeat(40_000)}\n`;
    const source = `${oversizedLine}${"#".repeat(CHAT_STREAM_MAX_WIRE_BYTES + 1)}`;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(source, {
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
        }),
      ),
    );
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-fragmented",
      url: "/stream",
      bodyText: "{}",
      headers: { "X-NyxID-Debug-Upstream": "1" },
    });

    await vi.waitFor(() => {
      expect(messages(scope)).toContainEqual({
        type: "stream.wire_end",
        requestId: "request-fragmented",
        outcome: "complete",
      });
    });
    const wireBatches = messages(scope).filter(
      (message) => message.type === "stream.wire_batch",
    );
    expect(wireBatches.length).toBeGreaterThan(1);
    expect(
      wireBatches.every(
        (message) =>
          new TextEncoder().encode(JSON.stringify(message)).byteLength <=
          CHAT_STREAM_MAX_WIRE_BATCH_BYTES,
      ),
    ).toBe(true);
    expect(wireBatches.some((message) => message.truncated)).toBe(true);
    const firstLine = reassembleWireLines(
      wireBatches.flatMap((message) => message.fragments),
    )[0];
    expect(firstLine).toEqual({
      text: oversizedLine.slice(0, -1),
      ending: "\n",
    });
  });

  it("flushes the retained tail and wire outcome before cancellation acknowledgement", async () => {
    const encoder = new TextEncoder();
    let push: (value: Uint8Array) => void = () => undefined;
    let fail: (reason: unknown) => void = () => undefined;
    let signal: AbortSignal | undefined;
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        push = (value) => controller.enqueue(value);
        fail = (reason) => controller.error(reason);
      },
    });
    vi.stubGlobal(
      "fetch",
      vi.fn((_url: string, init?: RequestInit) => {
        signal = init?.signal ?? undefined;
        signal?.addEventListener("abort", () =>
          fail(new DOMException("aborted", "AbortError")),
        );
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: { "Content-Type": "text/event-stream" },
          }),
        );
      }),
    );
    const scope = await installWorker();
    send(scope, {
      type: "stream.start",
      requestId: "request-cancel-wire",
      url: "/stream",
      bodyText: "{}",
      headers: { "X-NyxID-Debug-Upstream": "1" },
    });
    await vi.waitFor(() => expect(signal).toBeDefined());
    push(encoder.encode("data: partial"));
    send(scope, { type: "stream.cancel", requestId: "request-cancel-wire" });

    await vi.waitFor(() => {
      expect(messages(scope)).toContainEqual({
        type: "stream.cancelled",
        requestId: "request-cancel-wire",
      });
    });
    const all = messages(scope);
    const fragments = all
      .filter(
        (
          message,
        ): message is Extract<
          ChatStreamWorkerMessage,
          { type: "stream.wire_batch" }
        > =>
          message.requestId === "request-cancel-wire" &&
          message.type === "stream.wire_batch",
      )
      .flatMap((message) => message.fragments);
    expect(reassembleWireLines(fragments)).toEqual([
      { text: "data: partial", ending: "" },
    ]);
    const endIndex = all.findIndex(
      (message) =>
        message.type === "stream.wire_end" &&
        message.requestId === "request-cancel-wire",
    );
    const acknowledgementIndex = all.findIndex(
      (message) =>
        message.type === "stream.cancelled" &&
        message.requestId === "request-cancel-wire",
    );
    expect(all[endIndex]).toMatchObject({ outcome: "cancelled" });
    expect(endIndex).toBeGreaterThanOrEqual(0);
    expect(acknowledgementIndex).toBeGreaterThan(endIndex);
  });
});
