import { describe, expect, it } from "vitest";
import {
  AGUIEventType,
  CustomEventName,
  type AGUIEvent,
} from "@/lib/assistant/agui-types";
import {
  getLatestCustomEventData,
  parseHumanInputRequestData,
  parseWaitingSignalData,
} from "./custom-event-data";

describe("customEventData", () => {
  it("returns the latest valid custom payload and skips malformed entries", () => {
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.WaitingSignal,
        value: {
          runId: "run-1",
          stepId: "step-1",
          signalName: "deployment_ready",
        },
      },
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.WaitingSignal,
        value: "malformed",
      },
    ];

    const result = getLatestCustomEventData(
      events,
      CustomEventName.WaitingSignal,
      parseWaitingSignalData,
    );

    expect(result).toEqual({
      runId: "run-1",
      stepId: "step-1",
      signalName: "deployment_ready",
      prompt: undefined,
      timeoutMs: undefined,
    });
  });

  it("parses human input request payloads conservatively", () => {
    expect(
      parseHumanInputRequestData({
        runId: "run-1",
        stepId: "approve",
        suspensionType: "human_approval",
        prompt: "Approve release?",
        metadata: {
          owner: "ops",
        },
      }),
    ).toEqual({
      runId: "run-1",
      stepId: "approve",
      suspensionType: "human_approval",
      prompt: "Approve release?",
      timeoutSeconds: undefined,
      metadata: {
        owner: "ops",
      },
    });
    expect(parseHumanInputRequestData("bad")).toBeUndefined();
  });
});
