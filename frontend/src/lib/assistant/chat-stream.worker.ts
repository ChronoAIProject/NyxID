import { ChatStreamParser } from "@/lib/assistant/chat-stream-parser";
import {
  CHAT_STREAM_BATCH_INTERVAL_MS,
  CHAT_STREAM_MAX_BATCH_FRAMES,
  CHAT_STREAM_MAX_ERROR_BODY_CHARS,
  isTerminalChatStreamFrame,
  type ChatStreamFrame,
  type ChatStreamWorkerCommand,
  type ChatStreamWorkerMessage,
} from "@/lib/assistant/chat-stream-worker-protocol";

interface ActiveWorkerRequest {
  readonly controller: AbortController;
  frames: ChatStreamFrame[];
  timer: ReturnType<typeof setTimeout> | null;
}

interface WorkerScope {
  onmessage: ((event: MessageEvent<ChatStreamWorkerCommand>) => void) | null;
  postMessage(message: ChatStreamWorkerMessage): void;
}

const workerScope = self as unknown as WorkerScope;
const activeRequests = new Map<string, ActiveWorkerRequest>();

function post(message: ChatStreamWorkerMessage): void {
  workerScope.postMessage(message);
}

function clearBatchTimer(request: ActiveWorkerRequest): void {
  if (request.timer === null) return;
  clearTimeout(request.timer);
  request.timer = null;
}

function flushBatch(requestId: string, request: ActiveWorkerRequest): void {
  clearBatchTimer(request);
  if (request.frames.length === 0) return;
  const frames = request.frames;
  request.frames = [];
  post({ type: "stream.batch", requestId, frames });
}

function enqueueFrames(
  requestId: string,
  request: ActiveWorkerRequest,
  frames: readonly ChatStreamFrame[],
): void {
  if (frames.length === 0) return;
  for (const frame of frames) {
    request.frames.push(frame);
    if (
      request.frames.length >= CHAT_STREAM_MAX_BATCH_FRAMES ||
      isTerminalChatStreamFrame(frame)
    ) {
      flushBatch(requestId, request);
    }
  }
  if (request.frames.length === 0) return;
  request.timer ??= setTimeout(() => {
    if (activeRequests.get(requestId) === request) {
      flushBatch(requestId, request);
    }
  }, CHAT_STREAM_BATCH_INTERVAL_MS);
}

function completeRequest(
  requestId: string,
  request: ActiveWorkerRequest,
  frames: readonly ChatStreamFrame[],
): void {
  clearBatchTimer(request);
  request.frames.push(...frames);
  while (request.frames.length > CHAT_STREAM_MAX_BATCH_FRAMES) {
    post({
      type: "stream.batch",
      requestId,
      frames: request.frames.splice(0, CHAT_STREAM_MAX_BATCH_FRAMES),
    });
  }
  const finalFrames = request.frames;
  request.frames = [];
  post({ type: "stream.complete", requestId, frames: finalFrames });
}

async function runRequest(
  command: Extract<ChatStreamWorkerCommand, { type: "stream.start" }>,
  request: ActiveWorkerRequest,
): Promise<void> {
  const { requestId } = command;
  try {
    const response = await fetch(command.url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "text/event-stream",
        ...command.headers,
      },
      credentials: "include",
      body: command.bodyText,
      signal: request.controller.signal,
    });

    if (!response.ok) {
      const body = (await response.text().catch(() => "")).slice(
        0,
        CHAT_STREAM_MAX_ERROR_BODY_CHARS,
      );
      post({
        type: "stream.http_error",
        requestId,
        status: response.status,
        body,
        debugUpstream:
          response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
      });
      return;
    }

    post({
      type: "stream.response",
      requestId,
      status: response.status,
      contentType: response.headers.get("content-type") ?? "",
      debugUpstream:
        response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
    });

    if (!response.body) {
      post({
        type: "stream.network_error",
        requestId,
        code: "stream_closed",
        message: "The assistant stream closed before it started.",
      });
      return;
    }

    const parser = new ChatStreamParser();
    const reader = response.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      enqueueFrames(requestId, request, parser.push(value));
    }
    completeRequest(requestId, request, parser.finish());
  } catch {
    post(
      request.controller.signal.aborted
        ? { type: "stream.cancelled", requestId }
        : {
            type: "stream.network_error",
            requestId,
            code: "network_error",
            message: "The assistant stream was interrupted. Try again.",
          },
    );
  } finally {
    clearBatchTimer(request);
    if (activeRequests.get(requestId) === request) {
      activeRequests.delete(requestId);
    }
  }
}

workerScope.onmessage = (event) => {
  const command = event.data;
  if (command.type === "stream.cancel") {
    const request = activeRequests.get(command.requestId);
    if (request) {
      clearBatchTimer(request);
      request.controller.abort();
      activeRequests.delete(command.requestId);
    }
    return;
  }

  const previous = activeRequests.get(command.requestId);
  if (previous) {
    clearBatchTimer(previous);
    previous.controller.abort();
  }
  const request: ActiveWorkerRequest = {
    controller: new AbortController(),
    frames: [],
    timer: null,
  };
  activeRequests.set(command.requestId, request);
  void runRequest(command, request);
};
