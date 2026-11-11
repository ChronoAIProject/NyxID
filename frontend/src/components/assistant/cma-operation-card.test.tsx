import type { ReactNode } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  act,
  fireEvent,
  render as renderUi,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CmaOperation } from "@/schemas/cma-operation";
import { CmaOperationCard } from "./cma-operation-card";

const flags = vi.hoisted(() => ({ cma: false }));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => flags.cma }));
beforeEach(() => {
  flags.cma = false;
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-10-22T10:01:00Z"));
});
afterEach(() => {
  vi.useRealTimers();
});

const render = (ui: ReactNode) => renderUi(ui, { wrapper: TooltipProvider });

const operation: CmaOperation = {
  id: "watch-1",
  turn_id: "turn-1",
  kind: "cma_response",
  operation_id: "resp_agt_coffee_1",
  agent_id: "agt_coffee",
  idempotency_key: "coffee-site-20261022",
  status: "in_progress",
  phase: null,
  state: "waiting",
  reason: null,
  monitor_name: "testtest",
  created_at: "2026-10-22T10:00:00Z",
  checked_at: "2026-10-22T10:01:00Z",
};

describe("CMA submission card", () => {
  it("warns about elapsed time without declaring a running request failed", () => {
    vi.setSystemTime(new Date("2026-10-22T10:11:00Z"));
    render(
      <CmaOperationCard
        operation={{ ...operation, checked_at: "2026-10-22T10:10:59Z" }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Taking longer than usual",
    );
    expect(
      screen.getByText(/CMA still reports it as running/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/ask NyxBot to cancel this task/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Tracking progress/)).toBeInTheDocument();
    expect(screen.queryByText(/no progress/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Failed")).not.toBeInTheDocument();
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
  });

  it("updates the elapsed-time notice while open and removes it on completion", () => {
    vi.setSystemTime(new Date("2026-10-22T10:09:00Z"));
    const { rerender, unmount } = render(
      <CmaOperationCard operation={operation} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Working");
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.getByRole("status")).toHaveTextContent(
      "Taking longer than usual",
    );
    rerender(
      <CmaOperationCard
        operation={{
          ...operation,
          status: "completed",
          state: "delivered",
          reason: "completed",
        }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Completed");
    expect(
      screen.queryByText(/CMA still reports it as running/),
    ).not.toBeInTheDocument();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([
    { status: "waiting_approval" },
    { status: "waiting_for_connection" },
    { status: "queued" },
    { status: "paused" },
    { state: "cancelled" },
    { state: "cancelling" },
    { state: "ready", reason: "timeout" },
    { kind: "cma_provisioning" as const, status: "running" },
    { created_at: "invalid" },
    { created_at: "2026-10-23T10:00:00Z" },
  ])(
    "preserves specific states instead of adding an elapsed-time warning: %j",
    (patch) => {
      vi.setSystemTime(new Date("2026-10-22T10:30:00Z"));
      render(<CmaOperationCard operation={{ ...operation, ...patch }} />);
      expect(
        screen.queryByText("Taking longer than usual"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByText(/CMA still reports it as running/),
      ).not.toBeInTheDocument();
    },
  );

  it("explains that a tracking timeout does not stop the remote task", () => {
    render(
      <CmaOperationCard
        operation={{ ...operation, state: "delivered", reason: "timeout" }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Tracking timed out");
    expect(screen.getByText(/CMA may still be running/)).toBeInTheDocument();
    expect(screen.getByText(/before submitting it again/)).toBeInTheDocument();
  });

  it("offers the internal Agent panel when a local reference is available", () => {
    flags.cma = true;
    render(
      <CmaOperationCard
        operation={{
          ...operation,
          agent_ref_id: "0dc85e4c-6aaf-47de-9df0-e07a69b8ee19",
          response_id: "rsp-one",
        }}
      />,
    );
    expect(
      screen.getByRole("button", { name: /View execution/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: /View execution/ }),
    ).not.toBeInTheDocument();
  });
  it("does not invent an internal reference from a remote Agent ID", () => {
    flags.cma = true;
    render(<CmaOperationCard operation={operation} />);
    expect(
      screen.queryByRole("button", { name: /View execution/ }),
    ).not.toBeInTheDocument();
  });
  it("hides the internal execution entry when the feature is disabled", () => {
    render(
      <CmaOperationCard
        operation={{
          ...operation,
          agent_ref_id: "0dc85e4c-6aaf-47de-9df0-e07a69b8ee19",
          response_id: "rsp-one",
        }}
      />,
    );
    expect(
      screen.queryByRole("button", { name: /View execution/ }),
    ).not.toBeInTheDocument();
  });
  it("uses the dedicated card controls without inventing session activity for an unconfirmed submission", () => {
    render(
      <CmaOperationCard
        operation={{
          ...operation,
          agent_id: null,
          operation_id: null,
          reason: "submission_uncertain",
          state: "delivered",
          status: null,
        }}
      />,
    );
    expect(screen.queryByText("Delegated to CMA")).not.toBeInTheDocument();
    expect(screen.queryByText("Recent actions")).not.toBeInTheDocument();
    expect(screen.queryByText("Live")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Pin CMA/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Reconnect/ }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Session ID")).not.toBeInTheDocument();
    const details = screen.getByRole("button", { name: "Submission details" });
    expect(details).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: "Minimise CMA submission" }),
    );
    expect(details).not.toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Submission unconfirmed",
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Maximise CMA submission" }),
    );
    expect(details).toBeVisible();
  });

  it("keeps recorded identity in details and updates the same card when the job completes", () => {
    const { rerender } = render(<CmaOperationCard operation={operation} />);
    expect(screen.getByRole("status")).toHaveTextContent("Working");
    expect(screen.getByText("agt_coffee")).not.toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Submission details" }));
    expect(screen.getByText("agt_coffee")).toBeVisible();
    expect(screen.getByText("coffee-site-20261022")).toBeInTheDocument();
    expect(screen.getByText("testtest")).toBeInTheDocument();
    expect(screen.getByText(/Tracking progress/)).toBeInTheDocument();
    rerender(
      <CmaOperationCard
        operation={{
          ...operation,
          status: "completed",
          state: "delivered",
          reason: "completed",
        }}
      />,
    );
    expect(
      screen.getAllByRole("region", { name: "CMA submission: CMA task" }),
    ).toHaveLength(1);
    expect(screen.getByRole("status")).toHaveTextContent("Completed");
    expect(screen.getByText(/Status recorded/)).toBeInTheDocument();
  });

  it("distinguishes provisioning readiness from completed work", () => {
    render(
      <CmaOperationCard
        operation={{
          ...operation,
          kind: "cma_provisioning",
          status: "succeeded",
          reason: "completed",
          state: "queued",
        }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Ready");
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "CMA setup" })).toBeVisible();
    expect(
      screen.queryByRole("list", { name: "CMA progress" }),
    ).not.toBeInTheDocument();
  });

  it.each([
    ["sandbox_ready", "Preparing sandbox"],
    ["desktop_preparation", "Preparing desktop"],
  ])(
    "shows CMA's %s phase without claiming the Agent is ready",
    (phase, label) => {
      render(
        <CmaOperationCard
          operation={{
            ...operation,
            kind: "cma_provisioning",
            status: "running",
            phase,
          }}
        />,
      );
      expect(screen.getByRole("status")).toHaveTextContent("Starting");
      expect(screen.getByText(label)).toBeInTheDocument();
      expect(screen.queryByText("Ready")).not.toBeInTheDocument();
    },
  );

  it.each([
    [{ state: "waiting", status: "waiting_approval" }, "Needs approval"],
    [
      { state: "waiting", status: "waiting_for_connection" },
      "Needs connection",
    ],
    [{ state: "waiting", status: null }, "Awaiting status"],
    [{ state: "submitting", status: null }, "Submitting"],
    [{ state: "ready", reason: "timeout" }, "Tracking timed out"],
    [{ state: "ready", reason: "authority_lost" }, "Status unavailable"],
    [
      { state: "ready", reason: "submission_uncertain" },
      "Submission unconfirmed",
    ],
    [
      { state: "ready", reason: "operation_failed", status: "failed" },
      "Failed",
    ],
    [{ state: "ready", reason: "result_expired", status: "failed" }, "Failed"],
    [{ state: "cancelled" }, "Tracking stopped"],
  ] as const)("reports %j without inventing success", (patch, label) => {
    render(<CmaOperationCard operation={{ ...operation, ...patch }} />);
    expect(screen.getByRole("status")).toHaveTextContent(label);
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
  });

  it("does not claim that stopping the tracker cancelled CMA", () => {
    render(
      <CmaOperationCard operation={{ ...operation, state: "cancelled" }} />,
    );
    expect(
      screen.getByText("Tracking ended. CMA may still be running."),
    ).toBeInTheDocument();
  });

  it("distinguishes a failed creation submission from an uncertain outcome", () => {
    const failed: CmaOperation = {
      ...operation,
      kind: "cma_provisioning",
      operation_id: null,
      agent_id: null,
      status: null,
      state: "delivered",
      reason: "submission_failed",
    };
    const { rerender } = render(<CmaOperationCard operation={failed} />);
    expect(screen.getByRole("status")).toHaveTextContent("Submission failed");
    expect(
      screen.queryByText(/final outcome could not be confirmed/),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.queryByText("Ready")).not.toBeInTheDocument();

    rerender(
      <CmaOperationCard
        operation={{ ...failed, reason: "submission_uncertain" }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Submission unconfirmed",
    );
    expect(
      screen.getByText(/final outcome could not be confirmed/),
    ).toBeInTheDocument();
  });

  it("distinguishes confirmed response cancellation from an ordinary incomplete response", () => {
    const { rerender } = render(
      <CmaOperationCard
        operation={{
          ...operation,
          state: "delivered",
          status: "incomplete",
          reason: "operation_cancelled",
        }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Cancelled");
    expect(screen.queryByText(/may still be running/)).not.toBeInTheDocument();
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
    rerender(
      <CmaOperationCard
        operation={{
          ...operation,
          state: "delivered",
          status: "incomplete",
          reason: "operation_failed",
        }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Incomplete");
  });

  it("keeps tracking an accepted agent input while the agent is initially ready", () => {
    render(
      <CmaOperationCard
        operation={{
          ...operation,
          kind: "cma_agent",
          operation_id: null,
          status: "ready",
        }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Ready");
    expect(screen.getByText(/Tracking progress/)).toBeInTheDocument();
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
    expect(screen.queryByText("Live")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Pin CMA/ }),
    ).not.toBeInTheDocument();
  });

  it("preserves a confirmed outcome when the final result cannot be retrieved", () => {
    render(
      <CmaOperationCard
        operation={{
          ...operation,
          status: "completed",
          state: "ready",
          reason: "result_unavailable",
        }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Completed");
    expect(
      screen.getByText(
        "CMA finished, but its final result is unavailable here.",
      ),
    ).toBeInTheDocument();
  });

  it.each([
    ["starting", "Starting"],
    ["working", "Working"],
    ["ready", "Ready"],
    ["failed", "Needs attention"],
    ["stopped", "Stopped"],
    ["deleted", "Deleted"],
  ])("matches CMA's agent label for %s", (status, label) => {
    render(
      <CmaOperationCard
        operation={{ ...operation, kind: "cma_agent", status }}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(label);
  });

  it.each([
    "https://cma.example/agents/agt_coffee",
    "https://staging.example:8443/agents/agt_coffee",
    "http://localhost:8080/agents/agt_coffee",
  ])("links prominently to CMA's supplied callback: %s", (cmaUrl) => {
    render(<CmaOperationCard operation={{ ...operation, cma_url: cmaUrl }} />);
    const link = screen.getByRole("link", { name: /View in CMA/ });
    expect(link).toHaveAttribute("href", cmaUrl);
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  it.each([
    "https://example.com/agent?secret=value",
    "https://example.com/agent#secret",
    null,
    undefined,
    "",
    "/agents/agt_coffee",
    "//example.com/agents/agt_coffee",
    "javascript:alert(1)",
    "data:text/html,secret",
    "https://user:secret@example.com/agents/agt_coffee",
    " https://example.com/agents/agt_coffee",
    "https://example.com/agents/agt_\ncoffee",
    "https://example.com/agents/agt_\\coffee",
    "http://metadata.google.internal/secret",
    `https://example.com/${"a".repeat(2048)}`,
  ])(
    "does not render an unavailable or unsafe CMA destination: %s",
    (cmaUrl) => {
      render(
        <CmaOperationCard operation={{ ...operation, cma_url: cmaUrl }} />,
      );
      expect(
        screen.queryByRole("link", { name: /View in CMA/ }),
      ).not.toBeInTheDocument();
    },
  );

  it("updates link availability without changing the observed status", () => {
    const { rerender } = render(
      <CmaOperationCard operation={{ ...operation, cma_url: null }} />,
    );
    expect(
      screen.queryByRole("link", { name: /View in CMA/ }),
    ).not.toBeInTheDocument();
    rerender(
      <CmaOperationCard
        operation={{
          ...operation,
          cma_url: "https://cma.example/agents/agt_coffee",
        }}
      />,
    );
    expect(
      screen.getByRole("link", { name: /View in CMA/ }),
    ).toBeInTheDocument();
    rerender(<CmaOperationCard operation={{ ...operation, cma_url: null }} />);
    expect(
      screen.queryByRole("link", { name: /View in CMA/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Working");
  });

  it("shows the check date so a historical status cannot look like today's check", () => {
    render(<CmaOperationCard operation={operation} />);
    fireEvent.click(screen.getByRole("button", { name: "Submission details" }));
    const checked = screen.getByText(
      new Date(operation.checked_at!).toLocaleString(),
    );
    expect(checked).toBeVisible();
    expect(checked).toHaveAttribute("datetime", operation.checked_at);
  });

  it.each(["ready", "succeeded", "unknown"])(
    "does not present a response status of %s as completed work",
    (status) => {
      render(<CmaOperationCard operation={{ ...operation, status }} />);
      expect(screen.getByRole("status")).toHaveTextContent(
        "Status unavailable",
      );
      expect(screen.queryByText("Completed")).not.toBeInTheDocument();
    },
  );
});
