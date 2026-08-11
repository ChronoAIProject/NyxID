import { describe, expect, it } from "vitest";
import { decodeTaskPlan, TaskPlanProtocolError } from "./task-plan";

function step(
  stepId: string,
  order: number,
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    stepId,
    order,
    kind: "tool",
    status: "planned",
    required: true,
    description: `Step ${stepId}`,
    source: {
      tool: {
        toolName: "connected_service_operation",
        serviceSlug: "api-github",
      },
    },
    mayChangeExternalState: false,
    externalEffect: "not_started",
    availableActions: { retry: false, skip: false, stop: false },
    dependsOn: [],
    substeps: [],
    ...overrides,
  };
}

function plan(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    schemaVersion: 4,
    actorId: "nyxid-chat-actor-alpha",
    taskId: "task-alpha",
    turnId: "turn-alpha",
    planId: "plan-alpha",
    planRevision: 2,
    planRevisions: [],
    title: "Publish a weekly update",
    status: "active",
    gate: { mode: "auto", status: "satisfied" },
    steps: [step("step-b", 2), step("step-a", 1)],
    ...overrides,
  };
}

describe("decodeTaskPlan", () => {
  it("decodes the full actor plan, sorts steps, and tolerates additive fields", () => {
    const decoded = decodeTaskPlan({
      ...plan(),
      futureEnvelopeField: { enabled: true },
      steps: [
        step("step-b", 2, {
          futureStepField: "ignored",
          source: {
            tool: {
              toolName: "connected_service_operation",
              serviceSlug: "api-github",
            },
            futureSourceArm: { label: "ignored" },
          },
          substeps: [
            {
              substepId: "substep-b",
              title: "Search repository",
              status: "done",
            },
          ],
        }),
        step("step-a", 1),
      ],
    });

    expect(decoded.steps.map((candidate) => candidate.stepId)).toEqual([
      "step-a",
      "step-b",
    ]);
    expect(decoded.steps[1]?.substeps).toEqual([
      { substepId: "substep-b", title: "Search repository", status: "done" },
    ]);
  });

  it("fails closed on unknown action verbs while defaulting omitted known verbs", () => {
    const decoded = decodeTaskPlan(
      plan({
        steps: [step("step-a", 1, { availableActions: { retry: true } })],
      }),
    );
    expect(decoded.steps[0]?.availableActions).toEqual({
      retry: true,
      skip: false,
      stop: false,
    });

    expect(() =>
      decodeTaskPlan(
        plan({
          steps: [
            step("step-a", 1, {
              availableActions: { retry: false, pause: true },
            }),
          ],
        }),
      ),
    ).toThrow("unknown action verb");
  });

  it("preserves the public approval terminal outcome and non-sensitive subject kind", () => {
    const decoded = decodeTaskPlan(
      plan({
        steps: [
          step("step-a", 1, {
            approvalObservation: {
              approvalRequestId: "approval-alpha",
              decisionMode: "per_request",
              receiptStatus: "denied",
              observedAt: "2026-08-11T08:00:00Z",
              terminalOutcome: "rejected",
              subjectKind: "nyxid.user-service",
              futureObservationField: "ignored",
            },
          }),
        ],
      }),
    );

    expect(decoded.steps[0]?.approvalObservation).toEqual({
      approvalRequestId: "approval-alpha",
      decisionMode: "per_request",
      receiptStatus: "denied",
      observedAt: "2026-08-11T08:00:00Z",
      terminalOutcome: "rejected",
      subjectKind: "nyxid.user-service",
    });
  });

  it.each([
    ["unknown task status", plan({ status: "paused" })],
    [
      "source-kind mismatch",
      plan({ steps: [step("step-a", 1, { source: { web: {} } })] }),
    ],
    [
      "duplicate step identity",
      plan({ steps: [step("step-a", 1), step("step-a", 2)] }),
    ],
    [
      "operation actor identity mismatch",
      plan({
        steps: [
          step("step-a", 1, {
            operation: {
              conversationActorId: "nyxid-chat-actor-other",
              taskId: "task-alpha",
              stepId: "step-a",
              operationGeneration: 1,
            },
          }),
        ],
      }),
    ],
    [
      "operation task identity mismatch",
      plan({
        steps: [
          step("step-a", 1, {
            operation: {
              taskId: "task-other",
              stepId: "step-a",
              operationGeneration: 1,
            },
          }),
        ],
      }),
    ],
    [
      "operation step identity mismatch",
      plan({
        steps: [
          step("step-a", 1, {
            operation: {
              taskId: "task-alpha",
              stepId: "step-other",
              operationGeneration: 1,
            },
          }),
        ],
      }),
    ],
    [
      "unbound pending gate",
      plan({
        gate: {
          mode: "confirm",
          status: "pending",
          requestId: "input-plan",
          taskId: "task-other",
          planId: "plan-alpha",
          planRevision: 2,
        },
      }),
    ],
  ])("rejects %s", (_label, input) => {
    expect(() => decodeTaskPlan(input)).toThrow(TaskPlanProtocolError);
  });
});
