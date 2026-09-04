import { describe, expect, it } from "vitest";

import {
  ChatTaskPlanProtocolError,
  decodeChatTaskPlan,
} from "./chat-task-plan";

function taskPlanWithObservation(overrides: Record<string, unknown> = {}) {
  return {
    schemaVersion: 4,
    actorId: "conversation-alpha",
    taskId: "task-alpha",
    turnId: "turn-alpha",
    planId: "plan-alpha",
    planRevision: 1,
    planRevisions: [],
    title: "Update repository",
    status: "active",
    steps: [
      {
        stepId: "step-update",
        order: 1,
        kind: "tool",
        status: "waiting",
        required: true,
        description: "Update repository",
        source: {
          tool: {
            toolName: "repository_update",
            serviceSlug: "github-api",
            serviceId: "svc-alpha",
          },
        },
        mayChangeExternalState: true,
        externalEffect: "not_started",
        availableActions: {},
        dependsOn: [],
        substeps: [],
        approvalObservation: {
          approvalRequestId: "nyxid-approval-alpha",
          decisionMode: "per_request",
          receiptStatus: "approval_required",
          observedAt: "2026-08-08T00:10:00Z",
          terminalOutcome: "rejected",
          subjectKind: "nyxid.user-service",
          ...overrides,
        },
      },
    ],
  };
}

function taskPlanWithCondition(
  conditionOverrides: Record<string, unknown> = {},
) {
  return {
    schemaVersion: 5,
    actorId: "conversation-alpha",
    taskId: "task-alpha",
    turnId: "turn-alpha",
    planId: "plan-alpha",
    planRevision: 2,
    planRevisions: [],
    title: "Evaluate threshold",
    status: "active",
    steps: [
      {
        stepId: "step-condition",
        order: 1,
        kind: "condition",
        status: "done",
        required: true,
        description: "Evaluate the committed threshold",
        source: {
          condition: {
            condition: {
              conditionId: "condition-alpha",
              sourceInputRequestId: "input-alpha",
              suggestedThreshold: 70,
              effectiveThreshold: 75,
              thresholdOrigin: "user_override",
              observedValue: 80,
              comparison: "gte",
              outcome: "true",
              evaluatedAt: "2026-08-09T00:10:00Z",
              guardedToolName: "external_record_create",
              ...conditionOverrides,
            },
          },
        },
        mayChangeExternalState: false,
        externalEffect: "not_applied",
        availableActions: {},
        dependsOn: ["step-input"],
        substeps: [],
      },
      {
        stepId: "step-write",
        order: 2,
        kind: "tool",
        status: "planned",
        required: true,
        description: "Create the verified record",
        source: { tool: { toolName: "external_record_create" } },
        mayChangeExternalState: true,
        externalEffect: "not_started",
        availableActions: {},
        dependsOn: ["step-condition"],
        substeps: [],
        guard: {
          conditionStepId: "step-condition",
          requiredOutcome: "true",
        },
      },
    ],
  };
}

describe("decodeChatTaskPlan", () => {
  it("preserves the complete typed Tier-B approval observation", () => {
    const decoded = decodeChatTaskPlan(taskPlanWithObservation());

    expect(decoded.steps[0]?.approvalObservation).toEqual({
      approvalRequestId: "nyxid-approval-alpha",
      decisionMode: "per_request",
      receiptStatus: "approval_required",
      observedAt: "2026-08-08T00:10:00Z",
      terminalOutcome: "rejected",
      subjectKind: "nyxid.user-service",
    });
  });

  it.each([
    ["decisionMode", "session"],
    ["receiptStatus", "pending"],
    ["terminalOutcome", "cancelled"],
  ])("rejects an unknown approval observation %s", (field, value) => {
    expect(() =>
      decodeChatTaskPlan(taskPlanWithObservation({ [field]: value })),
    ).toThrow(ChatTaskPlanProtocolError);
  });

  it("preserves typed condition and guard facts across reload", () => {
    const decoded = decodeChatTaskPlan(taskPlanWithCondition());

    expect(decoded.steps[0]?.source).toEqual({
      kind: "condition",
      label: "80 >= 75",
      condition: {
        conditionId: "condition-alpha",
        sourceInputRequestId: "input-alpha",
        suggestedThreshold: 70,
        effectiveThreshold: 75,
        thresholdOrigin: "user_override",
        observedValue: 80,
        comparison: "gte",
        outcome: "true",
        evaluatedAt: "2026-08-09T00:10:00Z",
        guardedToolName: "external_record_create",
      },
    });
    expect(decoded.steps[1]?.guard).toEqual({
      conditionStepId: "step-condition",
      requiredOutcome: "true",
    });
  });

  it("decodes a historical confirm gate as a readable plan with no outbound command identity", () => {
    const decoded = decodeChatTaskPlan({
      schemaVersion: 4,
      actorId: "conversation-alpha",
      taskId: "task-alpha",
      turnId: "turn-alpha",
      planId: "plan-alpha",
      planRevision: 3,
      planRevisions: [],
      title: "Inspect repository",
      status: "active",
      gate: {
        mode: "confirm",
        status: "pending",
        requestId: "plan-gate-alpha",
        taskId: "task-alpha",
        planId: "plan-alpha",
        planRevision: 3,
        reason: "Confirm the planned write steps.",
      },
      steps: [
        {
          stepId: "step-alpha",
          order: 1,
          kind: "tool",
          status: "waiting",
          required: true,
          description: "Inspect repository",
          source: { tool: { toolName: "repository_read" } },
          mayChangeExternalState: false,
          externalEffect: "not_started",
          availableActions: {},
          dependsOn: [],
          substeps: [],
        },
      ],
    });

    expect(decoded.title).toBe("Inspect repository");
    expect(decoded.steps).toHaveLength(1);
    expect(decoded.gate).toEqual({
      mode: "confirm",
      status: "pending",
      reason: "Confirm the planned write steps.",
    });
    expect(decoded.gate).not.toHaveProperty("requestId");
    expect(decoded.gate).not.toHaveProperty("taskId");
    expect(decoded.gate).not.toHaveProperty("planId");
    expect(decoded.gate).not.toHaveProperty("planRevision");
    expect(JSON.stringify(decoded)).not.toContain("plan.resolve");
  });

  it.each([
    ["thresholdOrigin", "override"],
    ["comparison", "gt"],
    ["outcome", "passed"],
  ])("rejects an unknown condition %s", (field, value) => {
    expect(() =>
      decodeChatTaskPlan(taskPlanWithCondition({ [field]: value })),
    ).toThrow(ChatTaskPlanProtocolError);
  });
});
