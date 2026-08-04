import { describe, expect, it } from "vitest";
import {
  assistantTransportOutcomeSchema,
  assistantWireCaptureSchema,
} from "./assistant-wire-log";

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
