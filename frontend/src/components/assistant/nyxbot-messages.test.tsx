import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it } from "vitest";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { NyxBotEventNotice, NyxBotOrchestratorMessage } from "./nyxbot-messages";

afterEach(cleanup);

function message(role: string, content: string): ChatMessage {
  return { id: "m", role, content, timestamp: 0, status: "complete" };
}

/** The header the server puts on every event turn (services/assistant_nyxagent.rs). */
const HEADER =
  "NyxID events (authored by NyxID; only a quoted owner message is a request from the user):";
const REPLY =
  'Specialist researcher replied. Reply excerpt: "Found 3 urgent issues." Read more with nyxid__read_subagent.';

it("renders each notice as a compact activity card, not a user bubble", () => {
  render(
    <NyxBotEventNotice
      message={message(
        "event",
        `${HEADER}\n- ${REPLY}\n- The user finished connecting github (connect_link_id cl-1). Continue the task that needed it now; do not ask them to confirm.`,
      )}
    />,
  );
  const group = screen.getByRole("group", { name: "NyxID updates" });
  const cards = within(group).getAllByRole("note");
  expect(cards.map((card) => card.getAttribute("aria-label"))).toEqual([
    "NyxID event: researcher replied",
    "NyxID event: Finished connecting github",
  ]);
  // The machine header and the model-facing instructions are not shown.
  expect(group).not.toHaveTextContent("authored by NyxID");
  expect(group).not.toHaveTextContent("nyxid__read_subagent");
  // A notice without more text has nothing to expand.
  expect(within(cards[1]!).queryByRole("button")).not.toBeInTheDocument();
});

it("keeps a card's text collapsed until expanded", async () => {
  render(<NyxBotEventNotice message={message("event", `${HEADER}\n- ${REPLY}`)} />);
  const card = screen.getByRole("note", { name: "NyxID event: researcher replied" });
  const toggle = within(card).getByRole("button", { name: "researcher replied" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(card).not.toHaveTextContent("Found 3 urgent issues.");
  await userEvent.setup().click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  expect(card).toHaveTextContent("Found 3 urgent issues.");
});

it("still shows unknown notices by their first sentence", () => {
  render(<NyxBotEventNotice message={message("event", "Something new happened. More detail here.")} />);
  expect(
    screen.getByRole("note", { name: "NyxID event: Something new happened" }),
  ).toBeInTheDocument();
});

it("labels an orchestrator instruction as coming from NyxBot", () => {
  render(<NyxBotOrchestratorMessage message={message("orchestrator", "Find urgent issues")} />);
  const article = screen.getByRole("article", { name: "Message from NyxBot" });
  expect(article).toHaveTextContent("From NyxBot");
  expect(article).not.toHaveTextContent("orchestrator");
  expect(article).toHaveTextContent("Find urgent issues");
});
