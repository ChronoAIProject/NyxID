import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ChatStreamWorkerCommand,
  ChatStreamWorkerMessage,
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

afterEach(() => {
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
    expect((fetchMock.mock.calls[0]?.[1] as RequestInit).headers).toMatchObject({
      "X-NyxID-Debug-Upstream": "1",
    });

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
});
