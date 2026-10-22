import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { CmaOperation } from "@/schemas/cma-operation";
import { CmaOperationCard } from "./cma-operation-card";

const operation: CmaOperation = {
  id: "watch-1", turn_id: "turn-1", kind: "cma_response", operation_id: "resp_agt_coffee_1",
  agent_id: "agt_coffee", idempotency_key: "coffee-site-20261022", status: "in_progress",
  phase: null, state: "waiting", reason: null, monitor_name: "testtest",
  created_at: "2026-10-22T10:00:00Z", checked_at: "2026-10-22T10:01:00Z",
};

describe("CMA delegation card", () => {
  it("shows live execution identity and updates the same card when the job completes", () => {
    const { rerender } = render(<CmaOperationCard operation={operation} />);
    expect(screen.getByRole("status")).toHaveTextContent("Running");
    expect(screen.getByText("agt_coffee")).toBeInTheDocument();
    expect(screen.getByText("coffee-site-20261022")).toBeInTheDocument();
    expect(screen.getByText("testtest")).toBeInTheDocument();
    expect(screen.getByText(/Tracking progress/)).toBeInTheDocument();
    rerender(<CmaOperationCard operation={{ ...operation, status: "completed", state: "delivered", reason: "completed" }} />);
    expect(screen.getAllByRole("region", { name: "CMA delegation" })).toHaveLength(1);
    expect(screen.getByRole("status")).toHaveTextContent("Completed");
    expect(screen.getByText(/Status recorded/)).toBeInTheDocument();
  });

  it("distinguishes provisioning readiness from completed work", () => {
    render(<CmaOperationCard operation={{ ...operation, kind: "cma_provisioning", status: "succeeded", reason: "completed", state: "queued" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Agent ready");
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "CMA progress" })).getByText("Agent ready")).toBeInTheDocument();
  });

  it.each([
    [{ state: "waiting", status: "waiting_approval" }, "Needs approval"],
    [{ state: "waiting", status: "waiting_for_connection" }, "Needs connection"],
    [{ state: "waiting", status: null }, "Awaiting status"],
    [{ state: "submitting", status: null }, "Submitting"],
    [{ state: "ready", reason: "timeout" }, "Tracking timed out"],
    [{ state: "ready", reason: "authority_lost" }, "Status unavailable"],
    [{ state: "ready", reason: "submission_uncertain" }, "Submission unconfirmed"],
    [{ state: "ready", reason: "operation_failed", status: "failed" }, "Failed"],
    [{ state: "ready", reason: "result_expired", status: "failed" }, "Failed"],
    [{ state: "cancelled" }, "Tracking stopped"],
  ] as const)("reports %j without inventing success", (patch, label) => {
    render(<CmaOperationCard operation={{ ...operation, ...patch }} />);
    expect(screen.getByRole("status")).toHaveTextContent(label);
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
  });

  it("does not claim that stopping the tracker cancelled CMA", () => {
    render(<CmaOperationCard operation={{ ...operation, state: "cancelled" }} />);
    expect(screen.getByText("Tracking ended. CMA may still be running.")).toBeInTheDocument();
  });

  it("keeps tracking an accepted agent input while the agent is initially ready", () => {
    render(<CmaOperationCard operation={{ ...operation, kind: "cma_agent", operation_id: null, status: "ready" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Agent ready");
    expect(screen.getByText(/Tracking progress/)).toBeInTheDocument();
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "CMA progress" })).getByText("Submitted").querySelector("svg")).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "CMA progress" })).getByText("CMA working").querySelector("svg")).toBeNull();
  });

  it("preserves a confirmed outcome when the final result cannot be retrieved", () => {
    render(<CmaOperationCard operation={{ ...operation, status: "completed", state: "ready", reason: "result_unavailable" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Completed");
    expect(screen.getByText("CMA finished, but its final result is unavailable here.")).toBeInTheDocument();
  });
});
