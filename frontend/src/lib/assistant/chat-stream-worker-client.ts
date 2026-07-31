import { ChatStreamParser } from "@/lib/assistant/chat-stream-parser";
import {
  CHAT_STREAM_MAX_ERROR_BODY_CHARS,
  type ChatStreamFrame,
  type ChatStreamWorkerCommand,
  type ChatStreamWorkerMessage,
} from "@/lib/assistant/chat-stream-worker-protocol";

export type ChatStreamHeadersResult =
  | {
      readonly kind: "response";
      readonly status: number;
      readonly contentType: string;
      readonly debugUpstream?: string;
    }
  | {
      readonly kind: "http_error";
      readonly status: number;
      readonly body: string;
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
  readonly removeAbortListener: () => void;
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
    const worker = this.getWorker();
    return worker
      ? this.startWorkerRequest(worker, request)
      : this.startInline(request);
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
    request: ChatStreamRequest,
  ): ChatStreamRequestHandle {
    const requestId = crypto.randomUUID();
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
        // The local request still settles below. A failed/terminated worker
        // will be replaced on the next stream start.
      }
      this.settleRequest(requestId, { kind: "cancelled" });
    };
    const onAbort = () => cancel();
    request.signal.addEventListener("abort", onAbort, { once: true });
    this.pending.set(requestId, {
      headers,
      completion,
      onFrames: request.onFrames,
      removeAbortListener: () =>
        request.signal.removeEventListener("abort", onAbort),
      settled: false,
    });

    if (request.signal.aborted) {
      cancel();
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
    return { headers: headers.promise, completion: completion.promise, cancel };
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
          debugUpstream: message.debugUpstream,
        });
        return;
      case "stream.batch":
        this.deliverFrames(message.requestId, request, message.frames);
        return;
      case "stream.complete":
        if (!this.deliverFrames(message.requestId, request, message.frames))
          return;
        this.settleRequest(message.requestId, { kind: "complete" });
        return;
      case "stream.http_error":
        this.settleRequest(message.requestId, {
          kind: "http_error",
          status: message.status,
          body: message.body,
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
    for (const requestId of [...this.pending.keys()]) {
      this.settleRequest(requestId, failure);
    }
  }

  private startInline(request: ChatStreamRequest): ChatStreamRequestHandle {
    const headers = deferred<ChatStreamHeadersResult>();
    const completion = deferred<ChatStreamCompletionResult>();
    const controller = new AbortController();
    const cancel = () => controller.abort();
    const onAbort = () => cancel();
    request.signal.addEventListener("abort", onAbort, { once: true });
    if (request.signal.aborted) cancel();

    void (async () => {
      let headersSettled = false;
      let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
      const settle = (result: ChatStreamCompletionResult) => {
        request.signal.removeEventListener("abort", onAbort);
        if (!headersSettled && result.kind !== "complete")
          headers.resolve(result);
        completion.resolve(result);
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
        if (!response.ok) {
          const body = (await response.text().catch(() => "")).slice(
            0,
            CHAT_STREAM_MAX_ERROR_BODY_CHARS,
          );
          settle({
            kind: "http_error",
            status: response.status,
            body,
            debugUpstream:
              response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
          });
          return;
        }
        headersSettled = true;
        headers.resolve({
          kind: "response",
          status: response.status,
          contentType: response.headers.get("content-type") ?? "",
          debugUpstream:
            response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
        });
        if (!response.body) {
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
        }
        const finalFrames = parser.finish();
        if (!(await deliverFrames(finalFrames))) return;
        settle({ kind: "complete" });
      } catch {
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

    return { headers: headers.promise, completion: completion.promise, cancel };
  }
}

export const chatStreamClient = new ChatStreamWorkerClient();
