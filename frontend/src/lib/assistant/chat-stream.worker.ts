import { ChatStreamParser } from "@/lib/assistant/chat-stream-parser";
import {
  CHAT_STREAM_BATCH_INTERVAL_MS,
  CHAT_STREAM_MAX_BATCH_FRAMES,
  CHAT_STREAM_MAX_BODY_BYTES,
  CHAT_STREAM_MAX_ERROR_BODY_CHARS,
  CHAT_STREAM_MAX_WIRE_BATCH_BYTES,
  CHAT_STREAM_MAX_WIRE_BYTES,
  isTerminalChatStreamFrame,
  type ChatStreamFrame,
  type ChatStreamWireLineFragment,
  type ChatStreamWireOutcome,
  type ChatStreamWorkerCommand,
  type ChatStreamWorkerMessage,
} from "@/lib/assistant/chat-stream-worker-protocol";
import {
  isSseMediaType,
  WireBodyCapture,
} from "@/lib/assistant/wire-body-capture";
import { WireLineFramer } from "@/lib/assistant/wire-line-framer";

type WorkerWireCapture =
  | { readonly kind: "sse"; readonly framer: WireLineFramer }
  | { readonly kind: "body"; readonly capture: WireBodyCapture };

interface ActiveWorkerRequest {
  readonly controller: AbortController;
  readonly wireEnabled: boolean;
  frames: ChatStreamFrame[];
  timer: ReturnType<typeof setTimeout> | null;
  wireCapture: WorkerWireCapture | null;
  wireEnded: boolean;
  cancelOutcome: Extract<
    ChatStreamWireOutcome,
    "cancelled" | "protocol_cancel"
  >;
}

interface WorkerScope {
  onmessage: ((event: MessageEvent<ChatStreamWorkerCommand>) => void) | null;
  postMessage(message: ChatStreamWorkerMessage): void;
}

const workerScope = self as unknown as WorkerScope;
const activeRequests = new Map<string, ActiveWorkerRequest>();
const textEncoder = new TextEncoder();
const WIRE_MESSAGE_OVERHEAD_BYTES = 256;
const WIRE_FRAGMENT_TEXT_MAX_BYTES =
  CHAT_STREAM_MAX_WIRE_BATCH_BYTES - WIRE_MESSAGE_OVERHEAD_BYTES * 2;

function post(message: ChatStreamWorkerMessage): void {
  workerScope.postMessage(message);
}

function hasWireGate(headers: Readonly<Record<string, string>> | undefined) {
  return Object.entries(headers ?? {}).some(
    ([name, value]) =>
      name.toLowerCase() === "x-nyxid-debug-upstream" && value === "1",
  );
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

function splitFragmentText(text: string): string[] {
  if (text.length === 0) return [""];
  const parts: string[] = [];
  let part = "";
  let partBytes = 0;
  for (const character of text) {
    const characterBytes = textEncoder.encode(character).byteLength;
    if (partBytes + characterBytes > WIRE_FRAGMENT_TEXT_MAX_BYTES) {
      parts.push(part);
      part = "";
      partBytes = 0;
    }
    part += character;
    partBytes += characterBytes;
  }
  if (part.length > 0) parts.push(part);
  return parts;
}

function fragmentsForLines(
  lines: readonly {
    readonly text: string;
    readonly ending: "\n" | "\r\n" | "\r" | "";
  }[],
): ChatStreamWireLineFragment[] {
  const fragments: ChatStreamWireLineFragment[] = [];
  for (const line of lines) {
    const parts = splitFragmentText(line.text);
    for (let index = 0; index < parts.length; index += 1) {
      const final = index === parts.length - 1;
      fragments.push({
        text: parts[index] ?? "",
        ...(final ? { ending: line.ending } : {}),
        fragment: !final,
      });
    }
  }
  return fragments;
}

function fragmentBytes(fragment: ChatStreamWireLineFragment): number {
  return textEncoder.encode(JSON.stringify(fragment)).byteLength;
}

function postWireLines(
  requestId: string,
  lines: readonly {
    readonly text: string;
    readonly ending: "\n" | "\r\n" | "\r" | "";
  }[],
  bytes: number,
  truncated: boolean,
): void {
  const fragments = fragmentsForLines(lines);
  let batch: ChatStreamWireLineFragment[] = [];
  let batchBytes = WIRE_MESSAGE_OVERHEAD_BYTES;
  let unreportedBytes = bytes;
  const flush = () => {
    if (batch.length === 0 && unreportedBytes === 0) return;
    post({
      type: "stream.wire_batch",
      requestId,
      fragments: batch,
      bytes: unreportedBytes,
      truncated,
    });
    batch = [];
    batchBytes = WIRE_MESSAGE_OVERHEAD_BYTES;
    unreportedBytes = 0;
  };
  for (const fragment of fragments) {
    const nextBytes = fragmentBytes(fragment);
    if (
      batch.length > 0 &&
      batchBytes + nextBytes > CHAT_STREAM_MAX_WIRE_BATCH_BYTES
    ) {
      flush();
    }
    batch.push(fragment);
    batchBytes += nextBytes;
  }
  flush();
}

function beginWireCapture(
  request: ActiveWorkerRequest,
  contentType: string,
): void {
  if (!request.wireEnabled) return;
  request.wireCapture = isSseMediaType(contentType)
    ? { kind: "sse", framer: new WireLineFramer(CHAT_STREAM_MAX_WIRE_BYTES) }
    : {
        kind: "body",
        capture: new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES),
      };
}

function captureWireChunk(
  requestId: string,
  request: ActiveWorkerRequest,
  value: Uint8Array,
): void {
  const capture = request.wireCapture;
  if (!capture) return;
  if (capture.kind === "body") {
    capture.capture.push(value);
    return;
  }
  const result = capture.framer.push(value);
  postWireLines(requestId, result.lines, result.bytes, result.truncated);
}

function finishWire(
  requestId: string,
  request: ActiveWorkerRequest,
  outcome: ChatStreamWireOutcome,
): void {
  if (request.wireEnded || !request.wireCapture) return;
  request.wireEnded = true;
  if (request.wireCapture.kind === "sse") {
    const result = request.wireCapture.framer.finish();
    postWireLines(requestId, result.lines, result.bytes, result.truncated);
  } else {
    const result = request.wireCapture.capture.finish();
    post({ type: "stream.wire_body", requestId, ...result });
  }
  post({ type: "stream.wire_end", requestId, outcome });
}

async function captureErrorBody(
  requestId: string,
  request: ActiveWorkerRequest,
  response: Response,
): Promise<{ readonly text: string; readonly readFailed: boolean }> {
  const capture = new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES);
  request.wireCapture = { kind: "body", capture };
  if (!response.body) {
    finishWire(requestId, request, "complete");
    return { text: "", readFailed: false };
  }
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
  const result = capture.finish();
  finishWire(requestId, request, readFailed ? "network_error" : "complete");
  return { text: result.text, readFailed };
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
    const contentType = response.headers.get("content-type") ?? "";

    if (!response.ok) {
      if (request.wireEnabled) {
        const captured = await captureErrorBody(requestId, request, response);
        post({
          type: "stream.http_error",
          requestId,
          status: response.status,
          body: captured.text,
          debugUpstreamId:
            response.headers.get("x-nyxid-debug-upstream-id") ?? undefined,
          debugUpstream:
            response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
        });
      } else {
        const body = (await response.text().catch(() => "")).slice(
          0,
          CHAT_STREAM_MAX_ERROR_BODY_CHARS,
        );
        post({
          type: "stream.http_error",
          requestId,
          status: response.status,
          body,
          debugUpstreamId:
            response.headers.get("x-nyxid-debug-upstream-id") ?? undefined,
          debugUpstream:
            response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
        });
      }
      return;
    }

    post({
      type: "stream.response",
      requestId,
      status: response.status,
      contentType,
      debugUpstreamId:
        response.headers.get("x-nyxid-debug-upstream-id") ?? undefined,
      debugUpstream:
        response.headers.get("x-nyxid-debug-upstream-log") ?? undefined,
    });
    beginWireCapture(request, contentType);

    if (!response.body) {
      if (request.wireEnabled && !request.wireCapture) {
        request.wireCapture = {
          kind: "body",
          capture: new WireBodyCapture(CHAT_STREAM_MAX_BODY_BYTES),
        };
      }
      finishWire(requestId, request, "complete");
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
      captureWireChunk(requestId, request, value);
    }
    completeRequest(requestId, request, parser.finish());
    finishWire(requestId, request, "complete");
  } catch {
    const outcome = request.controller.signal.aborted
      ? request.cancelOutcome
      : "network_error";
    finishWire(requestId, request, outcome);
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
      request.cancelOutcome = "cancelled";
      request.controller.abort();
    }
    return;
  }

  const previous = activeRequests.get(command.requestId);
  if (previous) {
    previous.cancelOutcome = "protocol_cancel";
    previous.controller.abort();
  }
  const request: ActiveWorkerRequest = {
    controller: new AbortController(),
    wireEnabled: hasWireGate(command.headers),
    frames: [],
    timer: null,
    wireCapture: null,
    wireEnded: false,
    cancelOutcome: "cancelled",
  };
  activeRequests.set(command.requestId, request);
  void runRequest(command, request);
};
