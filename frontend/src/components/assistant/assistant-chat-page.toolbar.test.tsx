import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AssistantChatPage, NyxAgentAssistantChatPage } from "./assistant-chat-page";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

const state = vi.hoisted(() => ({ search: {} as Record<string, unknown>, streaming: false, destroyed: false }));
const agent = { id: "11111111-1111-4111-8111-111111111111", kind: "nyxbot", name: "NyxBot", status: "active" };
const conversation = { id: `nyxa-${"1".repeat(32)}`, model: "nyxagent/chat", agent, active_turn: null };
vi.mock("@tanstack/react-router", async (original) => ({
  ...await original<typeof import("@tanstack/react-router")>(),
  useNavigate: () => vi.fn(),
  useRouterState: ({ select }: { select: (value: unknown) => unknown }) => select({ location: { pathname: "/assistant", search: state.search } }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => false }));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [{ ...agent, can_use: !state.destroyed }] } }),
  selectedAgentOf: (rows: unknown[]) => rows[0],
  nyxBotOf: (rows: unknown[]) => rows[0],
  useNyxBotSettings: () => ({ data: { access_mode: "nyxmode" } }),
  useUpdateNyxBotSettings: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useNyxAgentModels: () => ({ data: [{ id: "nyxagent/chat", label: "chat", available: true }], isPending: false }),
  useChangeNyxAgentModel: () => ({ isPending: false, mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-nyxbot-groups", () => ({ useNyxBotGroups: () => ({ data: [] }) }));
vi.mock("@/hooks/use-assistant-nyxagent", () => ({ useNyxAgentAssistantChat: () => ({
  conversation: state.search.c ? conversation : undefined,
  session: { title: "Thread", messages: state.search.c ? [{ id: "one", role: "assistant", content: "Hello" }] : [] },
  isStreaming: state.streaming, waiting: [], acknowledgements: [], approvals: [], conversations: [], threadsLoaded: false,
  send: vi.fn(), stop: vi.fn(), refreshHistory: vi.fn(), deleteConversation: vi.fn(),
}) }));
vi.mock("@/lib/assistant/nyxagent-transport", () => ({ nyxAgentTransport: {
  getConversation: (id?: string) => id ? conversation : undefined,
  capabilities: () => Promise.resolve({ steer: null }),
} }));
vi.mock("./assistant-shell", () => ({ AssistantShell: ({ children }: { children: ReactNode }) => <main>{children}</main> }));
vi.mock("./assistant-engine-sidebar", () => ({ AssistantEngineSidebar: () => null }));
vi.mock("./nyxbot-agent-panels", () => ({
  ThreadHeader: ({ controls }: { controls?: ReactNode }) => <header data-testid="thread-header">{controls}</header>,
  TeamStrip: () => null, PendingEventsNote: () => null, WaitingNote: () => null, ChannelBadge: () => null,
}));
vi.mock("./nyxbot-agent-details", () => ({ AgentDetailsSheet: () => null }));
vi.mock("./nyxbot-home", () => ({ NyxBotHome: () => <div>Home</div> }));
vi.mock("./working-context-panel", () => ({ WorkingContextPanel: () => null }));
vi.mock("./machine-desktop-panel", () => ({ ConversationMachineDesktops: () => null }));
vi.mock("./chat-message", () => ({ ChatMessageList: () => null }));
vi.mock("./assistant-link-modals", () => ({ AssistantLinkModalHost: ({ children }: { children: ReactNode }) => children }));
beforeEach(() => {
  state.search = {}; state.streaming = false; state.destroyed = false;
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
});
function mount() {
  return render(<QueryClientProvider client={new QueryClient()}><NyxAgentAssistantChatPage /></QueryClientProvider>);
}
it.each([{}, { draft: true }, { draft: true, agent: agent.id }])("puts the permission selector in the home/draft composer %j without a model selector", (search) => {
  state.search = search; const view = mount();
  const toolbar = view.container.querySelector("[data-composer-toolbar]") as HTMLElement;
  expect(within(toolbar).getByRole("combobox", { name: "Permission mode" })).toHaveTextContent("Auto");
  expect(screen.queryByRole("combobox", { name: "Model" })).not.toBeInTheDocument();
  expect(view.container.querySelector("header [role=combobox]")).toBeNull();
});
it("puts both selectors inside the real thread composer and removes them from the header", () => {
  state.search = { c: conversation.id }; const view = mount();
  const toolbar = view.container.querySelector("[data-composer-toolbar]") as HTMLElement;
  expect(within(toolbar).getByRole("combobox", { name: "Permission mode" })).toBeEnabled();
  expect(within(toolbar).getByRole("combobox", { name: "Model" })).toHaveTextContent("Model:chat");
  expect(within(screen.getByTestId("thread-header")).queryByRole("combobox")).not.toBeInTheDocument();
});
it.each(["streaming", "destroyed"] as const)("disables both selectors while %s", (condition) => {
  state.search = { c: conversation.id }; state[condition] = true; mount();
  expect(screen.getByRole("combobox", { name: "Permission mode" })).toBeDisabled();
  expect(screen.getByRole("combobox", { name: "Model" })).toBeDisabled();
});

vi.mock("./chat-actor-controls", () => ({ ChatActorControls: () => null }));
vi.mock("@/hooks/use-assistant-chat", () => ({ useAssistantChat: () => ({
  visibleConversations: [], session: { title: "New chat", messages: [] }, detailState: { status: "ready" },
  isStreaming: false, stop: vi.fn(),
}) }));
it("keeps the Aevatar composer without permission or model toolbar", () => {
  const view = render(<QueryClientProvider client={new QueryClient()}><AssistantChatPage /></QueryClientProvider>);
  expect(screen.getByRole("textbox")).toBeInTheDocument();
  expect(view.container.querySelector("[data-composer-toolbar]")).toBeNull();
  expect(screen.queryByRole("combobox", { name: "Permission mode" })).not.toBeInTheDocument();
});
