import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState, type ComponentProps } from "react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { AwayList, NyxBotHome } from "./nyxbot-home";

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

type HomeProps = ComponentProps<typeof NyxBotHome>;
const suggestion = { id: "research", title: "Research a topic", description: "Choose something to learn", prompt: "Research", icon: "search" };

function home(props: Partial<HomeProps> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const base: HomeProps = {
    userName: "Reader", agents: [], agentsLoading: false, groups: [], bottomInset: 0,
    onChat: vi.fn(), onOpenConversation: vi.fn(), onOpenGroup: vi.fn(), onNewGroup: vi.fn(),
    onNewAgent: vi.fn(), onManageMemory: vi.fn(), nextSteps: [suggestion], onStartNextStep: vi.fn(),
    onHideNextSteps: vi.fn(), onShowNextSteps: vi.fn(), ...props,
  };
  function Harness() {
    const [hidden, setHidden] = useState(base.suggestionsHidden ?? false);
    return <NyxBotHome {...base} suggestionsHidden={hidden}
      onHideNextSteps={() => { base.onHideNextSteps?.(); setHidden(true); }}
      onShowNextSteps={() => { base.onShowNextSteps?.(); setHidden(false); }} />;
  }
  const wrapper = ({ children }: { children: React.ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  return { base, wrapper, ...render(<Harness />, { wrapper }) };
}

function pendingHome(initialHidden: boolean) {
  const { base, rerender } = home();
  function PendingHome({ settled = false, failed = false, serverHidden }: { settled?: boolean; failed?: boolean; serverHidden?: boolean }) {
    const [requestedHidden, setRequestedHidden] = useState<boolean>();
    return <NyxBotHome {...base}
      suggestionsHidden={serverHidden ?? (failed ? initialHidden : requestedHidden ?? initialHidden)}
      suggestionsPending={requestedHidden !== undefined && !settled}
      suggestionsError={failed ? initialHidden ? "show" : "hide" : undefined}
      onHideNextSteps={() => { base.onHideNextSteps?.(); setRequestedHidden(true); }}
      onShowNextSteps={() => { base.onShowNextSteps?.(); setRequestedHidden(false); }} />;
  }
  rerender(<PendingHome />);
  return {
    base,
    settle: (failed = false) => rerender(<PendingHome settled failed={failed} />),
    refetch: (hidden: boolean, failed = true) => rerender(<PendingHome settled failed={failed} serverHidden={hidden} />),
  };
}

async function repeatPendingActivation(button: HTMLElement) {
  const user = userEvent.setup();
  button.focus();
  expect(button).toHaveAttribute("aria-disabled", "true");
  expect(button).not.toBeDisabled();
  for (let repeat = 0; repeat < 2; repeat++) {
    await user.click(button);
    expect(button).toHaveFocus();
    expect(document.activeElement).not.toBe(document.body);
    await user.keyboard("{Enter}");
    expect(button).toHaveFocus();
    expect(document.activeElement).not.toBe(document.body);
    await user.keyboard(" ");
    expect(button).toHaveFocus();
    expect(document.activeElement).not.toBe(document.body);
  }
}

it("offers Show suggestions when Home loads with hidden recommendations", async () => {
  const { base } = home({ suggestionsHidden: true });
  expect(screen.getByText("Suggestions are hidden.")).toBeVisible();
  expect(screen.queryByText("Research a topic")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Show suggestions" }));
  expect(base.onShowNextSteps).toHaveBeenCalledOnce();
  expect(screen.getByText("Research a topic")).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent("Suggestions shown.");
});

it.each([false, true])("does not announce initial suggestion visibility (hidden: %s)", (hidden) => {
  home({ suggestionsHidden: hidden });
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
});

it.each([false, true])("transfers focused visibility controls without scrolling on a background flip (initially hidden: %s)", (hidden) => {
  const { base, rerender } = home();
  rerender(<NyxBotHome {...base} suggestionsHidden={hidden} />);
  screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" }).focus();
  const focus = vi.spyOn(HTMLElement.prototype, "focus");
  try {
    rerender(<NyxBotHome {...base} suggestionsHidden={!hidden} />);
    expect(screen.getByRole("button", { name: hidden ? "Hide" : "Show suggestions" })).toHaveFocus();
    expect(document.activeElement).not.toBe(document.body);
    expect(focus).toHaveBeenCalledExactlyOnceWith({ preventScroll: true });
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
    expect(base.onHideNextSteps).not.toHaveBeenCalled();
    expect(base.onShowNextSteps).not.toHaveBeenCalled();
  } finally {
    focus.mockRestore();
  }
});

it("leaves focus elsewhere and forgets a visibility control after focus moves away", () => {
  const { base, rerender } = home();
  rerender(<NyxBotHome {...base} suggestionsHidden />);
  screen.getByRole("button", { name: "Show suggestions" }).focus();
  const elsewhere = screen.getByRole("button", { name: "New agent" });
  elsewhere.focus();
  rerender(<NyxBotHome {...base} suggestionsHidden={false} />);
  expect(elsewhere).toHaveFocus();
  elsewhere.blur();
  expect(document.activeElement).toBe(document.body);
  rerender(<NyxBotHome {...base} suggestionsHidden />);
  expect(document.activeElement).toBe(document.body);
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
});

it.each([false, true])("clears stale success announcements without announcing background flips (initially hidden: %s)", async (hidden) => {
  const { base, rerender } = home();
  rerender(<NyxBotHome {...base} suggestionsHidden={hidden} />);
  await userEvent.click(screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" }));
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  rerender(<NyxBotHome {...base} suggestionsHidden={!hidden} />);
  const announcement = hidden ? "Suggestions shown." : "Suggestions hidden.";
  expect(screen.getByRole("status")).toHaveTextContent(announcement);
  expect(screen.getByRole("button", { name: hidden ? "Hide" : "Undo" })).toHaveFocus();
  rerender(<NyxBotHome {...base} suggestionsHidden={hidden} />);
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" })).toHaveFocus();
  expect(document.activeElement).not.toBe(document.body);
  rerender(<NyxBotHome {...base} suggestionsHidden={!hidden} />);
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(hidden ? base.onShowNextSteps : base.onHideNextSteps).toHaveBeenCalledOnce();
  expect(hidden ? base.onHideNextSteps : base.onShowNextSteps).not.toHaveBeenCalled();
});

it("replaces Hide with a focused Undo, announces the change and restores with the keyboard", async () => {
  const user = userEvent.setup();
  const { base } = home();
  const hide = screen.getByRole("button", { name: "Hide" });
  hide.focus();
  await user.keyboard("{Enter}");
  const undo = screen.getByRole("button", { name: "Undo" });
  await waitFor(() => expect(undo).toHaveFocus());
  expect(screen.getByRole("status")).toHaveTextContent("Suggestions hidden.");
  expect(screen.getByRole("status")).toHaveAttribute("aria-live", "polite");
  await user.keyboard("{Enter}");
  expect(base.onShowNextSteps).toHaveBeenCalledOnce();
  expect(screen.getByText("Research a topic")).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent("Suggestions shown.");
  await waitFor(() => expect(screen.getByRole("button", { name: "Hide" })).toHaveFocus());
});

it.each([
  { name: "Hide", hidden: false },
  { name: "Show suggestions", hidden: true },
  { name: "Retry", hidden: false, error: "hide" as const },
  { name: "Retry", hidden: true, error: "show" as const },
])("ignores pending click, Enter and Space on $name (error: $error)", async ({ name, hidden, error }) => {
  const { base } = home({ suggestionsHidden: hidden, suggestionsPending: true, suggestionsError: error });
  await repeatPendingActivation(screen.getByRole("button", { name }));
  expect(base.onHideNextSteps).not.toHaveBeenCalled();
  expect(base.onShowNextSteps).not.toHaveBeenCalled();
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" })).toBeVisible();
});

it.each([false, true])("keeps the counterpart focused throughout a pending request (initially hidden: %s)", async (hidden) => {
  const user = userEvent.setup();
  const { base, settle } = pendingHome(hidden);
  screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" }).focus();
  await user.keyboard("{Enter}");
  const counterpart = screen.getByRole("button", { name: hidden ? "Hide" : "Undo" });
  expect(counterpart).toHaveFocus();
  expect(document.activeElement).not.toBe(document.body);
  await repeatPendingActivation(counterpart);
  expect(hidden ? base.onShowNextSteps : base.onHideNextSteps).toHaveBeenCalledOnce();
  expect(hidden ? base.onHideNextSteps : base.onShowNextSteps).not.toHaveBeenCalled();
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  settle();
  expect(counterpart).toHaveFocus();
  expect(document.activeElement).not.toBe(document.body);
  expect(counterpart).toHaveAttribute("aria-disabled", "false");
  expect(screen.getByRole("status")).toHaveTextContent(hidden ? "Suggestions shown." : "Suggestions hidden.");
});

it.each([false, true])("returns focus to the original control after rollback (initially hidden: %s)", async (hidden) => {
  const user = userEvent.setup();
  const { base, settle } = pendingHome(hidden);
  screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" }).focus();
  await user.keyboard("{Enter}");
  const counterpart = screen.getByRole("button", { name: hidden ? "Hide" : "Undo" });
  expect(counterpart).toHaveFocus();
  expect(document.activeElement).not.toBe(document.body);
  await repeatPendingActivation(counterpart);
  settle(true);
  const original = screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" });
  expect(original).toHaveFocus();
  expect(document.activeElement).not.toBe(document.body);
  expect(original).toHaveAttribute("aria-disabled", "false");
  expect(screen.getByRole("button", { name: "Retry" })).toBeVisible();
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(hidden ? base.onShowNextSteps : base.onHideNextSteps).toHaveBeenCalledOnce();
  expect(hidden ? base.onHideNextSteps : base.onShowNextSteps).not.toHaveBeenCalled();
});

it.each([false, true])("keeps focus through rollback followed by a server refetch flip (initially hidden: %s)", async (hidden) => {
  const { base, settle, refetch } = pendingHome(hidden);
  await userEvent.click(screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" }));
  settle(true);
  expect(screen.getByRole("button", { name: hidden ? "Show suggestions" : "Hide" })).toHaveFocus();
  refetch(!hidden);
  expect(screen.getByRole("button", { name: hidden ? "Hide" : "Undo" })).toHaveFocus();
  expect(document.activeElement).not.toBe(document.body);
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  refetch(!hidden, false);
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  expect(hidden ? base.onShowNextSteps : base.onHideNextSteps).toHaveBeenCalledOnce();
  expect(hidden ? base.onHideNextSteps : base.onShowNextSteps).not.toHaveBeenCalled();
});

it("announces a restore only after the request succeeds", async () => {
  const { base, rerender } = home();
  function PendingRestore({ pending }: { pending: boolean }) {
    const [hidden, setHidden] = useState(true);
    return <NyxBotHome {...base} suggestionsHidden={hidden} suggestionsPending={!hidden && pending}
      onShowNextSteps={() => setHidden(false)} />;
  }
  rerender(<PendingRestore pending />);
  await userEvent.click(screen.getByRole("button", { name: "Show suggestions" }));
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
  rerender(<PendingRestore pending={false} />);
  await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Suggestions shown."));
});

it.each(["hide", "show"] as const)("shows a localized retry action after %s fails", async (operation) => {
  const { base } = home({ suggestionsHidden: operation === "show", suggestionsError: operation });
  expect(screen.getByRole("alert")).toHaveTextContent(`Couldn't ${operation} suggestions. Try again.`);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(operation === "hide" ? base.onHideNextSteps : base.onShowNextSteps).toHaveBeenCalledOnce();
  expect(screen.getByRole("status")).toBeEmptyDOMElement();
});

it("has no restore row without recommendations", () => {
  home({ suggestionsHidden: true, nextSteps: [] });
  expect(screen.queryByRole("button", { name: "Show suggestions" })).not.toBeInTheDocument();
  expect(screen.queryByText("Suggestions are hidden.")).not.toBeInTheDocument();
});
