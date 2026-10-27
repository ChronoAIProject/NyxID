import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { PendingEventsNote, ThreadHeader, WaitingNote } from "./nyxbot-agent-panels";

afterEach(cleanup);

const at = "2026-09-28T00:00:00Z";
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
