import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { AwayList } from "./nyxbot-home";

afterEach(cleanup);

const at = (minute: number) => `2026-09-29T10:${String(minute).padStart(2, "0")}:00Z`;

function specialist(index: number): AssistantAgent {
  return {
    id: `agent-${index}`,
    kind: "specialist",
    name: `agent-${index}`,
    description: "",
    display_name: null,
    persona: null,
    specialty: null,
    created_by: "user",
    status: "idle",
    services: [],
    account_read: false,
    guest_access: {},
    pending_requests: [],
    last_reply: { seq: 1, status: "completed", text: `reply ${index}`, created_at: at(index) },
    home_conversation_id: null,
    memory_count: 0,
    created_at: at(0),
    last_active_at: at(index),
    destroyed_at: null,
    pending_acknowledgements: 0,
    channels: [],
  };
}

it("shows the newest six and reveals the rest six at a time", async () => {
  const agents = Array.from({ length: 14 }, (_, index) => specialist(index + 1));
  render(<AwayList agents={agents} onOpenAgent={vi.fn()} onOpenConversation={vi.fn()} />);
  expect(screen.getAllByRole("listitem")).toHaveLength(6);
  expect(screen.getByText("reply 14")).toBeInTheDocument();
  expect(screen.queryByText("reply 8")).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "and 8 more" }));
  expect(screen.getAllByRole("listitem")).toHaveLength(12);
  await userEvent.click(screen.getByRole("button", { name: "and 2 more" }));
  expect(screen.getAllByRole("listitem")).toHaveLength(14);
  expect(screen.getByText("reply 1")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Show less" }));
  expect(screen.getAllByRole("listitem")).toHaveLength(6);
});

it("offers no toggle when everything fits", () => {
  render(<AwayList agents={[specialist(1), specialist(2)]} onOpenAgent={vi.fn()} onOpenConversation={vi.fn()} />);
  expect(screen.getAllByRole("listitem")).toHaveLength(2);
  expect(screen.queryByRole("button", { name: /^Show/ })).not.toBeInTheDocument();
});
