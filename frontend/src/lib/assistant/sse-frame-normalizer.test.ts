import { describe, expect, it } from "vitest";
import { AGUIEventType } from "@/lib/assistant/agui-types";
import {
  normalizeBackendSseFrame,
  parseBackendSSEStream,
} from "./sse-frame-normalizer";

describe("sseFrameNormalizer", () => {
  it("normalizes tool call frames emitted in backend oneof format", () => {
    expect(
      normalizeBackendSseFrame({
        timestamp: 1,
        toolCallStart: {
          toolCallId: "tool-1",
          toolName: "knowledge.search",
        },
      }),
    ).toEqual({
      timestamp: 1,
      toolCallId: "tool-1",
      toolName: "knowledge.search",
      type: AGUIEventType.TOOL_CALL_START,
    });
    expect(
      normalizeBackendSseFrame({
        timestamp: 2,
        toolCallEnd: {
          result: "3 matches",
          toolCallId: "tool-1",
        },
      }),
    ).toEqual({
      result: "3 matches",
      timestamp: 2,
      toolCallId: "tool-1",
      type: AGUIEventType.TOOL_CALL_END,
    });
  });

  it("flattens typed tool approval frames that keep payload in a nested object", () => {
    expect(
      normalizeBackendSseFrame({
        timestamp: 3,
        toolApprovalRequest: {
          argumentsJson: '{"scopeId":"scope-a"}',
          isDestructive: true,
          requestId: "approval-1",
          timeoutSeconds: 30,
          toolCallId: "tool-7",
          toolName: "scope.bind",
        },
        type: "TOOL_APPROVAL_REQUEST",
      }),
    ).toEqual({
      argumentsJson: '{"scopeId":"scope-a"}',
      isDestructive: true,
      requestId: "approval-1",
      timeoutSeconds: 30,
      timestamp: 3,
      toolCallId: "tool-7",
      toolName: "scope.bind",
      type: "TOOL_APPROVAL_REQUEST",
    });
  });

  it("keeps final assistant text from textMessageEnd frames", () => {
    expect(
      normalizeBackendSseFrame({
        textMessageEnd: {
          delta: "final delta",
          message: "final message",
          messageId: "msg-1",
        },
        timestamp: 4,
      }),
    ).toEqual({
      delta: "final delta",
      message: "final message",
      messageId: "msg-1",
      timestamp: 4,
      type: AGUIEventType.TEXT_MESSAGE_END,
    });
  });

  it("normalizes typed actor mediaContent oneof frames", () => {
    expect(
      normalizeBackendSseFrame({
        mediaContent: {
          dataBase64: "aGVsbG8=",
          kind: "image",
          mediaType: "image/png",
          name: "chart.png",
          uri: "https://example.test/chart.png",
        },
        timestamp: 5,
        type: "MEDIA_CONTENT",
      }),
    ).toEqual({
      dataBase64: "aGVsbG8=",
      kind: "image",
      mediaType: "image/png",
      name: "chart.png",
      text: "",
      timestamp: 5,
      type: AGUIEventType.MEDIA_CONTENT,
      uri: "https://example.test/chart.png",
    });
  });

  it("extracts run identifiers from nested backend frames", () => {
    expect(
      normalizeBackendSseFrame({
        runStarted: {
          actorId: "actor-1",
          commandId: "cmd-1",
          correlationId: "corr-1",
          runId: "run-1",
        },
        timestamp: 5,
      }),
    ).toEqual({
      actorId: "actor-1",
      commandId: "cmd-1",
      correlationId: "corr-1",
      runId: "run-1",
      threadId: "actor-1",
      timestamp: 5,
      type: AGUIEventType.RUN_STARTED,
    });

    expect(
      normalizeBackendSseFrame({
        runFinished: {
          command_id: "cmd-2",
          correlation_id: "corr-2",
          result: {
            output: "complete",
          },
          runId: "run-2",
          threadId: "actor-2",
        },
        timestamp: 6,
      }),
    ).toEqual({
      commandId: "cmd-2",
      correlationId: "corr-2",
      result: {
        output: "complete",
      },
      runId: "run-2",
      threadId: "actor-2",
      timestamp: 6,
      type: AGUIEventType.RUN_FINISHED,
    });
  });

  it("extracts run identifiers and error code from flat typed backend frames", () => {
    expect(
      normalizeBackendSseFrame({
        code: "ERR_RUNTIME",
        commandId: "cmd-1",
        correlationId: "corr-1",
        message: "failed",
        runId: "run-1",
        timestamp: 7,
        type: AGUIEventType.RUN_ERROR,
      }),
    ).toEqual({
      code: "ERR_RUNTIME",
      commandId: "cmd-1",
      correlationId: "corr-1",
      message: "failed",
      runId: "run-1",
      timestamp: 7,
      type: AGUIEventType.RUN_ERROR,
    });
  });
});

const encoder = new TextEncoder();

function chunkedResponse(chunks: readonly string[]): Response {
  return new Response(
    new ReadableStream<Uint8Array>({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
        controller.close();
      },
    }),
    { status: 200, headers: { "Content-Type": "text/event-stream" } },
  );
}

async function collect(response: Response) {
  const events = [];
  for await (const event of parseBackendSSEStream(response)) events.push(event);
  return events;
}

describe("parseBackendSSEStream", () => {
  it.each([
    ["LF", "\n"],
    ["CRLF", "\r\n"],
    ["CR", "\r"],
  ])("frames %s line endings", async (_label, ending) => {
    const first = JSON.stringify({
      textMessageContent: { messageId: "message-1", delta: "hello" },
    });
    const second = JSON.stringify({ runFinished: { runId: "run-1" } });
    await expect(
      collect(
        chunkedResponse([
          `data: ${first}${ending}${ending}data: ${second}${ending}${ending}`,
        ]),
      ),
    ).resolves.toMatchObject([
      { type: AGUIEventType.TEXT_MESSAGE_CONTENT, delta: "hello" },
      { type: AGUIEventType.RUN_FINISHED, runId: "run-1" },
    ]);
  });

  it("holds a trailing CR when CRLF is split across chunks", async () => {
    const frame = JSON.stringify({
      textMessageEnd: { messageId: "message-1", message: "complete" },
    });
    await expect(
      collect(chunkedResponse([`data: ${frame}\r`, "\n\r", "\n"])),
    ).resolves.toMatchObject([
      { type: AGUIEventType.TEXT_MESSAGE_END, message: "complete" },
    ]);
  });

  it("joins data lines and skips comments, malformed frames, and DONE", async () => {
    await expect(
      collect(
        chunkedResponse([
          ': keepalive\n\ndata: {"textMessageEnd":\n',
          'data: {"messageId":"message-1","message":"joined"}}\n\n',
          "data: {not-json}\n\n",
          "data: [DONE]\n\n",
        ]),
      ),
    ).resolves.toMatchObject([
      { type: AGUIEventType.TEXT_MESSAGE_END, message: "joined" },
    ]);
  });

  it("flushes an unterminated final event", async () => {
    const frame = JSON.stringify({ runStopped: { runId: "run-1" } });
    await expect(
      collect(chunkedResponse([`data: ${frame}`])),
    ).resolves.toMatchObject([
      { type: AGUIEventType.RUN_STOPPED, runId: "run-1" },
    ]);
  });
});
