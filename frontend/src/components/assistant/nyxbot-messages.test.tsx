import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
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

it("keeps a multi-line reply excerpt in one card, not a card per line", async () => {
  const user = userEvent.setup();
  render(
    <NyxBotEventNotice
      message={message(
        "event",
        `${HEADER}\n- Specialist reservation-test replied. Reply excerpt: "Configured FAQ answers:\n• Address: Junction 8\n- Hours: 11:00-21:30" Read more with nyxid__read_subagent.\n- The user finished connecting github (connect_link_id cl-1).`,
      )}
    />,
  );
  const cards = within(screen.getByRole("group", { name: "NyxID updates" })).getAllByRole("note");
  expect(cards.map((card) => card.getAttribute("aria-label"))).toEqual([
    "NyxID event: reservation-test replied",
    "NyxID event: Finished connecting github",
  ]);
  await user.click(within(cards[0]!).getByRole("button"));
  expect(cards[0]).toHaveTextContent("• Address: Junction 8");
  expect(cards[0]).toHaveTextContent("- Hours: 11:00-21:30");
});

it("shows an automation run by its automation's name, the instruction on demand", async () => {
  const user = userEvent.setup();
  const root = createRootRoute();
  const route = createRoute({
    getParentRoute: () => root,
    path: "/",
    component: () => (
      <NyxBotEventNotice
        message={{
          ...message("event", "Maintain only Notion FAQ KEK specs.\nEvery day recheck each outlet."),
          origin: "trigger",
          automation: { triggerId: "t1", runId: "r1", label: "Keep KEK restaurant FAQ current" },
        }}
      />
    ),
  });
  const automations = createRoute({ getParentRoute: () => root, path: "/assistant/automations" });
  const router = createRouter({
    routeTree: root.addChildren([route, automations]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  await router.load();
  render(<RouterProvider router={router} />);
  const card = await screen.findByRole("note", { name: "NyxID event: Automation · Keep KEK restaurant FAQ current" });
  expect(card).not.toHaveTextContent("Maintain only");
  await user.click(within(card).getByRole("button"));
  expect(card).toHaveTextContent("Every day recheck each outlet.");
  expect(within(card).getByRole("link", { name: "Open Automations" })).toBeInTheDocument();
});

it("names an automation run whose record has expired generically", () => {
  render(<NyxBotEventNotice message={{ ...message("event", "Do the thing."), origin: "trigger" }} />);
  expect(screen.getByRole("note", { name: "NyxID event: Automation run" })).toBeInTheDocument();
});

it("labels a finished background task", () => {
  render(
    <NyxBotEventNotice
      message={message("event", `${HEADER}\n- Asynchronous service operation finished: done. Its result is quoted in this turn's context.`)}
    />,
  );
  expect(screen.getByRole("note", { name: "NyxID event: Background task finished" })).toBeInTheDocument();
});

it("labels an orchestrator instruction as coming from NyxBot", () => {
  render(<NyxBotOrchestratorMessage message={message("orchestrator", "Find urgent issues")} />);
  const article = screen.getByRole("article", { name: "Message from NyxBot" });
  expect(article).toHaveTextContent("From NyxBot");
  expect(article).not.toHaveTextContent("orchestrator");
  expect(article).toHaveTextContent("Find urgent issues");
});

it("stamps an orchestrator instruction with its send time", () => {
  const sent = "2026-10-12T07:40:43.000Z";
  render(
    <NyxBotOrchestratorMessage
      message={{ ...message("orchestrator", "Find urgent issues"), timestamp: Date.parse(sent) }}
    />,
  );
  const article = screen.getByRole("article", { name: "Message from NyxBot" });
  expect(article.querySelector(`time[datetime="${sent}"]`)).toBeInTheDocument();
});
