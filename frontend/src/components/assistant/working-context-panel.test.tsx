import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { WorkingContextPanel } from "./working-context-panel";

describe("WorkingContextPanel", () => {
  it("loads checkpoint, offers reset, and links to memory", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, json: async () => ({ conversation_id: "nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", goal: "Ship it", completed_actions: ["check=completed"], pending_actions: [], failed_actions: [], last_successful_turn: null, reset_reason: "turn_failed", binding_state: "bound" }) }));
    render(<WorkingContextPanel conversationId="nyxa-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" />);
    fireEvent.click(screen.getByText("Working context"));
    expect(await screen.findByText("Goal: Ship it")).toBeTruthy();
    expect(screen.getByText("Manage memory notes")).toHaveAttribute("href", "/assistant?panel=memory");
  });
});
