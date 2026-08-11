import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { decodeTaskPlan } from "@/lib/assistant/task-plan";
import type { TaskPlanContentBlock } from "@/types/assistant";
import { TaskPlanCard } from "./task-plan-card";

function block(
  actions = { retry: true, skip: true, stop: true },
): TaskPlanContentBlock {
  return {
    type: "task_plan",
    block_id: "task-plan-alpha",
    state_version: 17,
    progress_sequence: 9,
    plan: decodeTaskPlan({
      schemaVersion: 4,
      actorId: "nyxid-chat-actor-alpha",
      taskId: "task-alpha",
      turnId: "turn-alpha",
      planId: "plan-alpha",
      planRevision: 3,
      planRevisions: [],
      title: "Publish a weekly update",
      status: "active",
      gate: {
        mode: "auto",
        status: "satisfied",
        reason: "Read-only steps run automatically.",
      },
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
});
