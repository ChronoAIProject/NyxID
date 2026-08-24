import { describe, expect, it } from "vitest";
import {
  AGUIEventType,
  CustomEventName,
  type AGUIEvent,
} from "@/lib/assistant/agui-types";
import {
  applyRuntimeEvent,
  createRuntimeEventAccumulator,
} from "./runtime-event-semantics";

describe("runtimeEventSemantics", () => {
  it("keeps run-finished output ahead of later completed step output", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.RUN_FINISHED,
        result: {
          output: "final run answer",
        },
        runId: "run-1",
        threadId: "thread-1",
      },
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.StepCompleted,
        value: {
          runId: "run-1",
          stepId: "late-step",
          success: true,
          output: "late step output",
        },
      },
    ];

    events.forEach((event) => {
      applyRuntimeEvent(accumulator, event);
    });

    expect(accumulator.finalOutput).toBe("final run answer");
  });

  it("allows run-finished output to replace earlier step output", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.StepCompleted,
        value: {
          runId: "run-1",
          stepId: "first-step",
          success: true,
          output: "first step output",
        },
      },
      {
        type: AGUIEventType.RUN_FINISHED,
        result: {
          output: "final run answer",
        },
        runId: "run-1",
        threadId: "thread-1",
      },
    ];

    events.forEach((event) => {
      applyRuntimeEvent(accumulator, event);
    });

    expect(accumulator.finalOutput).toBe("final run answer");
  });

  it("tracks command, correlation, and error code identifiers", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.RUN_STARTED,
        actorId: "actor-1",
        commandId: "cmd-1",
        correlationId: "corr-1",
        runId: "run-1",
        threadId: "actor-1",
      } as unknown as AGUIEvent,
      {
        type: AGUIEventType.RUN_ERROR,
        code: "ERR_RUNTIME",
        commandId: "cmd-1",
        correlationId: "corr-1",
        message: "failed",
        runId: "run-1",
      } as unknown as AGUIEvent,
    ];

    events.forEach((event) => {
      applyRuntimeEvent(accumulator, event);
    });

    expect(accumulator.actorId).toBe("actor-1");
    expect(accumulator.commandId).toBe("cmd-1");
    expect(accumulator.correlationId).toBe("corr-1");
    expect(accumulator.errorCode).toBe("ERR_RUNTIME");
    expect(accumulator.errorText).toBe("failed");
  });

  it("keeps run-started command and correlation ids through run finish", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.RUN_STARTED,
        actorId: "actor-1",
        commandId: "cmd-1",
        correlationId: "corr-1",
        runId: "run-1",
        threadId: "actor-1",
      } as unknown as AGUIEvent,
      {
        type: AGUIEventType.RUN_FINISHED,
        result: {
          output: "done",
        },
        runId: "run-1",
      } as unknown as AGUIEvent,
    ];

    events.forEach((event) => {
      applyRuntimeEvent(accumulator, event);
    });

    expect(accumulator.actorId).toBe("actor-1");
    expect(accumulator.commandId).toBe("cmd-1");
    expect(accumulator.correlationId).toBe("corr-1");
    expect(accumulator.finalOutput).toBe("done");
    expect(accumulator.runId).toBe("run-1");
  });
});

describe("runtime event accumulation", () => {
  it("uses text end, completed step, and run finished in source-priority order", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.TEXT_MESSAGE_CONTENT,
        messageId: "message-1",
        delta: "delta fallback",
      },
      {
        type: AGUIEventType.TEXT_MESSAGE_END,
        messageId: "message-1",
        message: "text end",
      },
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.StepCompleted,
        value: { stepId: "step-1", success: true, output: "step output" },
      },
      {
        type: AGUIEventType.RUN_FINISHED,
        runId: "run-1",
        result: { output: "run output" },
      },
    ];
    events.forEach((event) => applyRuntimeEvent(accumulator, event));
    expect(accumulator).toMatchObject({
      assistantText: "delta fallback",
      finalOutput: "run output",
      finalOutputSource: "run_finished",
    });
  });

  it("falls back to completed step output when no text deltas exist", () => {
    const accumulator = createRuntimeEventAccumulator();
    applyRuntimeEvent(accumulator, {
      type: AGUIEventType.CUSTOM,
      name: CustomEventName.StepCompleted,
      value: { stepId: "step-1", success: true, output: "step output" },
    });
    expect(accumulator.assistantText).toBe("step output");
    expect(accumulator.finalOutput).toBe("step output");
  });

  it("accumulates reasoning and step/tool lifecycles", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.LlmReasoning,
        value: { role: "assistant", delta: "Checking " },
      },
      {
        type: AGUIEventType.CUSTOM,
        name: CustomEventName.LlmReasoning,
        value: { role: "assistant", delta: "the account." },
      },
      { type: AGUIEventType.STEP_STARTED, stepName: "inspect", timestamp: 1 },
      {
        type: AGUIEventType.TOOL_CALL_START,
        toolCallId: "tool-1",
        toolName: "account.read",
        timestamp: 2,
      },
      {
        type: AGUIEventType.TOOL_CALL_END,
        toolCallId: "tool-1",
        result: "ready",
        timestamp: 3,
      },
      { type: AGUIEventType.STEP_FINISHED, stepName: "inspect", timestamp: 4 },
    ];
    events.forEach((event) => applyRuntimeEvent(accumulator, event));
    expect(accumulator.thinking).toBe("Checking the account.");
    expect(accumulator.steps).toEqual([
      {
        id: "inspect",
        name: "inspect",
        startedAt: 1,
        finishedAt: 4,
        status: "done",
      },
    ]);
    expect(accumulator.toolCalls).toEqual([
      {
        id: "tool-1",
        name: "account.read",
        startedAt: 2,
        finishedAt: 3,
        result: "ready",
        status: "done",
      },
    ]);
  });

  it("records run errors without replacing a higher-priority final output", () => {
    const accumulator = createRuntimeEventAccumulator();
    applyRuntimeEvent(accumulator, {
      type: AGUIEventType.RUN_FINISHED,
      runId: "run-1",
      result: { output: "committed output" },
    });
    applyRuntimeEvent(accumulator, {
      type: AGUIEventType.RUN_ERROR,
      runId: "run-1",
      code: "ERR_RUNTIME",
      message: "delivery failed",
    });
    expect(accumulator).toMatchObject({
      errorCode: "ERR_RUNTIME",
      errorText: "delivery failed",
      finalOutput: "committed output",
    });
  });

  it("accumulates media artifacts and both typed blocker sources", () => {
    const accumulator = createRuntimeEventAccumulator();
    const events: AGUIEvent[] = [
      {
        type: AGUIEventType.MEDIA_CONTENT,
        dataBase64: "aGVsbG8=",
        mediaType: "text/plain",
        name: "fixture.txt",
      },
      {
        type: AGUIEventType.CUSTOM,
        name: "nyxid.authorization.required",
        value: {
          serviceSlug: "api-github",
          serviceLabel: "GitHub",
          reasonCode: "NYXID_UNAUTHORIZED",
          safeMessage: "Reconnect GitHub.",
        },
      },
      {
        type: AGUIEventType.TOOL_CALL_END,
        toolCallId: "tool-lark",
        result: JSON.stringify({
          blocked: true,
          service_slug: "api-lark",
          readiness_status: "ServiceRegistrationRequired",
          reason_code: "USER_SERVICE_NOT_VISIBLE",
          safe_message: "No visible service.",
        }),
      },
    ];
    events.forEach((event) => applyRuntimeEvent(accumulator, event));
    expect(accumulator.artifacts).toHaveLength(1);
    expect(accumulator.artifacts[0]).toMatchObject({
      name: "fixture.txt",
      download_url: "data:text/plain;base64,aGVsbG8=",
    });
    expect(accumulator.authorizationBlockers).toEqual([
      expect.objectContaining({
        reasonCode: "NYXID_UNAUTHORIZED",
        serviceSlug: "api-github",
      }),
      expect.objectContaining({
        reasonCode: "NYXID_SERVICE_NOT_CONNECTED",
        serviceSlug: "api-lark",
      }),
    ]);
  });

  it("retains RUN_STOPPED as a terminal runtime event", () => {
    const accumulator = createRuntimeEventAccumulator();
    applyRuntimeEvent(accumulator, {
      type: AGUIEventType.RUN_STOPPED,
      runId: "run-1",
      reason: "operator",
    });
    expect(accumulator.events.at(-1)).toMatchObject({
      type: AGUIEventType.RUN_STOPPED,
      reason: "operator",
    });
    expect(accumulator.errorText).toBe("");
  });
});
