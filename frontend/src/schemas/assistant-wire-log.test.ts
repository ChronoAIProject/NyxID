import { describe, expect, it } from "vitest";
import {
  assistantTransportOutcomeSchema,
  assistantWireCaptureSchema,
  assistantWireLogExchangeSchema,
  assistantWireLogRecordSchema,
} from "./assistant-wire-log";

const minimalEcho = {
  degraded: true as const,
  method: "POST",
  path: "api/chat",
  commandType: "text",
  upstreamOutcome: "response" as const,
  status: 200,
};

describe("assistant wire-log record schema", () => {
  it("accepts metadata-first exchanges without an inline payload", () => {
    expect(
      assistantWireLogExchangeSchema.parse({
        id: "exchange-1",
        ts: 1_776_947_200_000,
        kind: "header",
        status: 200,
        conversationId: "nyxchat-1",
        wireLogId: "d7dbbf38-a31c-4331-8ddb-13fda5a70d12",
        label: "GET /assistant/conversations/nyxchat-1/state",
      }),
    ).toMatchObject({
      conversationId: "nyxchat-1",
      wireLogId: "d7dbbf38-a31c-4331-8ddb-13fda5a70d12",
      label: "GET /assistant/conversations/nyxchat-1/state",
    });
  });

  it("parses the strict fetch response with the unchanged v2 envelope", () => {
    const record = {
      id: "d7dbbf38-a31c-4331-8ddb-13fda5a70d12",
      conversation_id: null,
      created_at: "2026-08-20T12:00:00Z",
      payload: {
        version: 2 as const,
        echoes: [minimalEcho],
        droppedEchoCount: 0,
      },
    };

    expect(assistantWireLogRecordSchema.parse(record)).toEqual(record);
    expect(
      assistantWireLogRecordSchema.safeParse({ ...record, unexpected: true })
        .success,
    ).toBe(false);
    expect(
      assistantWireLogRecordSchema.safeParse({
        ...record,
        payload: { ...record.payload, version: 3 },
      }).success,
    ).toBe(false);
  });
});

describe("assistant wire-log telemetry schema", () => {
  it("accepts metadata-only wire and transport outcomes with bounded counters", () => {
    expect(
      assistantWireCaptureSchema.parse({
        state: "settled",
        outcome: "complete",
        wireOutcome: "complete",
        transportOutcome: "stream_closed",
        framesSeen: 2,
        printableFramesSeen: 0,
        printableTurnEvents: 0,
        wireBytes: 384,
        terminalReceived: false,
        firstFrameMs: 42,
        lastFrameMs: 287,
      }),
    ).toEqual({
      state: "settled",
      outcome: "complete",
      wireOutcome: "complete",
      transportOutcome: "stream_closed",
      framesSeen: 2,
      printableFramesSeen: 0,
      printableTurnEvents: 0,
      wireBytes: 384,
      terminalReceived: false,
      firstFrameMs: 42,
      lastFrameMs: 287,
    });
  });

  it("rejects payload-shaped transport outcomes and undeclared telemetry fields", () => {
    expect(
      assistantTransportOutcomeSchema.safeParse(
        "stream_closed: assistant reply payload",
      ).success,
    ).toBe(false);
    expect(
      assistantWireCaptureSchema.safeParse({
        state: "settled",
        outcome: "complete",
        wireOutcome: "complete",
        transportOutcome: "stream_closed",
        framesSeen: 0,
        printableFramesSeen: 0,
        printableTurnEvents: 0,
        wireBytes: 0,
        terminalReceived: false,
        firstFrameMs: null,
        lastFrameMs: null,
        terminalPayload: { output: "must never be recorded" },
      }).success,
    ).toBe(false);
  });
});
