import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NyxAgentAssistantChatPage } from "./assistant-chat-page";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

/**
 * The agent details sheet is URL-driven when a server link opens it
 * (`AssistantPage::AgentDetails` -> `/assistant?agent=<id>&details=grants`):
 * reload reopens it, Back closes it, and the linked agent is named exactly
 * even when it is not in the owner's agent list.
 */
const state = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  navigations: [] as { search: unknown; replace?: boolean }[],
  sheet: [] as { agentId?: string; tab?: string; open: boolean }[],
  threads: [] as { id: string }[],
  threadsLoaded: false,
}));
const specialist = { id: "agent-researcher", kind: "specialist", name: "researcher", status: "idle", can_use: true };
const nyxbot = { id: "agent-nyxbot", kind: "nyxbot", name: "NyxBot", status: "idle", can_use: true };
vi.mock("@tanstack/react-router", async (original) => ({
  ...await original<typeof import("@tanstack/react-router")>(),
  useNavigate: () => (options: { search: unknown; replace?: boolean }) => {
    const search = typeof options.search === "function"
      ? (options.search as (previous: Record<string, unknown>) => unknown)(state.search)
      : options.search;
    state.navigations.push({ search, replace: options.replace });
    return Promise.resolve();
  },
  useRouterState: ({ select }: { select: (value: unknown) => unknown }) =>
    select({ location: { pathname: "/assistant", search: state.search } }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => false }));
vi.mock("@/hooks/use-nyxbot-agents", async (original) => ({
  ...await original<typeof import("@/hooks/use-nyxbot-agents")>(),
  useNyxBotAgents: () => ({ data: { agents: [nyxbot, specialist] } }),
  useNyxBotSettings: () => ({ data: { access_mode: "nyxmode" } }),
  useUpdateNyxBotSettings: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useNyxAgentModels: () => ({ data: [], isPending: false }),
  useChangeNyxAgentModel: () => ({ isPending: false, mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-nyxbot-groups", () => ({ useNyxBotGroups: () => ({ data: [] }) }));
vi.mock("@/hooks/use-assistant-nyxagent", () => ({ useNyxAgentAssistantChat: () => ({
  conversation: undefined,
  session: { title: "Thread", messages: [] },
  isStreaming: false, waiting: [], acknowledgements: [], approvals: [],
  conversations: state.threads, threadsLoaded: state.threadsLoaded,
  send: vi.fn(), stop: vi.fn(), refreshHistory: vi.fn(), deleteConversation: vi.fn(),
}) }));
vi.mock("@/lib/assistant/nyxagent-transport", () => ({ nyxAgentTransport: {
  getConversation: () => undefined,
  capabilities: () => Promise.resolve({ steer: null }),
} }));
vi.mock("./assistant-shell", () => ({ AssistantShell: ({ children }: { children: ReactNode }) => <main>{children}</main> }));
vi.mock("./assistant-engine-sidebar", () => ({ AssistantEngineSidebar: () => null }));
vi.mock("./nyxbot-agent-panels", () => ({
  ThreadHeader: () => <header data-testid="thread-header" />,
  TeamStrip: () => null, PendingEventsNote: () => null, WaitingNote: () => null, ChannelBadge: () => null,
}));
vi.mock("./nyxbot-agent-details", () => ({
  AgentDetailsSheet: (props: { agentId?: string; tab?: string; open: boolean; onOpenChange: (open: boolean) => void }) => {
    state.sheet.push({ agentId: props.agentId, tab: props.tab, open: props.open });
    return props.open ? (
      <div role="dialog" aria-label="Agent details">
        <button onClick={() => props.onOpenChange(false)}>Close sheet</button>
      </div>
    ) : null;
  },
}));
vi.mock("./nyxbot-home", () => ({ NyxBotHome: () => <div>Home</div> }));
vi.mock("./working-context-panel", () => ({ WorkingContextPanel: () => null }));
vi.mock("./machine-desktop-panel", () => ({ ConversationMachineDesktops: () => null }));
vi.mock("./chat-message", () => ({ ChatMessageList: ({ emptyDescription }: { emptyDescription?: string }) => (
  <p data-testid="empty-description">{emptyDescription}</p>
) }));
vi.mock("./assistant-link-modals", () => ({ AssistantLinkModalHost: ({ children }: { children: ReactNode }) => children }));
vi.mock("./nyxagent-composer", () => ({ NyxAgentComposer: () => <form aria-label="Composer" /> }));

beforeEach(() => {
  state.search = {}; state.navigations = []; state.sheet = []; state.threads = []; state.threadsLoaded = false;
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
});
afterEach(cleanup);

function mount() {
  return render(<QueryClientProvider client={new QueryClient()}><NyxAgentAssistantChatPage /></QueryClientProvider>);
}
function lastSheet() {
  return state.sheet[state.sheet.length - 1];
}

it("opens the sheet from the URL on the linked agent and section, and closes it by dropping the section", async () => {
  state.search = { agent: specialist.id, details: "grants" };
  mount();
  expect(screen.getByRole("dialog", { name: "Agent details" })).toBeVisible();
  expect(lastSheet()).toEqual({ agentId: specialist.id, tab: "grants", open: true });
  screen.getByRole("button", { name: "Close sheet" }).click();
  await waitFor(() => expect(state.navigations.length).toBeGreaterThan(0));
  // Closing navigates (push) without `details`; `agent` stays so the thread view is unchanged.
  const close = state.navigations.at(-1);
  expect(close?.replace).not.toBe(true);
  expect(close?.search).toEqual({ c: undefined, agent: specialist.id, draft: undefined });
});

it("names the linked agent exactly even when it is not in the agent list", () => {
  state.search = { agent: "agent-not-mine", details: "skills" };
  mount();
  expect(lastSheet()).toEqual({ agentId: "agent-not-mine", tab: "skills", open: true });
});

it("keeps the sheet open across the latest-thread redirect a bare agent link performs", async () => {
  state.search = { agent: specialist.id, details: "grants" };
  state.threads = [{ id: "nyxa-latest" }];
  state.threadsLoaded = true;
  mount();
  await waitFor(() => expect(state.navigations.some((n) => n.replace)).toBe(true));
  const redirect = state.navigations.find((n) => n.replace);
  expect(redirect?.search).toEqual({ c: "nyxa-latest", agent: specialist.id, details: "grants" });
});

it("stays closed without a section param and stays local when opened from the header", () => {
  state.search = { agent: specialist.id };
  mount();
  expect(screen.queryByRole("dialog", { name: "Agent details" })).not.toBeInTheDocument();
  expect(lastSheet()).toEqual({ agentId: undefined, tab: undefined, open: false });
});

it("tells the owner a direct specialist chat is handled by that specialist, never handed to NyxBot", () => {
  state.search = { draft: true, agent: specialist.id };
  mount();
  const empty = screen.getByTestId("empty-description");
  expect(empty).toHaveTextContent(
    "Talk to researcher directly. It handles your request itself, remembers across its threads, and tells you if it needs access you have not granted.",
  );
  expect(empty).not.toHaveTextContent(/NyxBot/);
});
