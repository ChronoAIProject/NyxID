import { describe, expect, it } from "vitest";
import {
  applyCurrentTaskState,
  createTaskProjection,
  reduceTaskFrame,
  taskCan,
  TaskStateProtocolError,
} from "./task-state";

const ACTOR_ID = "nyxid-chat-actor-alpha";

function step(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    stepId: "step-alpha",
    order: 1,
    kind: "tool",
    status: "running",
    required: true,
    description: "Read repository history",
    source: { tool: { toolName: "github_history", serviceSlug: "api-github" } },
    mayChangeExternalState: false,
    externalEffect: "not_started",
    availableActions: { retry: true, skip: true, stop: true },
    dependsOn: [],
    substeps: [],
    operation: {
      turnId: "turn-alpha",
      taskId: "task-alpha",
      stepId: "step-alpha",
      operationId: "operation-alpha",
      operationGeneration: 3,
    },
    ...overrides,
  };
}

function plan(
  stepOverrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    schemaVersion: 4,
    actorId: ACTOR_ID,
    taskId: "task-alpha",
    turnId: "turn-alpha",
    planId: "plan-alpha",
    planRevision: 2,
    planRevisions: [],
    title: "Research dinner options",
    status: "active",
    activeStepId: "step-alpha",
    gate: { mode: "auto", status: "satisfied" },
    steps: [step(stepOverrides)],
  };
}

function currentState(
  stateVersion = 11,
  progressSequence = 5,
): Record<string, unknown> {
  return {
    status: "current",
    stateVersion,
    snapshot: {
      actorId: ACTOR_ID,
      stateVersion,
      progressSequence,
      activeTurn: { turnId: "turn-alpha", status: "active" },
      latestTurn: { turnId: "turn-alpha", status: "active" },
      activeTask: plan(),
      pendingInput: { requestId: "input-alpha" },
      pendingApproval: { approvalRequestId: "approval-alpha" },
      pendingActions: [{ actionRequestId: "action-alpha" }],
      recentActions: [{ actionRequestId: "action-alpha" }],
    },
  };
}

describe("assistant task projection", () => {
  it("produces the same TaskPlan model from live snapshot and current state", () => {
    const live = reduceTaskFrame(
      createTaskProjection(ACTOR_ID),
      "nyxid.task.snapshot",
      plan(),
      5,
    );
    const reloaded = applyCurrentTaskState(
      createTaskProjection(ACTOR_ID),
      currentState(),
    ).projection;

    expect(reloaded.task).toEqual(live.task);
    expect([...reloaded.steps]).toEqual([...live.steps]);
    expect(reloaded.pendingActions).toHaveLength(1);
  });

  it("applies only newer exact-plan step changes", () => {
    const snapshot = reduceTaskFrame(
      createTaskProjection(ACTOR_ID),
      "nyxid.task.snapshot",
      plan(),
      5,
    );
    const changed = reduceTaskFrame(
      snapshot,
      "nyxid.task.step.changed",
      {
        taskId: "task-alpha",
        planRevision: 2,
        changeKind: "status",
        step: step({ status: "done", externalEffect: "not_applied" }),
      },
      6,
    );

    expect(changed.steps.get("step-alpha")?.status).toBe("done");
    expect(reduceTaskFrame(changed, "nyxid.task.snapshot", plan(), 6)).toBe(
      changed,
    );
    expect(() =>
      reduceTaskFrame(
        changed,
        "nyxid.task.step.changed",
        {
          taskId: "task-other",
          planRevision: 2,
          changeKind: "status",
          step: step(),
        },
        7,
      ),
    ).toThrow(TaskStateProtocolError);
  });

  it("never rolls state or progress backward and exposes only actor actions", () => {
    const current = applyCurrentTaskState(
      createTaskProjection(ACTOR_ID),
      currentState(),
    ).projection;
    const stale = applyCurrentTaskState(
      current,
      currentState(10, 4),
    ).projection;

    expect(stale).toBe(current);
    expect(taskCan(current, "stop")).toBe(true);
    expect(taskCan(current, "retry", "step-alpha")).toBe(true);
    expect(taskCan(current, "skip", "missing-step")).toBe(false);
  });

  it("handles conditional current-state statuses explicitly", () => {
    const current = applyCurrentTaskState(
      createTaskProjection(ACTOR_ID),
      currentState(),
    ).projection;
    expect(
      applyCurrentTaskState(current, {
        status: "not_modified",
        stateVersion: 11,
      }),
    ).toEqual({ projection: current, reload: false });
    expect(
      applyCurrentTaskState(current, { status: "reload_required" }).reload,
    ).toBe(true);
    expect(
      applyCurrentTaskState(current, { status: "not_found" }).projection.task,
    ).toBeNull();
    expect(() =>
      applyCurrentTaskState(current, {
        ...currentState(),
        snapshot: {
          ...(currentState().snapshot as Record<string, unknown>),
          actorId: "nyxid-chat-other",
        },
      }),
    ).toThrow("different conversation");
  });

  it("rejects a live TaskPlan for a different conversation actor", () => {
    expect(() =>
      reduceTaskFrame(
        createTaskProjection("nyxid-chat-actor-other"),
        "nyxid.task.snapshot",
        plan(),
        1,
      ),
    ).toThrow("different conversation");
  });
});
