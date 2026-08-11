import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createActorProjection, type ActorProjection } from "@/lib/assistant/actor-state";
import { ActorTaskPanel } from "./actor-task-panel";

const ACTOR_ID = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";

const callbacks = {
  onResolvePlan: vi.fn().mockResolvedValue(undefined),
  onResolveInput: vi.fn().mockResolvedValue(undefined),
  onResolveApproval: vi.fn().mockResolvedValue(undefined),
  onStop: vi.fn().mockResolvedValue(undefined),
  onSteer: vi.fn().mockResolvedValue(undefined),
  onRetry: vi.fn().mockResolvedValue(undefined),
  onSkip: vi.fn().mockResolvedValue(undefined),
};

function projection(overrides: Partial<ActorProjection> = {}): ActorProjection {
  return {
    ...createActorProjection(ACTOR_ID),
    scopeId: "user-1",
    stateVersion: 12,
    ...overrides,
  };
}

function renderPanel(value: ActorProjection) {
  return render(
    <ActorTaskPanel projection={value} busy={false} {...callbacks} />,
  );
}

describe("ActorTaskPanel", () => {
  beforeEach(() => {
    for (const callback of Object.values(callbacks)) callback.mockClear();
  });

  it("renders plan decisions only for an exact confirm/pending gate", async () => {
    const task = {
      taskId: "task-1",
      planId: "plan-1",
      planRevision: 3,
      status: "active",
      title: "Review the rollout",
      gate: {
        mode: "auto",
        status: "pending",
        requestId: "gate-1",
      },
    };
    const { rerender } = renderPanel(projection({ task }));

    expect(screen.queryByRole("button", { name: "Confirm" })).not.toBeInTheDocument();

    rerender(
      <ActorTaskPanel
        projection={
          projection({
            task: { ...task, gate: { ...task.gate, mode: "confirm" } },
          })
        }
        busy={false}
        {...callbacks}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Confirm" }));

    expect(callbacks.onResolvePlan).toHaveBeenCalledWith(true);
  });

  it("submits a restored input with its exact actor request identity", async () => {
    renderPanel(
      projection({
        pendingInput: {
          requestId: "input-restored-1",
          prompt: "Choose a region",
          options: [{ optionId: "sg", label: "Singapore" }],
          allowFreeText: false,
          multiSelect: false,
        },
      }),
    );

    await userEvent.click(screen.getByRole("button", { name: "Singapore" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Submit input response" }),
    );

    expect(callbacks.onResolveInput).toHaveBeenCalledWith(
      "input-restored-1",
      { selectedOptionIds: ["sg"] },
    );
  });

  it("offers approval only inside the actor-authored grant boundary", async () => {
    const pendingApproval = {
      approvalRequestId: "approval-1",
      action: "Publish release",
      target: "Production",
      grantBoundary: "nyxid_step_up",
    };
    const { rerender } = renderPanel(projection({ pendingApproval }));

    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();

    rerender(
      <ActorTaskPanel
        projection={
          projection({
            pendingApproval: {
              ...pendingApproval,
              grantBoundary: "within_grant",
            },
          })
        }
        busy={false}
        {...callbacks}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));

    expect(callbacks.onResolveApproval).toHaveBeenCalledWith(
      "approval-1",
      true,
    );
  });

  it("exposes step and stop controls only from actor-authored availability", async () => {
    const runnable = {
      stepId: "step-runnable",
      description: "Retryable step",
      status: "failed",
      availableActions: { retry: true, skip: true, stop: true },
    };
    const locked = {
      stepId: "step-locked",
      description: "Locked step",
      status: "failed",
      availableActions: { retry: false, skip: false, stop: false },
    };
    renderPanel(
      projection({
        task: {
          taskId: "task-1",
          planId: "plan-1",
          planRevision: 1,
          status: "active",
        },
        taskStatus: "active",
        steps: new Map([
          ["step-runnable", runnable],
          ["step-locked", locked],
        ]),
      }),
    );

    expect(screen.getAllByRole("button", { name: "Retry step" })).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Skip step" })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Stop active task" })).toBeEnabled();

    await userEvent.click(screen.getByRole("button", { name: "Retry step" }));
    await userEvent.click(screen.getByRole("button", { name: "Skip step" }));
    await userEvent.click(screen.getByRole("button", { name: "Stop active task" }));

    expect(callbacks.onRetry).toHaveBeenCalledWith("step-runnable");
    expect(callbacks.onSkip).toHaveBeenCalledWith("step-runnable");
    expect(callbacks.onStop).toHaveBeenCalledOnce();
  });

  it("renders state-only action summaries as unavailable", () => {
    renderPanel(
      projection({
        actions: new Map([
          [
            "action-1",
            {
              actionRequestId: "action-1",
              action: "service.connect",
              executable: false,
            },
          ],
        ]),
      }),
    );

    expect(
      screen.getByText("service.connect - unavailable after reload"),
    ).toBeInTheDocument();
  });
});
