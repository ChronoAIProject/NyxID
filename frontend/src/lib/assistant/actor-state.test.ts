import { describe, expect, it } from "vitest";
import {
  actorOperationGeneration,
  applyCurrentStateResult,
  createActorProjection,
  decodeActorEvent,
  reduceActorEvent,
} from "@/lib/assistant/actor-state";

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";
const SCOPE_ID = "add69059-bece-4f0e-9559-99cfd10b47eb";

function snapshot(overrides: Record<string, unknown> = {}) {
  return {
    status: "current",
    stateVersion: 7,
    turnId: "turn-1",
    snapshot: {
      actorId: ACTOR_ID,
      scopeId: SCOPE_ID,
      stateVersion: 7,
      progressSequence: 11,
      activeTurn: { turnId: "turn-1", taskId: "task-1", status: "active" },
      latestTurn: null,
      recentTerminalTurns: [],
      activeTask: {
        actorId: ACTOR_ID,
        turnId: "turn-1",
        taskId: "task-1",
        planId: "plan-1",
        planRevision: 1,
        status: "active",
        steps: [
          {
            stepId: "step-1",
            order: 1,
            status: "running",
            availableActions: { stop: true, retry: false, skip: false },
          },
        ],
      },
      taskStatus: "active",
      pendingInput: null,
      pendingApproval: null,
      pendingActions: [],
      recentActions: [],
      latestInputResolution: null,
      latestApprovalResolution: null,
      latestControlResult: null,
      latestStepControlResult: null,
      recentStepControlResults: [],
      controlFence: null,
      continuationAdmission: null,
      attentionKind: "none",
      attentionSince: null,
      activeStepSummary: null,
      canaryEffectFault: { enabled: false },
      ...overrides,
    },
  };
}

describe("typed actor projection", () => {
  it("applies the full current-state envelope while tolerating additive fields", () => {
    const result = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot(),
    );

    expect(result.status).toBe("current");
    expect(result.projection).toMatchObject({
      actorId: ACTOR_ID,
      scopeId: SCOPE_ID,
      stateVersion: 7,
      progressSequence: 11,
      taskStatus: "active",
    });
    expect(result.projection.steps.get("step-1")).toMatchObject({
      status: "running",
    });
    expect(result.projection.conflicts).toEqual([]);
  });

  it("keeps taskStatus independent when activeTask is absent", () => {
    const result = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot({ activeTask: null, taskStatus: "succeeded" }),
    );

    expect(result.projection.task).toBeNull();
    expect(result.projection.taskStatus).toBe("succeeded");
  });

  it("restores input, approval, action history, control, continuation, and step-control facts", () => {
    const result = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot({
        pendingInput: { requestId: "input-1", prompt: "Choose" },
        pendingApproval: { approvalRequestId: "approval-1" },
        recentActions: [{ actionRequestId: "action-complete-1", status: "completed" }],
        latestControlResult: { outcome: "accepted" },
        continuationAdmission: { continuationTurnId: "turn-2" },
        latestStepControlResult: { kind: "retry", outcome: "accepted" },
        recentStepControlResults: [{ kind: "skip", outcome: "rejected" }],
      }),
    );

    expect(result.projection.pendingInput?.["requestId"]).toBe("input-1");
    expect(result.projection.pendingApproval?.["approvalRequestId"]).toBe(
      "approval-1",
    );
    expect(result.projection.recentActions).toEqual([
      { actionRequestId: "action-complete-1", status: "completed" },
    ]);
    expect(result.projection.latestControlResult?.["outcome"]).toBe("accepted");
    expect(result.projection.continuation?.["continuationTurnId"]).toBe(
      "turn-2",
    );
    expect(result.projection.latestStepControlResult?.["kind"]).toBe("retry");
    expect(result.projection.recentStepControlResults).toHaveLength(1);
  });

  it("handles not_modified and requests an uncursored reload", () => {
    const current = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot(),
    ).projection;
    expect(
      applyCurrentStateResult(current, {
        status: "not_modified",
        stateVersion: 7,
      }),
    ).toMatchObject({ status: "not_modified", reloadWithoutCursor: false });
    expect(
      applyCurrentStateResult(current, { status: "reload_required" }),
    ).toMatchObject({ status: "reload_required", reloadWithoutCursor: true });
  });

  it("fails closed on identity, envelope-version, and unsafe integer conflicts", () => {
    const actorConflict = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot({ actorId: "nyxid-chat-00000000000000000000000000000000" }),
    );
    expect(actorConflict.status).toBe("invalid");
    expect(actorConflict.projection.conflicts.at(-1)?.code).toBe(
      "NYXID_ACTOR_ID_CONFLICT",
    );

    const versionConflict = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      { ...snapshot(), stateVersion: 8 },
    );
    expect(versionConflict.projection.conflicts.at(-1)?.code).toBe(
      "NYXID_STATE_VERSION_CONFLICT",
    );

    const unsafeSequence = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot({ progressSequence: Number.MAX_SAFE_INTEGER + 1 }),
    );
    expect(unsafeSequence.projection.conflicts.at(-1)?.code).toBe(
      "NYXID_STATE_VERSION_INVALID",
    );
  });

  it("converges a live snapshot and following step change", () => {
    const task = (snapshot().snapshot as Record<string, unknown>)[
      "activeTask"
    ];
    const taskEvent = decodeActorEvent("nyxid.task.snapshot", task, 11);
    expect(taskEvent).not.toBeNull();
    let projection = reduceActorEvent(
      createActorProjection(ACTOR_ID),
      taskEvent!,
    );
    const stepEvent = decodeActorEvent(
      "nyxid.task.step.changed",
      {
        taskId: "task-1",
        planRevision: 1,
        step: {
          stepId: "step-1",
          order: 1,
          status: "done",
          availableActions: { stop: false, retry: false, skip: false },
        },
      },
      12,
    );
    projection = reduceActorEvent(projection, stepEvent!);

    expect(projection.progressSequence).toBe(12);
    expect(projection.steps.get("step-1")?.["status"]).toBe("done");
  });

  it("rejects missing and unsafe live progress sequences", () => {
    expect(() =>
      decodeActorEvent("nyxid.task.snapshot", { actorId: ACTOR_ID }, undefined),
    ).toThrow("NYXID_SEQUENCE_INVALID");
    expect(() =>
      decodeActorEvent(
        "nyxid.task.snapshot",
        { actorId: ACTOR_ID },
        Number.MAX_SAFE_INTEGER + 1,
      ),
    ).toThrow("NYXID_SEQUENCE_INVALID");
  });

  it("rejects actor changes and plan-revision rollback in live task facts", () => {
    const initial = decodeActorEvent(
      "nyxid.task.snapshot",
      {
        actorId: ACTOR_ID,
        taskId: "task-1",
        planRevision: 3,
        steps: [],
      },
      1,
    )!;
    let projection = reduceActorEvent(createActorProjection(ACTOR_ID), initial);
    projection = reduceActorEvent(
      projection,
      decodeActorEvent(
        "nyxid.task.step.changed",
        {
          taskId: "task-1",
          planRevision: 2,
          step: { stepId: "step-1", order: 1 },
        },
        2,
      )!,
    );
    expect(projection.conflicts.at(-1)?.code).toBe(
      "NYXID_PLAN_REVISION_CONFLICT",
    );

    projection = reduceActorEvent(
      projection,
      decodeActorEvent(
        "nyxid.task.snapshot",
        {
          actorId: "nyxid-chat-00000000000000000000000000000000",
          taskId: "task-2",
          planRevision: 1,
          steps: [],
        },
        3,
      )!,
    );
    expect(projection.conflicts.at(-1)?.code).toBe(
      "NYXID_ACTOR_ID_CONFLICT",
    );
  });

  it("orders equal-rank steps by identity and rejects unsafe operation generations", () => {
    const result = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot({
        activeTask: {
          actorId: ACTOR_ID,
          turnId: "turn-1",
          taskId: "task-1",
          planRevision: 1,
          status: "active",
          steps: [
            { stepId: "step-b", order: 1 },
            { stepId: "step-a", order: 1 },
          ],
        },
      }),
    );
    expect([...result.projection.steps.keys()]).toEqual(["step-a", "step-b"]);
    expect(
      actorOperationGeneration({
        operation: { key: { operationGeneration: Number.MAX_SAFE_INTEGER + 1 } },
      }),
    ).toBeNull();
    expect(
      actorOperationGeneration({ operation: { key: { operationGeneration: 4 } } }),
    ).toBe(4);
  });

  it("keeps state-only action summaries non-executable", () => {
    const result = applyCurrentStateResult(
      createActorProjection(ACTOR_ID),
      snapshot({
        pendingActions: [
          {
            schemaVersion: 4,
            originTurnId: "turn-1",
            taskId: "task-1",
            stepId: "step-1",
            actionRequestId: "action-1",
            action: "service.connect",
          },
        ],
      }),
    );

    expect(result.projection.actions.get("action-1")).toMatchObject({
      request: null,
      executable: false,
      conflicted: false,
    });
  });
});
