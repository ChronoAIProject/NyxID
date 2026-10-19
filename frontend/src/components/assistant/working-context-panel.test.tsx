import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WorkingContextPanel } from "./working-context-panel";

const ID = "nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const context = {
  conversation_id: ID,
  agent_id: "agent-1",
  binding_state: "reset",
  reset_reason: "turn_failed",
  active_turn_id: null,
  pending_actions: [],
  checkpoint: {
    turn_id: "t2",
    status: "failed",
    error_code: "idle_timeout",
    goal: "Ship it",
    completed_actions: [{ label: "mail__send", turn_id: "t2" }],
    failed_actions: [{ label: "drive__upload", turn_id: "t2" }],
    last_successful_turn_id: "t1",
    last_successful_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:01:00Z",
  },
};
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function renderPanel(props: Partial<ComponentProps<typeof WorkingContextPanel>> = {}) {
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <WorkingContextPanel conversationId={ID} {...props} />
    </QueryClientProvider>,
  );
}

afterEach(() => vi.unstubAllGlobals());

describe("WorkingContextPanel", () => {
  it("shows the checkpoint and opens the thread agent's memory", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(json(context)));
    const onManageMemory = vi.fn();
    renderPanel({ onManageMemory });
    fireEvent.click(screen.getByText("Working context"));
    expect(await screen.findByText("Goal: Ship it")).toBeInTheDocument();
    expect(screen.getByText(/Last checkpoint: failed \(idle_timeout\)/)).toBeInTheDocument();
    expect(screen.getByText(/Last successful checkpoint:/)).toHaveTextContent("Last successful checkpoint: 1/1/2026");
    expect(screen.getByText("Completed since then: mail__send")).toBeInTheDocument();
    expect(screen.getByText("Failed or interrupted: drive__upload")).toBeInTheDocument();
    expect(screen.getByText("Running now: None")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Manage memory notes" }));
    expect(onManageMemory).toHaveBeenCalledWith("agent-1");
  });

  it("confirms a reset, then refreshes the transcript", async () => {
    const fetch = vi.fn()
      .mockResolvedValueOnce(json(context))
      .mockResolvedValueOnce(json({ ...context, reset_reason: "user_reset", checkpoint: null }));
    vi.stubGlobal("fetch", fetch);
    const onReset = vi.fn();
    renderPanel({ onReset });
    fireEvent.click(screen.getByText("Working context"));
    fireEvent.click(await screen.findByRole("button", { name: "Reset context" }));
    expect(fetch).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Confirm reset" }));
    await waitFor(() => expect(onReset).toHaveBeenCalledOnce());
    expect(String(fetch.mock.calls[1]?.[0])).toContain(`/conversations/${ID}/context/reset`);
    expect(await screen.findByText("Goal: No goal recorded")).toBeInTheDocument();
    expect(screen.getByText("Last reset: You reset the working context.")).toBeInTheDocument();
  });

  it("shows a reset failure instead of failing silently", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn()
        .mockResolvedValueOnce(json(context))
        .mockResolvedValueOnce(json({ error: "turn_active", error_code: 12100, message: "busy" }, 409)),
    );
    const onReset = vi.fn();
    renderPanel({ onReset });
    fireEvent.click(screen.getByText("Working context"));
    fireEvent.click(await screen.findByRole("button", { name: "Reset context" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm reset" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Unable to reset");
    expect(onReset).not.toHaveBeenCalled();
  });
});
