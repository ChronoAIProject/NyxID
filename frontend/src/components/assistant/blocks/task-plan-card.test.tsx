import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { decodeChatTaskPlan } from "@/lib/assistant/chat-task-plan";
import { TaskPlanCard } from "./task-plan-card";

function block(
  actions = { retry: true, skip: true, stop: true },
  gate: Record<string, unknown> = {
    mode: "auto",
    status: "satisfied",
    reason: "Read-only steps run automatically.",
  },
) {
  return {
    type: "task_plan" as const,
    block_id: "task-plan-alpha",
    state_version: 17,
    progress_sequence: 9,
    plan: decodeChatTaskPlan({
      schemaVersion: 4,
      actorId: "nyxid-chat-actor-alpha",
      taskId: "task-alpha",
      turnId: "turn-alpha",
      planId: "plan-alpha",
      planRevision: 3,
      planRevisions: [],
      title: "Publish a weekly update",
      status: "active",
      gate,
      steps: [
        {
          stepId: "step-alpha",
          order: 1,
          kind: "tool",
          status: "uncertain",
          required: true,
          description: "Read repository history",
          source: {
            tool: { toolName: "github_history", serviceSlug: "api-github" },
          },
          mayChangeExternalState: false,
          externalEffect: "may_have_changed",
          availableActions: actions,
          dependsOn: [],
          substeps: [
            {
              substepId: "substep-alpha",
              title: "Inspect merged changes",
              status: "done",
            },
          ],
          operation: {
            operationId: "operation-alpha",
            operationGeneration: 2,
            kind: "tool",
            phase: "verify",
          },
        },
      ],
    }),
  };
}

describe("TaskPlanCard", () => {
  it("renders complete task facts and dispatches only actor-authorized controls", async () => {
    const onStop = vi.fn().mockResolvedValue(undefined);
    const onRetry = vi.fn().mockResolvedValue(undefined);
    const onSkip = vi.fn().mockResolvedValue(undefined);
    render(
      <TaskPlanCard
        block={block()}
        onStop={onStop}
        onRetry={onRetry}
        onSkip={onSkip}
      />,
    );

    expect(screen.getByText("Publish a weekly update")).toBeInTheDocument();
    expect(
      screen.getByText(/Revision 3 \/ state 17 \/ sequence 9/),
    ).toBeInTheDocument();
    expect(screen.getByText("github_history / api-github")).toBeInTheDocument();
    expect(screen.getByText("may have changed")).toBeInTheDocument();
    expect(screen.getByText("Inspect merged changes")).toBeInTheDocument();
    expect(
      screen.getByText(/operation-alpha \/ generation 2/),
    ).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Retry Read repository history" }),
      );
    });
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Skip Read repository history" }),
      );
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Stop task" }));
    });
    expect(onRetry).toHaveBeenCalledWith("step-alpha");
    expect(onSkip).toHaveBeenCalledWith("step-alpha");
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("does not render controls the actor did not offer", () => {
    render(
      <TaskPlanCard
        block={block({ retry: false, skip: false, stop: false })}
        onStop={vi.fn()}
        onRetry={vi.fn()}
        onSkip={vi.fn()}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /Retry/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Skip/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Stop task" }),
    ).not.toBeInTheDocument();
  });

  it("renders a historical confirm gate as a readable plan with no confirm controls", () => {
    render(
      <TaskPlanCard
        block={block(
          { retry: false, skip: false, stop: false },
          {
            mode: "confirm",
            status: "pending",
            requestId: "plan-gate-alpha",
            taskId: "task-alpha",
            planId: "plan-alpha",
            planRevision: 3,
          },
        )}
        onStop={vi.fn()}
        onRetry={vi.fn()}
        onSkip={vi.fn()}
      />,
    );

    expect(screen.getByText("Publish a weekly update")).toBeInTheDocument();
    expect(screen.getByText("confirm / pending")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Confirm plan" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Reject plan" }),
    ).not.toBeInTheDocument();
  });

  it("disables plan, step, and stop controls behind the state-version fence", () => {
    render(
      <TaskPlanCard
        block={block(
          { retry: true, skip: true, stop: true },
          {
            mode: "confirm",
            status: "pending",
            requestId: "plan-gate-alpha",
            taskId: "task-alpha",
            planId: "plan-alpha",
            planRevision: 3,
          },
        )}
        disabled
        onStop={vi.fn()}
        onRetry={vi.fn()}
        onSkip={vi.fn()}
      />,
    );

    for (const button of screen.getAllByRole("button")) {
      expect(button).toBeDisabled();
    }
  });
});
