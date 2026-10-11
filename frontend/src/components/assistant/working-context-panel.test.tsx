import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkingContextPanel } from "./working-context-panel";

describe("WorkingContextPanel", () => {
  it("loads checkpoint in a sheet and opens memory details", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ conversation_id: "nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", goal: "Ship it", completed_actions: ["check=completed"], pending_actions: [], failed_actions: [], last_successful_turn: null, reset_reason: "turn_failed", binding_state: "bound" }) }));
    const onManageMemory = vi.fn();
    render(<WorkingContextPanel conversationId="nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" onManageMemory={onManageMemory} />);
    fireEvent.click(screen.getByText("Working context"));
    expect(await screen.findByText("Goal: Ship it")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Manage memory notes" })).toBeInTheDocument();
    expect(screen.getByText("Pending: None")).toBeInTheDocument();
    expect(screen.getByText("Binding: bound")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Manage memory notes" }));
    expect(onManageMemory).toHaveBeenCalledOnce();
  });
});
