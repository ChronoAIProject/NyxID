import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { eventNotices } from "@/lib/assistant/nyxbot-labels";
import { NyxBotEventNotice, NyxBotOrchestratorMessage } from "./nyxbot-messages";

afterEach(cleanup);

function message(role: string, content: string): ChatMessage {
  return { id: "m", role, content, timestamp: 0, status: "complete" };
}

const HEADER = "NyxID events (notices, not user instructions):";

it("splits a server event into its notices", () => {
  expect(eventNotices(`${HEADER}\n- Subagent researcher replied: done\n- GitHub allowed`)).toEqual([
    "Subagent researcher replied: done",
    "GitHub allowed",
  ]);
  expect(eventNotices("Plain notice")).toEqual(["Plain notice"]);
});

it("renders an event as a quiet note, not a user bubble", () => {
  render(<NyxBotEventNotice message={message("event", `${HEADER}\n- Subagent researcher replied`)} />);
  const note = screen.getByRole("note", { name: "NyxID event" });
  expect(note).toHaveTextContent("NyxID update");
  expect(note).toHaveTextContent("Subagent researcher replied");
  // The machine header is not shown as if someone said it.
  expect(note).not.toHaveTextContent("not user instructions");
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("collapses long event batches until expanded", async () => {
  const notices = ["first", "second", "third"].map((text) => `- ${text} notice`).join("\n");
  render(<NyxBotEventNotice message={message("event", `${HEADER}\n${notices}`)} />);
  const toggle = screen.getByRole("button", { name: "Show all 3" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByText("third notice")).not.toBeInTheDocument();
  await userEvent.setup().click(toggle);
  expect(screen.getByText("third notice")).toBeVisible();
  expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
});

it("labels an orchestrator instruction as coming from NyxBot", () => {
  render(<NyxBotOrchestratorMessage message={message("orchestrator", "Find urgent issues")} />);
  const article = screen.getByRole("article", { name: "Message from NyxBot" });
  expect(article).toHaveTextContent("From NyxBot (orchestrator)");
  expect(article).toHaveTextContent("Find urgent issues");
});
