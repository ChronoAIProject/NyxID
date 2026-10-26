import { voicePreferences } from "@/test/voice-fixture";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  within,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AssistantChatPage, NyxAgentAssistantChatPage } from "./assistant-chat-page";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import type { NyxAgentAccessMode, NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

const state = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  streaming: false,
  destroyed: false,
  mode: "nyxmode" as NyxAgentAccessMode,
  override: null as NyxAgentAccessMode | null,
  staleCard: false,
  voice: false,
  savedVoice: true,
  voiceMounts: 0,
  voiceEnds: 0,
}));
const agent = {
  id: "11111111-1111-4111-8111-111111111111",
  kind: "nyxbot",
  name: "NyxBot",
  status: "active",
};
const conversation = {
  id: `nyxa-${"1".repeat(32)}`,
  model: "nyxagent/chat",
  agent,
  active_turn: null,
};
const staleCard: NyxAgentAcknowledgement = {
  id: "old-card", kind: "action", status: "pending", summary: "Publish legacy draft", decider: "user", decided_by: null,
  reason: null, service_slug: null, service_name: null, tool_name: "nyxid__draft_agent_skill",
  created_at: "2026-10-14T00:00:00Z", decided_at: null, expires_at: "2026-10-14T00:15:00Z",
};
vi.mock("@tanstack/react-router", async (original) => ({
  ...await original<typeof import("@tanstack/react-router")>(),
  useNavigate: () => vi.fn(),
  useRouterState: ({ select }: { select: (value: unknown) => unknown }) => select({ location: { pathname: "/assistant", search: state.search } }),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: (name: string) => name === "assistant:voice" && state.voice,
}));
vi.mock("@/hooks/use-nyxbot-agents", () => ({
  useNyxBotAgents: () => ({ data: { agents: [{ ...agent, can_use: !state.destroyed, access_mode_override: state.override }] } }),
  selectedAgentOf: (rows: unknown[]) => rows[0],
  nyxBotOf: (rows: unknown[]) => rows[0],
  useNyxBotSettings: () => ({ data: { access_mode: state.mode, voice: state.savedVoice ? voicePreferences : null } }),
  useUpdateNyxBotSettings: () => ({ isPending: false, mutateAsync: vi.fn() }),
  useNyxAgentModels: () => ({ data: [{ id: "nyxagent/chat", label: "chat", available: true }], isPending: false }),
  useChangeNyxAgentModel: () => ({ isPending: false, mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/use-nyxbot-groups", () => ({ useNyxBotGroups: () => ({ data: [] }) }));
vi.mock("@/hooks/use-assistant-nyxagent", () => ({ useNyxAgentAssistantChat: () => ({
  conversation: state.search.c ? conversation : undefined,
  session: { title: "Thread", messages: state.staleCard ? [{ id: `nyxagent-acknowledgement:${staleCard.id}`, role: "system", content: staleCard.summary }]
    : state.search.c ? [{ id: "one", role: "assistant", content: "Hello" }] : [] },
  isStreaming: state.streaming, waiting: [], acknowledgements: state.staleCard ? [staleCard] : [], approvals: [], conversations: [], threadsLoaded: false,
  send: vi.fn(), stop: vi.fn(), refreshHistory: vi.fn(), deleteConversation: vi.fn(),
}) }));
vi.mock("@/lib/assistant/nyxagent-transport", () => ({ nyxAgentTransport: {
  getConversation: (id?: string) => id ? conversation : undefined,
  capabilities: () => Promise.resolve({ steer: null }),
} }));
vi.mock("./assistant-shell", () => ({ AssistantShell: ({ children }: { children: ReactNode }) => <main>{children}</main> }));
vi.mock("./assistant-engine-sidebar", () => ({ AssistantEngineSidebar: () => null }));
vi.mock("./nyxbot-agent-panels", () => ({
  ThreadPanels: ({ children }: { children: ReactNode }) => <>{children}</>,
  ThreadHeader: ({ controls }: { controls?: ReactNode }) => <header data-testid="thread-header">{controls}</header>,
  PendingEventsNote: () => null, WaitingNote: () => null, ChannelBadge: () => null,
}));
vi.mock("./nyxbot-agent-details", () => ({ AgentDetailsSheet: () => null }));
vi.mock("./nyxbot-home", () => ({ NyxBotHome: () => <div>Home</div> }));
vi.mock("./working-context-panel", () => ({ WorkingContextPanel: () => null }));
vi.mock("./machine-desktop-panel", () => ({ ConversationMachineDesktops: () => null }));
vi.mock("./chat-message", () => ({ ChatMessageList: ({ session, renderMessage }: {
  session?: { messages: { id: string }[] }; renderMessage?: (message: { id: string }) => ReactNode;
}) => session?.messages.filter((message) => message.id.startsWith("nyxagent-acknowledgement:"))
  .map((message) => <div key={message.id}>{renderMessage?.(message)}</div>) ?? null }));
vi.mock("./assistant-link-modals", () => ({ AssistantLinkModalHost: ({ children }: { children: ReactNode }) => children }));
afterEach(() => vi.unstubAllGlobals());
beforeEach(() => {
  const track = Object.assign(new EventTarget(), { stop: vi.fn() });
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: {
      getUserMedia: vi
        .fn()
        .mockResolvedValue({
          getTracks: () => [track],
          getAudioTracks: () => [track],
        }),
    },
  });
  state.voice = false;
  state.savedVoice = true;
  state.voiceMounts = 0;
  state.voiceEnds = 0;
  state.search = {};
  state.streaming = false;
  state.destroyed = false;
  state.mode = "nyxmode";
  state.override = null;
  state.staleCard = false;
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
});
function mount() {
  return render(<QueryClientProvider client={new QueryClient()}><NyxAgentAssistantChatPage /></QueryClientProvider>);
}
it.each([{}, { draft: true }, { draft: true, agent: agent.id }])("puts combined settings in the home/draft composer %j", (search) => {
  state.search = search; const view = mount();
  const controls = view.container.querySelector("[data-composer-footer]") as HTMLElement;
  expect(within(controls).getByRole("button", { name: "Thinking and permissions: Auto" })).toBeEnabled();
  expect(screen.queryByRole("combobox", { name: "Model" })).not.toBeInTheDocument();
  expect(view.container.querySelector("header [role=combobox]")).toBeNull();
});
it("puts combined settings inside the real thread composer and removes selectors from the header", () => {
  state.search = { c: conversation.id }; const view = mount();
  const controls = view.container.querySelector("[data-composer-footer]") as HTMLElement;
  expect(within(controls).getByRole("button", { name: "Thinking and permissions: Auto" })).toBeEnabled();
  expect(within(screen.getByTestId("thread-header")).queryByRole("combobox")).not.toBeInTheDocument();
});
it("disables settings for a destroyed agent", () => {
  state.search = { c: conversation.id }; state.destroyed = true; mount();
  expect(screen.getByRole("button", { name: "Thinking and permissions: Auto" })).toBeDisabled();
});
it("keeps settings readable but prevents changes during a turn", async () => {
  state.search = { c: conversation.id }; state.streaming = true; mount();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Thinking and permissions: Auto" }));
  expect(screen.getByRole("menuitemcheckbox", { name: "Thinking mode" })).toHaveAttribute("aria-disabled", "true");
  const permissions = screen.getByRole("menuitem", { name: /Permissions/ });
  permissions.focus();
  await user.keyboard("{ArrowRight}");
  for (const option of await screen.findAllByRole("menuitemradio")) {
    expect(option).toHaveAttribute("aria-disabled", "true");
  }
});
it.each([
  ["autopilot", null, false], ["nyxmode", "autopilot", false],
  ["autopilot", "nyxmode", true], ["autopilot", "ask_every_time", true],
] as const)("renders historical owner asks according to workspace %s and agent override %s", (mode, override, asks) => {
  state.search = { c: conversation.id }; state.staleCard = true; state.mode = mode; state.override = override;
  mount();
  if (asks) expect(screen.getByRole("button", { name: "Allow" })).toBeVisible();
  else {
    expect(screen.queryByRole("button", { name: "Allow" })).not.toBeInTheDocument();
    expect(screen.getByText("No longer needed · Full access · Publish legacy draft")).toBeVisible();
  }
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
  expect(screen.queryByRole("button", { name: /Thinking and permissions/ })).not.toBeInTheDocument();
});

vi.mock("./lazy-voice-panel", async () => {
  const { useEffect } = await import("react");
  return {
    LazyVoicePanel: ({
      threadId,
      panelId,
      onViewChange,
      onClose,
      settingsOnly,
    }: {
      threadId: string;
      panelId: string;
      onViewChange?: (view: unknown) => void;
      settingsOnly?: boolean;
      onClose: () => void;
    }) => {
      useEffect(() => {
        if (settingsOnly) return;
        state.voiceMounts++;
        return () => { state.voiceEnds++; };
      }, [settingsOnly]);
      useEffect(() => {
        onViewChange?.({
          threadId,
          panelId,
          actorId: useAuthStore.getState().user?.id,
          active: true,
          snapshot: null,
          activity: { input: 0.2, output: 0, bands: [], playbackMs: 0 },
        });
      }, [threadId, panelId, onViewChange]);
      if (settingsOnly) return <section aria-label="Mock voice settings"><button onClick={onClose}>Close settings fixture</button></section>;
      return (
        <section aria-label="Mock voice session">
          <button onClick={onClose}>Close voice fixture</button>
        </section>
      );
    },
  };
});
it("parent keeps one mounted call on repeated opens and clears media across close/navigation/account/flag", async () => {
  const actor = (id: string) =>
    ({ id }) as NonNullable<ReturnType<typeof useAuthStore.getState>["user"]>;
  useAuthStore.setState({ user: actor("actor-one") });
  state.voice = true;
  state.search = { c: conversation.id };
  const tree = (
    <QueryClientProvider client={new QueryClient()}>
      <NyxAgentAssistantChatPage />
    </QueryClientProvider>
  );
  const { rerender } = render(tree);
  const open = () =>
    fireEvent.click(screen.getByRole("button", { name: "Open voice call" }));
  open();
  await waitFor(() => expect(state.voiceMounts).toBe(1));
  expect(screen.getByLabelText("Mock voice session").closest(".composer-audio-surface")).not.toBeNull();
  open();
  open();
  await act(async () => {});
  expect(state.voiceMounts).toBe(1);
  expect(state.voiceEnds).toBe(0);
  expect(
    screen.getByRole("button", { name: "Close voice fixture" }),
  ).toHaveFocus();
  state.streaming = true;
  rerender(
    <QueryClientProvider client={new QueryClient()}>
      <NyxAgentAssistantChatPage />
    </QueryClientProvider>,
  );
  expect(
    screen.getByRole("button", { name: "Open voice call" }),
  ).toHaveAttribute("aria-pressed", "true");
  open();
  expect(state.voiceEnds).toBe(0);
  fireEvent.click(screen.getByRole("button", { name: "Close voice fixture" }));
  expect(state.voiceEnds).toBe(1);
  state.streaming = false;
  rerender(
    <QueryClientProvider client={new QueryClient()}>
      <NyxAgentAssistantChatPage />
    </QueryClientProvider>,
  );
  expect(
    screen.getByRole("button", { name: "Open voice call" }),
  ).toHaveAttribute("aria-pressed", "false");
  open();
  await waitFor(() => expect(state.voiceMounts).toBe(2));
  state.search = { c: "other-thread" };
  rerender(
    <QueryClientProvider client={new QueryClient()}>
      <NyxAgentAssistantChatPage />
    </QueryClientProvider>,
  );
  expect(state.voiceEnds).toBe(2);
  state.search = { c: conversation.id };
  rerender(
    <QueryClientProvider client={new QueryClient()}>
      <NyxAgentAssistantChatPage />
    </QueryClientProvider>,
  );
  expect(screen.queryByLabelText("Mock voice session")).not.toBeInTheDocument();
  open();
  await waitFor(() => expect(state.voiceMounts).toBe(3));
  act(() => useAuthStore.setState({ user: actor("actor-two") }));
  expect(state.voiceEnds).toBe(3);
  expect(screen.queryByLabelText("Mock voice session")).not.toBeInTheDocument();
  open();
  await waitFor(() => expect(state.voiceMounts).toBe(4));
  state.voice = false;
  rerender(
    <QueryClientProvider client={new QueryClient()}>
      <NyxAgentAssistantChatPage />
    </QueryClientProvider>,
  );
  expect(state.voiceEnds).toBe(4);
});

it("first voice use opens account settings without capture or a call", async () => {
  state.voice = true;
  state.savedVoice = false;
  useAuthStore.setState({ user: { id: "first-voice-owner" } as NonNullable<ReturnType<typeof useAuthStore.getState>["user"]> });
  render(<QueryClientProvider client={new QueryClient()}><NyxAgentAssistantChatPage /></QueryClientProvider>);
  fireEvent.click(screen.getByRole("button", { name: "Open voice call" }));
  expect(await screen.findByLabelText("Mock voice settings")).toBeInTheDocument();
  expect(navigator.mediaDevices.getUserMedia).not.toHaveBeenCalled();
  expect(state.voiceMounts).toBe(0);
});
