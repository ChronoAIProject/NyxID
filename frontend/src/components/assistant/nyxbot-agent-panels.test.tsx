import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { PendingEventsNote, TeamStrip, ThreadHeader, WaitingNote } from "./nyxbot-agent-panels";

afterEach(cleanup);

const at = "2026-09-28T00:00:00Z";
const thread = `nyxa-${"b".repeat(32)}`;

function agent(fields: Partial<AssistantAgent>): AssistantAgent {
  return {
    id: "agent-nyxbot",
    kind: "nyxbot",
    name: "NyxBot",
    description: "",
    display_name: null,
    persona: null,
    specialty: null,
    created_by: "user",
    status: "idle",
    services: [],
    account_read: true,
    guest_access: {},
    pending_requests: [],
    last_reply: null,
    home_conversation_id: null,
    memory_count: 0,
    created_at: at,
    last_active_at: at,
    destroyed_at: null,
    pending_acknowledgements: 0,
    channels: [],
    ...fields,
  };
}

it("names the thread's agent with its kind and opens its details", async () => {
  const open = vi.fn();
  render(<ThreadHeader name="researcher" kind="specialist" destroyed={false} onOpenDetails={open} />);
  expect(screen.getByRole("heading")).toHaveTextContent("researcher");
  expect(screen.getByText("Specialist")).toBeVisible();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  await userEvent.setup().click(screen.getByRole("button", { name: "Agent details" }));
  expect(open).toHaveBeenCalledOnce();
});

it("marks a destroyed agent's thread read-only", () => {
  render(<ThreadHeader name="researcher" kind="specialist" destroyed onOpenDetails={vi.fn()} />);
  expect(screen.getByRole("status")).toHaveTextContent("this thread is read-only");
});

it("shows NyxBot's kind badge", () => {
  render(<ThreadHeader name="NyxBot" kind="nyxbot" destroyed={false} />);
  expect(screen.getAllByText("NyxBot")).toHaveLength(2);
});

it("lists live specialists with their latest reply and opens a request's thread", async () => {
  const open = vi.fn();
  render(
    <TeamStrip
      agents={[
        agent({}),
        agent({
          id: "agent-researcher",
          kind: "specialist",
          name: "researcher",
          status: "running",
          home_conversation_id: thread,
          last_reply: { seq: 2, status: "completed", text: "Found 3 urgent issues.", created_at: at },
          pending_requests: [
            {
              request_id: "r1",
              agent: "researcher",
              agent_id: "agent-researcher",
              conversation_id: thread,
              kind: "service",
              service_slug: "slack",
              summary: "Use Slack",
              requested_by: null,
              expires_at: at,
            },
          ],
        }),
        agent({ id: "agent-old", kind: "specialist", name: "old", status: "destroyed", destroyed_at: at }),
      ]}
      onOpenConversation={open}
    />,
  );
  const strip = screen.getByRole("region", { name: "Team" });
  const toggle = within(strip).getByRole("button", { expanded: true });
  expect(toggle).toHaveTextContent("1 specialist · 1 working");
  expect(strip).toHaveTextContent("1 request");
  expect(strip).toHaveTextContent("Found 3 urgent issues.");
  expect(strip).not.toHaveTextContent("old");
  const user = userEvent.setup();
  await user.click(
    within(strip).getByRole("button", { name: "Review researcher's request: Use Slack" }),
  );
  expect(open).toHaveBeenCalledWith(thread);
  await user.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(strip).not.toHaveTextContent("Found 3 urgent issues.");
});

it("renders no team strip without live specialists", () => {
  const view = render(<TeamStrip agents={[agent({})]} onOpenConversation={vi.fn()} />);
  expect(view.container).toBeEmptyDOMElement();
});

it("notes queued updates only when there are some", () => {
  const view = render(<PendingEventsNote count={0} agentName="NyxBot" />);
  expect(view.container).toBeEmptyDOMElement();
  view.rerender(<PendingEventsNote count={2} agentName="NyxBot" />);
  expect(screen.getByRole("status")).toHaveTextContent("2 updates waiting for NyxBot's next turn");
});

it("shows what the thread is waiting for and that the agent continues by itself", () => {
  const view = render(<WaitingNote items={[]} agentName="NyxBot" />);
  expect(view.container).toBeEmptyDOMElement();
  const soon = new Date(Date.now() + 30 * 60_000 + 5_000).toISOString();
  view.rerender(
    <WaitingNote
      agentName="NyxBot"
      items={[
        {
          kind: "channel_bot",
          title: "Waiting for your Telegram bot to be created",
          detail: null,
          since: at,
          expires_at: soon,
        },
        {
          kind: "owner_verification",
          title: "Waiting for you to verify your Telegram account with @helper_bot",
          detail: "NyxID has not received any message from this bot yet.",
          since: at,
          expires_at: null,
        },
      ]}
    />,
  );
  const note = screen.getByRole("status", { name: "Waiting" });
  expect(note).toHaveTextContent("Waiting for your Telegram bot to be created");
  expect(note).toHaveTextContent("expires in 30m");
  expect(note).toHaveTextContent("verify your Telegram account with @helper_bot");
  expect(note).toHaveTextContent("NyxID has not received any message from this bot yet.");
  expect(note).toHaveTextContent("NyxBot continues here by itself when this happens");
});
