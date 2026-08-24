import { describe, expect, it } from "vitest";

import {
  actorCan,
  applyCurrentStateResult,
  createChatActorProjection,
  decodeActorFrame,
  reduceActorFrame,
  validateActionRequest,
} from "./chat-actor-state";

function taskPlan(stepStatus: "running" | "failed" = "running") {
  return {
    schemaVersion: 4,
    actorId: "conversation-alpha",
    taskId: "task-alpha",
    turnId: "turn-alpha",
    planId: "plan-alpha",
    planRevision: 3,
    planRevisionHistoryStart: 1,
    planRevisions: [
      {
        planRevision: 3,
        revisionCause: "steering",
        committedAt: "2026-08-08T00:00:00Z",
        addedStepIds: ["step-alpha"],
        cancelledStepIds: [],
      },
    ],
    title: "Inspect repository",
    status: "active",
    activeStepId: "step-alpha",
    gate: {
      mode: "confirm",
      status: "satisfied",
      requestId: "gate-alpha",
      taskId: "task-alpha",
      planId: "plan-alpha",
      planRevision: 3,
    },
    steps: [
      {
        stepId: "step-alpha",
        order: 1,
        kind: "tool",
        status: stepStatus,
        required: true,
        description: "Inspect repository",
        source: {
          tool: {
            toolName: "repository_read",
            serviceSlug: "github-api",
            serviceId: "svc-alpha",
          },
        },
        mayChangeExternalState: false,
        externalEffect: stepStatus === "failed" ? "not_applied" : "not_started",
        availableActions:
          stepStatus === "failed" ? { retry: true } : { stop: true },
        updatedAt: "2026-08-08T00:00:00Z",
        addedBy: "steering",
        addedInPlanRevision: 3,
        dependsOn: [],
        substeps: [
          {
            substepId: "substep-alpha",
            title: "Read",
            status: stepStatus === "failed" ? "failed" : "running",
          },
        ],
        operation: {
          conversationActorId: "conversation-alpha",
          turnId: "turn-alpha",
          taskId: "task-alpha",
          stepId: "step-alpha",
          operationId: "operation-alpha",
          operationGeneration: 2,
          phase: stepStatus,
          lastProgressAt: "2026-08-08T00:00:30Z",
          stalledAt:
            stepStatus === "failed" ? undefined : "2026-08-08T00:02:30Z",
        },
        approvalObservation: {
          approvalRequestId: "nyxid-approval-alpha",
          decisionMode: "grant",
          receiptStatus: "denied",
          observedAt: "2026-08-08T00:02:31Z",
          terminalOutcome: "expired",
          subjectKind: "nyxid.user-service",
        },
      },
    ],
  };
}

const actionRequest = {
  schemaVersion: 4,
  actorId: "conversation-alpha",
  originTurnId: "turn-alpha",
  taskId: "task-alpha",
  stepId: "step-connect",
  actionRequestId: "action-alpha",
  action: "service.connect",
  params: {
    catalogService: { serviceSlug: "api-github", requestedScopes: ["repo"] },
  },
} as const;

describe("chatActorState", () => {
  it("decodes the same bounded numeric threshold from live frames and reloads", () => {
    const pendingInput = {
      requestId: "input-threshold",
      prompt: "Choose the threshold",
      options: [],
      allowFreeText: true,
      multiSelect: false,
      numericThreshold: {
        suggestedValue: 70,
        minimumValue: 0,
        maximumValue: 100,
      },
    };
    const live = reduceActorFrame(
      createChatActorProjection("conversation-alpha"),
      decodeActorFrame({
        sequence: 1,
        custom: { name: "nyxid.input.request", payload: pendingInput },
      }),
    );
    const reload = applyCurrentStateResult(
      createChatActorProjection("conversation-alpha"),
      {
        status: "current",
        stateVersion: 5,
        snapshot: {
          actorId: "conversation-alpha",
          scopeId: "scope-alpha",
          stateVersion: 5,
          progressSequence: 1,
          activeTurn: null,
          latestTurn: null,
          recentTerminalTurns: [],
          activeTask: null,
          pendingInput,
          pendingApproval: null,
          pendingActions: [],
        },
      },
    ).projection;

    expect(live.pendingInput).toEqual(pendingInput);
    expect(reload.pendingInput).toEqual(pendingInput);
  });

  it.each([
    { suggestedValue: 70.5, minimumValue: 0, maximumValue: 100 },
    {
      suggestedValue: Number.MAX_SAFE_INTEGER + 1,
      minimumValue: 0,
      maximumValue: Number.MAX_SAFE_INTEGER + 1,
    },
    { suggestedValue: 70, minimumValue: 80, maximumValue: 100 },
    { suggestedValue: 101, minimumValue: 0, maximumValue: 100 },
  ])("rejects an invalid live numeric threshold %#", (numericThreshold) => {
    expect(() =>
      decodeActorFrame({
        sequence: 1,
        custom: {
          name: "nyxid.input.request",
          payload: {
            requestId: "input-threshold",
            numericThreshold,
          },
        },
      }),
    ).toThrow(
      expect.objectContaining({
        code: "NYXID_INPUT_NUMERIC_THRESHOLD_INVALID",
      }),
    );
  });

  it("rejects an invalid reloaded numeric threshold", () => {
    expect(() =>
      applyCurrentStateResult(createChatActorProjection("conversation-alpha"), {
        status: "current",
        stateVersion: 5,
        snapshot: {
          actorId: "conversation-alpha",
          scopeId: "scope-alpha",
          stateVersion: 5,
          progressSequence: 1,
          activeTurn: null,
          latestTurn: null,
          recentTerminalTurns: [],
          activeTask: null,
          pendingInput: {
            requestId: "input-threshold",
            numericThreshold: {
              suggestedValue: 70,
              minimumValue: 80,
              maximumValue: 100,
            },
          },
          pendingApproval: null,
          pendingActions: [],
        },
      }),
    ).toThrow(
      expect.objectContaining({
        code: "NYXID_INPUT_NUMERIC_THRESHOLD_INVALID",
      }),
    );
  });

  it("uses the same typed TaskPlan decoder for live frames and current-state reload", () => {
    const live = reduceActorFrame(
      createChatActorProjection("conversation-alpha"),
      decodeActorFrame({
        type: "CUSTOM",
        sequence: 7,
        custom: { name: "nyxid.task.snapshot", payload: taskPlan() },
      }),
    );
    const reloaded = applyCurrentStateResult(
      createChatActorProjection("conversation-alpha"),
      {
        status: "current",
        stateVersion: 17,
        snapshot: {
          actorId: "conversation-alpha",
          scopeId: "scope-alpha",
          stateVersion: 17,
          progressSequence: 7,
          activeTurn: {
            turnId: "turn-alpha",
            taskId: "task-alpha",
            status: "active",
          },
          latestTurn: null,
          recentTerminalTurns: [],
          activeTask: taskPlan(),
          pendingInput: null,
          pendingApproval: null,
          pendingActions: [],
        },
      },
    ).projection;

    expect(reloaded.task).toEqual(live.task);
    expect([...reloaded.steps.values()]).toEqual([...live.steps.values()]);
    expect(reloaded.steps.get("step-alpha")?.operation).toEqual(
      expect.objectContaining({
        lastProgressAt: "2026-08-08T00:00:30Z",
        stalledAt: "2026-08-08T00:02:30Z",
      }),
    );
    expect(reloaded.steps.get("step-alpha")?.approvalObservation).toEqual({
      approvalRequestId: "nyxid-approval-alpha",
      decisionMode: "grant",
      receiptStatus: "denied",
      observedAt: "2026-08-08T00:02:31Z",
      terminalOutcome: "expired",
      subjectKind: "nyxid.user-service",
    });
    expect(actorCan(reloaded, "stop")).toBe(true);
    expect(reloaded.steps.get("step-alpha")?.availableActions).toEqual({
      retry: false,
      skip: false,
      stop: true,
    });

    const changed = reduceActorFrame(
      reloaded,
      decodeActorFrame({
        type: "CUSTOM",
        sequence: 8,
        custom: {
          name: "nyxid.task.step.changed",
          payload: {
            taskId: "task-alpha",
            planRevision: 3,
            step: {
              ...taskPlan("failed").steps[0],
              updatedAt: "2026-08-08T00:02:00Z",
            },
            changeKind: "status",
          },
        },
      }),
    );
    expect(changed.steps.get("step-alpha")?.status).toBe("failed");
    expect(changed.steps.get("step-alpha")?.updatedAt).toBe(
      "2026-08-08T00:02:00Z",
    );
    expect(actorCan(changed, "retry", "step-alpha")).toBe(true);
    expect(changed.steps.get("step-alpha")?.availableActions).toEqual({
      retry: true,
      skip: false,
      stop: false,
    });

    const stale = reduceActorFrame(
      changed,
      decodeActorFrame({
        type: "CUSTOM",
        sequence: 7,
        custom: {
          name: "nyxid.task.step.changed",
          payload: {
            taskId: "task-alpha",
            planRevision: 3,
            step: taskPlan().steps[0],
            changeKind: "status",
          },
        },
      }),
    );
    const conflictingDuplicate = reduceActorFrame(
      changed,
      decodeActorFrame({
        type: "CUSTOM",
        sequence: 8,
        custom: {
          name: "nyxid.task.step.changed",
          payload: {
            taskId: "task-alpha",
            planRevision: 3,
            step: taskPlan().steps[0],
            changeKind: "status",
          },
        },
      }),
    );
    expect(stale).toBe(changed);
    expect(conflictingDuplicate).toBe(changed);
    expect(stale.steps.get("step-alpha")?.status).toBe("failed");
  });

  it("keeps the committed control result across duplicate and stale current-state reads", () => {
    const state = (
      stateVersion: number,
      outcome: string,
      stepOutcome: string,
      recentStepOutcomes: readonly string[],
    ) => ({
      status: "current",
      stateVersion,
      snapshot: {
        actorId: "conversation-alpha",
        scopeId: "scope-alpha",
        stateVersion,
        progressSequence: stateVersion,
        activeTurn: {
          turnId: "turn-alpha",
          taskId: "task-alpha",
          status: "active",
        },
        latestTurn: null,
        recentTerminalTurns: [],
        activeTask: taskPlan(),
        pendingInput: null,
        pendingApproval: null,
        pendingActions: [],
        recentActions: [],
        latestControlResult: { outcome },
        latestStepControlResult: { outcome: stepOutcome },
        recentStepControlResults: recentStepOutcomes.map((recentOutcome) => ({
          outcome: recentOutcome,
        })),
      },
    });
    const committed = applyCurrentStateResult(
      createChatActorProjection("conversation-alpha"),
      state(11, "steered", "retry_started", [
        "retry_requested",
        "retry_started",
      ]),
    ).projection;
    const duplicate = applyCurrentStateResult(
      committed,
      state(11, "duplicate-regressed", "duplicate-step-regressed", [
        "duplicate-step-regressed",
      ]),
    ).projection;
    const stale = applyCurrentStateResult(
      duplicate,
      state(10, "stale-result", "stale-step-result", ["stale-step-result"]),
    ).projection;

    expect(duplicate).toBe(committed);
    expect(duplicate.latestControlResult).toEqual({ outcome: "steered" });
    expect(duplicate.latestStepControlResult).toEqual({
      outcome: "retry_started",
    });
    expect(duplicate.recentStepControlResults).toEqual([
      { outcome: "retry_requested" },
      { outcome: "retry_started" },
    ]);
    expect(stale).toBe(duplicate);
    expect(stale.latestControlResult).toEqual({ outcome: "steered" });
    expect(stale.latestStepControlResult).toEqual({
      outcome: "retry_started",
    });
    expect(stale.recentStepControlResults).toEqual([
      { outcome: "retry_requested" },
      { outcome: "retry_started" },
    ]);
    expect(stale.stateVersion).toBe(11);
  });

  it("handles every current-state envelope outcome and unknown snapshot fields", () => {
    const projection = createChatActorProjection("conversation-alpha");
    projection.stateVersion = 4;

    const notModified = applyCurrentStateResult(projection, {
      status: "not_modified",
      stateVersion: 4,
    });
    const reloadRequired = applyCurrentStateResult(projection, {
      status: "reload_required",
    });
    const notFound = applyCurrentStateResult(projection, {
      status: "not_found",
    });
    const current = applyCurrentStateResult(projection, {
      status: "current",
      stateVersion: 5,
      snapshot: {
        actorId: "conversation-alpha",
        scopeId: "scope-alpha",
        stateVersion: 5,
        progressSequence: 2,
        activeTurn: null,
        latestTurn: null,
        recentTerminalTurns: [],
        activeTask: null,
        pendingInput: null,
        pendingApproval: null,
        pendingActions: [],
        canaryEffectFault: { futureField: true },
      },
    });

    expect(notModified).toEqual({
      projection,
      reloadWithoutCursor: false,
    });
    expect(reloadRequired).toEqual({
      projection,
      reloadWithoutCursor: true,
    });
    expect(notFound.projection).toMatchObject({
      actorId: "conversation-alpha",
      stateVersion: 0,
    });
    expect(current.projection).toMatchObject({
      actorId: "conversation-alpha",
      scopeId: "scope-alpha",
      stateVersion: 5,
    });
  });

  it("records state version and identity conflicts without replacing state", () => {
    const projection = createChatActorProjection("conversation-alpha");
    projection.stateVersion = 4;

    const versionConflict = applyCurrentStateResult(projection, {
      status: "not_modified",
      stateVersion: 5,
    }).projection;
    const identityConflict = applyCurrentStateResult(projection, {
      status: "current",
      stateVersion: 5,
      snapshot: {
        actorId: "conversation-other",
        scopeId: "scope-alpha",
        stateVersion: 5,
        progressSequence: 1,
      },
    }).projection;

    expect(versionConflict.conflicts).toContainEqual({
      code: "NYXID_STATE_VERSION_CONFLICT",
    });
    expect(identityConflict.conflicts).toContainEqual({
      code: "NYXID_STATE_IDENTITY_CONFLICT",
    });
    expect(identityConflict.actorId).toBe("conversation-alpha");
  });

  it("fails closed on invalid closed vocabulary instead of constructing browser state", () => {
    expect(() =>
      decodeActorFrame({
        type: "CUSTOM",
        sequence: 7,
        custom: {
          name: "nyxid.task.snapshot",
          payload: { ...taskPlan(), status: "almost_done" },
        },
      }),
    ).not.toThrow();
    expect(() =>
      reduceActorFrame(
        createChatActorProjection("conversation-alpha"),
        decodeActorFrame({
          type: "CUSTOM",
          sequence: 7,
          custom: {
            name: "nyxid.task.snapshot",
            payload: { ...taskPlan(), status: "almost_done" },
          },
        }),
      ),
    ).toThrow(expect.objectContaining({ code: "NYXID_TASK_PLAN_INVALID" }));
  });

  it("rehydrates only secret-free exact pending and recent action requests", () => {
    expect(validateActionRequest(actionRequest)).toMatchObject({
      request: actionRequest,
      supported: true,
      recovered: false,
    });

    const live = reduceActorFrame(
      createChatActorProjection("conversation-alpha"),
      decodeActorFrame({
        sequence: 1,
        custom: { name: "nyxid.action.request", payload: actionRequest },
      }),
    );
    expect(live.actions.get("action-alpha")?.request).toMatchObject(
      actionRequest,
    );

    const reload = applyCurrentStateResult(
      createChatActorProjection("conversation-alpha"),
      {
        status: "current",
        stateVersion: 5,
        snapshot: {
          actorId: "conversation-alpha",
          scopeId: "scope-alpha",
          stateVersion: 5,
          progressSequence: 2,
          activeTurn: null,
          latestTurn: null,
          recentTerminalTurns: [],
          activeTask: null,
          pendingInput: null,
          pendingApproval: null,
          pendingActions: [
            {
              schemaVersion: 4,
              originTurnId: "turn-alpha",
              taskId: "task-alpha",
              stepId: "step-connect",
              actionRequestId: "action-alpha",
              action: "service.connect",
              reports: [],
              postconditionResult: null,
              request: actionRequest,
            },
          ],
          recentActions: [
            {
              schemaVersion: 4,
              originTurnId: "turn-alpha",
              taskId: "task-alpha",
              stepId: "step-recent",
              actionRequestId: "action-recent",
              action: "service.connect",
              reports: [],
              postconditionResult: null,
              request: {
                ...actionRequest,
                stepId: "step-recent",
                actionRequestId: "action-recent",
              },
            },
          ],
        },
      },
    ).projection;
    expect(reload.actions.get("action-alpha")?.request).toMatchObject(
      actionRequest,
    );
    expect(reload.actions.get("action-recent")?.request).toMatchObject({
      ...actionRequest,
      stepId: "step-recent",
      actionRequestId: "action-recent",
    });
  });

  it("keeps a malformed reloaded action summary declinable without throwing", () => {
    const reload = applyCurrentStateResult(
      createChatActorProjection("conversation-alpha"),
      {
        status: "current",
        stateVersion: 5,
        snapshot: {
          actorId: "conversation-alpha",
          scopeId: "scope-alpha",
          stateVersion: 5,
          progressSequence: 2,
          activeTurn: null,
          latestTurn: null,
          recentTerminalTurns: [],
          activeTask: null,
          pendingInput: null,
          pendingApproval: null,
          pendingActions: [
            {
              schemaVersion: 4,
              originTurnId: "turn-alpha",
              taskId: "task-alpha",
              stepId: "step-key",
              actionRequestId: "action-malformed",
              action: "key.create",
              reports: [],
              postconditionResult: null,
              request: {
                ...actionRequest,
                stepId: "step-key",
                actionRequestId: "action-malformed",
                action: "key.create",
                params: {},
              },
            },
          ],
          recentActions: [],
        },
      },
    ).projection;

    expect(reload.actions.get("action-malformed")).toMatchObject({
      action: "key.create",
      request: null,
      supported: false,
      recovered: true,
    });
  });

  it("marks a reused action request identity with different facts as conflicted", () => {
    const first = reduceActorFrame(
      createChatActorProjection("conversation-alpha"),
      decodeActorFrame({
        sequence: 1,
        custom: { name: "nyxid.action.request", payload: actionRequest },
      }),
    );
    const conflicted = reduceActorFrame(
      first,
      decodeActorFrame({
        sequence: 2,
        custom: {
          name: "nyxid.action.request",
          payload: {
            ...actionRequest,
            action: "node.delete",
            params: {},
          },
        },
      }),
    );

    expect(conflicted.actions.get("action-alpha")).toMatchObject({
      action: "service.connect",
      conflicted: true,
    });
  });
});
