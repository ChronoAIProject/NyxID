import { ChatStreamParser } from "@/lib/assistant/chat-stream-parser";
import {
  CHAT_STREAM_MAX_BODY_BYTES,
  CHAT_STREAM_MAX_ERROR_BODY_CHARS,
  CHAT_STREAM_MAX_WIRE_BYTES,
  type ChatStreamFrame,
  type ChatStreamWireEvent,
  type ChatStreamWireOutcome,
  type ChatStreamWorkerCommand,
  type ChatStreamWorkerMessage,
} from "@/lib/assistant/chat-stream-worker-protocol";
import {
  isSseMediaType,
  WireBodyCapture,
} from "@/lib/assistant/wire-body-capture";
import { WireLineFramer } from "@/lib/assistant/wire-line-framer";

export type ChatStreamHeadersResult =
  | {
      readonly kind: "response";
      readonly status: number;
      readonly contentType: string;
      readonly debugUpstreamId?: string;
      readonly debugUpstream?: string;
    }
  | {
      readonly kind: "http_error";
      readonly status: number;
      readonly body: string;
      readonly debugUpstreamId?: string;
      readonly debugUpstream?: string;
    }
  | {
      readonly kind: "network_error";
      readonly code: "network_error" | "stream_closed" | "worker_error";
      readonly message: string;
    }
  | { readonly kind: "cancelled" };

export type ChatStreamCompletionResult =
  | { readonly kind: "complete" }
  | Exclude<ChatStreamHeadersResult, { kind: "response" }>;

export interface ChatStreamRequest {
  readonly url: string;
  readonly bodyText: string;
  readonly signal: AbortSignal;
  readonly headers?: Readonly<Record<string, string>>;
  readonly onFrames: (frames: readonly ChatStreamFrame[]) => void;
  readonly onWire?: (event: ChatStreamWireEvent) => void;
}

export interface ChatStreamRequestHandle {
  readonly headers: Promise<ChatStreamHeadersResult>;
  readonly completion: Promise<ChatStreamCompletionResult>;
  cancel(): void;
}

interface Deferred<T> {
  readonly promise: Promise<T>;
  readonly resolve: (value: T) => void;
}

interface PendingRequest {
  readonly headers: Deferred<ChatStreamHeadersResult>;
  readonly completion: Deferred<ChatStreamCompletionResult>;
  readonly onFrames: (frames: readonly ChatStreamFrame[]) => void;
  readonly onWire?: (event: ChatStreamWireEvent) => void;
  readonly removeAbortListener: () => void;
  readonly wireExpected: boolean;
  wireFragment: string;
  wireEnded: boolean;
  completionReceived: boolean;
  settled: boolean;
}

type WorkerFactory = () => Worker | null;

function deferred<T>(): Deferred<T> {
  let resolvePromise: ((value: T) => void) | undefined;
  const promise = new Promise<T>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: (value) => resolvePromise?.(value) };
}

function createModuleWorker(): Worker | null {
  if (typeof Worker === "undefined" || import.meta.env.MODE === "test") {
    return null;
  }
  try {
    return new Worker(new URL("./chat-stream.worker.ts", import.meta.url), {
      type: "module",
      name: "nyxid-assistant-stream",
    });
  } catch {
    return null;
  }
}

function networkFailure(
  code: "network_error" | "stream_closed" | "worker_error",
  message: string,
): Extract<ChatStreamHeadersResult, { kind: "network_error" }> {
  return { kind: "network_error", code, message };
}

function hasWireGate(headers: Readonly<Record<string, string>> | undefined) {
  return Object.entries(headers ?? {}).some(
    ([name, value]) =>
      name.toLowerCase() === "x-nyxid-debug-upstream" && value === "1",
  );
}

async function readBoundedBody(response: Response): Promise<{
  readonly result: ReturnType<WireBodyCapture["finish"]>;
  readonly readFailed: boolean;
}> {
  const capture = new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES);
  if (!response.body) return { result: capture.finish(), readFailed: false };
  const reader = response.body.getReader();
  let readFailed = false;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      capture.push(value);
      if (capture.truncated) {
        await reader.cancel().catch(() => undefined);
        break;
      }
    }
  } catch {
    readFailed = true;
  }
  return { result: capture.finish(), readFailed };
}

export class ChatStreamWorkerClient {
  private worker: Worker | null = null;
  private workerReady = false;
  private workerUnavailable = false;
  private readonly pending = new Map<string, PendingRequest>();
  private readonly workerFactory: WorkerFactory;

  constructor(workerFactory: WorkerFactory = createModuleWorker) {
    this.workerFactory = workerFactory;
  }

  start(request: ChatStreamRequest): ChatStreamRequestHandle {
    const requestId = crypto.randomUUID();
    const worker = this.getWorker();
    return worker
      ? this.startWorkerRequest(worker, requestId, request)
      : this.startInline(requestId, request);
  }

  private getWorker(): Worker | null {
    if (this.workerUnavailable) return null;
    if (this.worker) return this.worker;
    const worker = this.workerFactory();
    if (!worker) return null;
    worker.onmessage = (event: MessageEvent<ChatStreamWorkerMessage>) => {
      if (this.worker !== worker) return;
      this.workerReady = true;
      this.handleWorkerMessage(event.data);
    };
    worker.onerror = (event) => {
      event.preventDefault();
      this.failWorker(worker);
    };
    worker.onmessageerror = () => {
      this.failWorker(worker);
    };
    this.workerReady = false;
    this.worker = worker;
    return worker;
  }

  private startWorkerRequest(
    worker: Worker,
    requestId: string,
    request: ChatStreamRequest,
  ): ChatStreamRequestHandle {
    const headers = deferred<ChatStreamHeadersResult>();
    const completion = deferred<ChatStreamCompletionResult>();
    const cancel = () => {
      const pending = this.pending.get(requestId);
      if (!pending || pending.settled) return;
      try {
        worker.postMessage({
          type: "stream.cancel",
          requestId,
        } satisfies ChatStreamWorkerCommand);
      } catch {
        this.deliverWire(requestId, pending, {
          type: "end",
          requestId,
          outcome: "cancelled",
        });
        this.settleRequest(requestId, { kind: "cancelled" });
        return;
      }
      if (!pending.wireExpected) {
        this.settleRequest(requestId, { kind: "cancelled" });
      }
    };
    const onAbort = () => cancel();
    request.signal.addEventListener("abort", onAbort, { once: true });
    this.pending.set(requestId, {
      headers,
      completion,
      onFrames: request.onFrames,
      onWire: request.onWire,
      removeAbortListener: () =>
        request.signal.removeEventListener("abort", onAbort),
      wireExpected: hasWireGate(request.headers),
      wireFragment: "",
      wireEnded: false,
      completionReceived: false,
      settled: false,
    });

    if (request.signal.aborted) {
      this.settleRequest(requestId, { kind: "cancelled" });
    } else {
      try {
        worker.postMessage({
          type: "stream.start",
          requestId,
          url: request.url,
          bodyText: request.bodyText,
          headers: request.headers,
        } satisfies ChatStreamWorkerCommand);
      } catch {
        this.failWorker(worker);
      }
    }
    return {
      headers: headers.promise,
      completion: completion.promise,
      cancel,
    };
  }

  private handleWorkerMessage(message: ChatStreamWorkerMessage): void {
    const request = this.pending.get(message.requestId);
    if (!request || request.settled) return;
    switch (message.type) {
      case "stream.response":
        request.headers.resolve({
          kind: "response",
          status: message.status,
          contentType: message.contentType,
          debugUpstreamId: message.debugUpstreamId,
          debugUpstream: message.debugUpstream,
        });
        return;
      case "stream.batch":
        this.deliverFrames(message.requestId, request, message.frames);
        return;
      case "stream.wire_batch":
        this.deliverWireBatch(message.requestId, request, message);
        return;
      case "stream.wire_body":
        this.deliverWire(message.requestId, request, {
          type: "body",
          requestId: message.requestId,
          text: message.text,
          bytes: message.bytes,
          truncated: message.truncated,
        });
        return;
      case "stream.wire_end":
        request.wireFragment = "";
        request.wireEnded = true;
        this.deliverWire(message.requestId, request, {
          type: "end",
          requestId: message.requestId,
          outcome: message.outcome,
        });
        if (request.completionReceived) {
          this.settleRequest(message.requestId, { kind: "complete" });
        }
        return;
      case "stream.complete":
        if (!this.deliverFrames(message.requestId, request, message.frames))
          return;
        request.completionReceived = true;
        if (!request.wireExpected || request.wireEnded) {
          this.settleRequest(message.requestId, { kind: "complete" });
        }
        return;
      case "stream.http_error":
        this.settleRequest(message.requestId, {
          kind: "http_error",
          status: message.status,
          body: message.body,
          debugUpstreamId: message.debugUpstreamId,
          debugUpstream: message.debugUpstream,
        });
        return;
      case "stream.network_error":
        this.settleRequest(
          message.requestId,
          networkFailure(message.code, message.message),
        );
        return;
      case "stream.cancelled":
        this.settleRequest(message.requestId, { kind: "cancelled" });
    }
  }

  private deliverWireBatch(
    requestId: string,
    request: PendingRequest,
    message: Extract<ChatStreamWorkerMessage, { type: "stream.wire_batch" }>,
  ): void {
    const lines: Array<{
      text: string;
      ending: "\n" | "\r\n" | "\r" | "";
    }> = [];
    for (const fragment of message.fragments) {
      request.wireFragment += fragment.text;
      if (fragment.fragment) continue;
      lines.push({
        text: request.wireFragment,
        ending: fragment.ending ?? "",
      });
      request.wireFragment = "";
    }
    this.deliverWire(requestId, request, {
      type: "lines",
      requestId,
      lines,
      bytes: message.bytes,
      truncated: message.truncated,
    });
  }

  private deliverWire(
    _requestId: string,
    request: PendingRequest,
    event: ChatStreamWireEvent,
  ): void {
    try {
      request.onWire?.(event);
    } catch {
      // Capture is diagnostic-only and cannot affect the live stream.
    }
  }

  private deliverFrames(
    requestId: string,
    request: PendingRequest,
    frames: readonly ChatStreamFrame[],
  ): boolean {
    if (frames.length === 0) return true;
    try {
      request.onFrames(frames);
      return true;
    } catch {
      this.worker?.postMessage({
        type: "stream.cancel",
        requestId,
      } satisfies ChatStreamWorkerCommand);
      this.deliverWire(requestId, request, {
        type: "end",
        requestId,
        outcome: "worker_error",
      });
      this.settleRequest(
        requestId,
        networkFailure(
          "worker_error",
          "The assistant stream could not be processed. Try again.",
        ),
      );
      return false;
    }
  }

  private settleRequest(
    requestId: string,
    result: ChatStreamCompletionResult,
  ): void {
    const request = this.pending.get(requestId);
    if (!request || request.settled) return;
    request.settled = true;
    request.removeAbortListener();
    if (result.kind !== "complete") request.headers.resolve(result);
    request.completion.resolve(result);
    this.pending.delete(requestId);
  }

  private failWorker(failedWorker: Worker): void {
    if (this.worker !== failedWorker) return;
    if (!this.workerReady) this.workerUnavailable = true;
    this.worker = null;
    this.workerReady = false;
    failedWorker.terminate();
    const failure = networkFailure(
      "worker_error",
      "The assistant stream worker stopped unexpectedly. Try again.",
    );
    for (const [requestId, request] of [...this.pending.entries()]) {
      this.deliverWire(requestId, request, {
        type: "end",
        requestId,
        outcome: "worker_error",
      });
      this.settleRequest(requestId, failure);
    }
  }

  private startInline(
    requestId: string,
    request: ChatStreamRequest,
  ): ChatStreamRequestHandle {
    const headers = deferred<ChatStreamHeadersResult>();
    const completion = deferred<ChatStreamCompletionResult>();
    const controller = new AbortController();
    const wireEnabled = hasWireGate(request.headers);
    const emitWire = (event: ChatStreamWireEvent) => {
      try {
        request.onWire?.(event);
      } catch {
        // Capture is diagnostic-only and cannot affect the live stream.
      }
    };
    const cancel = () => controller.abort();
    const onAbort = () => cancel();
    request.signal.addEventListener("abort", onAbort, { once: true });
    if (request.signal.aborted) cancel();

    void (async () => {
      let headersSettled = false;
      let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
      let wireCapture:
        | { readonly kind: "sse"; readonly framer: WireLineFramer }
        | { readonly kind: "body"; readonly capture: WireBodyCapture }
        | null = null;
      let wireEnded = false;
      const settle = (result: ChatStreamCompletionResult) => {
        request.signal.removeEventListener("abort", onAbort);
        if (!headersSettled && result.kind !== "complete") {
          headers.resolve(result);
        }
        completion.resolve(result);
      };
      const emitLines = (result: ReturnType<WireLineFramer["push"]>): void => {
        if (result.lines.length === 0 && result.bytes === 0) return;
        emitWire({
          type: "lines",
          requestId,
          lines: result.lines,
          bytes: result.bytes,
          truncated: result.truncated,
        });
      };
      const finishWire = (outcome: ChatStreamWireOutcome) => {
        if (wireEnded || !wireCapture) return;
        wireEnded = true;
        if (wireCapture.kind === "sse") {
          emitLines(wireCapture.framer.finish());
        } else {
          emitWire({
            type: "body",
            requestId,
            ...wireCapture.capture.finish(),
          });
        }
        emitWire({ type: "end", requestId, outcome });
      };
      const deliverFrames = async (
        frames: readonly ChatStreamFrame[],
      ): Promise<boolean> => {
        if (frames.length === 0) return true;
        try {
          request.onFrames(frames);
          return true;
        } catch {
          await reader?.cancel().catch(() => undefined);
          finishWire("worker_error");
          settle(
            networkFailure(
              "worker_error",
              "The assistant stream could not be processed. Try again.",
            ),
          );
          return false;
        }
      };
      try {
        const response = await fetch(request.url, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Accept: "text/event-stream",
            ...request.headers,
          },
          credentials: "include",
          body: request.bodyText,
          signal: controller.signal,
        });
        const contentType = response.headers.get("content-type") ?? "";
        if (!response.ok) {
          if (wireEnabled) {
            const captured = await readBoundedBody(response);
            emitWire({
              type: "body",
              requestId,
              ...captured.result,
            });
            emitWire({
              type: "end",
              requestId,
              outcome: captured.readFailed ? "network_error" : "complete",
            });
            settle({
              kind: "http_error",
              status: response.status,
              body: captured.result.text,
              debugUpstreamId:
                response.headers.get("x-nyxid-debug-upstream-id") ?? undefined,
              debugUpstream:
                response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
            });
            return;
          }
          const body = (await response.text().catch(() => "")).slice(
            0,
            CHAT_STREAM_MAX_ERROR_BODY_CHARS,
          );
          settle({
            kind: "http_error",
            status: response.status,
            body,
            debugUpstreamId:
              response.headers.get("x-nyxid-debug-upstream-id") ?? undefined,
            debugUpstream:
              response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
          });
          return;
        }
        headersSettled = true;
        headers.resolve({
          kind: "response",
          status: response.status,
          contentType,
          debugUpstreamId:
            response.headers.get("x-nyxid-debug-upstream-id") ?? undefined,
          debugUpstream:
            response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
        });
        if (wireEnabled) {
          wireCapture = isSseMediaType(contentType)
            ? {
                kind: "sse",
                framer: new WireLineFramer(CHAT_STREAM_MAX_WIRE_BYTES),
              }
            : {
                kind: "body",
                capture: new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES),
              };
        }
        if (!response.body) {
          if (wireEnabled && !wireCapture) {
            wireCapture = {
              kind: "body",
              capture: new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES),
            };
          }
          finishWire("complete");
          settle(
            networkFailure(
              "stream_closed",
              "The assistant stream closed before it started.",
            ),
          );
          return;
        }

        const parser = new ChatStreamParser();
        reader = response.body.getReader();
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          const frames = parser.push(value);
          if (!(await deliverFrames(frames))) return;
          if (wireCapture?.kind === "sse") {
            emitLines(wireCapture.framer.push(value));
          } else if (wireCapture?.kind === "body") {
            wireCapture.capture.push(value);
          }
        }
        const finalFrames = parser.finish();
        if (!(await deliverFrames(finalFrames))) return;
        finishWire("complete");
        settle({ kind: "complete" });
      } catch {
        finishWire(controller.signal.aborted ? "cancelled" : "network_error");
        settle(
          controller.signal.aborted
            ? { kind: "cancelled" }
            : networkFailure(
                "network_error",
                "The assistant stream was interrupted. Try again.",
              ),
        );
      }
    })();

    return {
      headers: headers.promise,
      completion: completion.promise,
      cancel,
    };
  }
}

export const chatStreamClient = new ChatStreamWorkerClient();
