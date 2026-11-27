import {
  configure,
  render,
  screen,
  within,
} from "@testing-library/react-native";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement, ReactNode } from "react";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { AgentAvatarsFlag } from "../../lib/avatars/agentAvatarsFlag";
import { agentsApi } from "../../lib/api/agentsApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { AgentsScreen } from "../agents/AgentsScreen";
import { agentSchema } from "../agents/agentContracts";
import { GroupChatScreen } from "../groups/GroupChatScreen";
import { ChatIdentity } from "./ChatIdentityHeader";
import { NyxBotHome } from "./NyxBotHome";

let mockLanguage: "en" | "zh-TW" = "en";
jest.mock("../../i18n/LanguageProvider", () => {
  const { translate } = jest.requireActual("../../i18n/translate");
  return {
    useAppLanguage: () => ({
      language: mockLanguage,
      t: (phrase: string, values?: Record<string, string | number>) =>
        translate(mockLanguage, phrase, values),
    }),
  };
});
jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});
jest.mock("../../lib/api/agentsApi", () => ({
  agentsApi: { list: jest.fn() },
}));
jest.mock("../../lib/api/groupsApi", () => ({
  groupsApi: { list: jest.fn() },
}));
jest.mock("../../lib/api/http", () => ({
  getCurrentUserProfileRequest: async () => ({}),
}));
jest.mock("../../hooks/useNetworkStatus", () => ({
  useNetworkStatus: () => ({ isConnected: true }),
}));
jest.mock("@react-navigation/native", () => ({
  ...jest.requireActual("@react-navigation/native"),
  useIsFocused: () => true,
}));
jest.mock("../../app/WorkspaceHeader", () => ({
  ...jest.requireActual("../../app/WorkspaceHeader"),
  useWorkspaceBack: () => {},
}));
jest.mock("../groups/useGroupRuntime", () => ({
  groupKeys: () => ["workspace", "groups", 0],
  useGroupAvailability: () => ({
    epoch: 0,
    active: true,
    online: true,
    available: () => true,
  }),
  useGroupChat: () => ({
    epoch: 0,
    active: true,
    online: true,
    data: {
      group: {
        id: "crew",
        name: "Research crew",
        members: [mockResearcher],
        working_agent_ids: [mockResearcher.id],
      },
      messages: [],
      collaboration_decisions: [],
      pending_actions: [],
    },
  }),
}));
jest.mock("../groups/useGroupsLive", () => ({ useGroupsLive: () => false }));
jest.mock("../groups/CrewVoiceComposer", () => ({
  CrewVoiceComposer: () => null,
}));
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: async () => null,
  setItem: async () => {},
}));
jest.mock("./useComposerDictation", () => ({
  useComposerDictation: () => ({ active: false }),
}));
jest.mock("./AssistantReplyBody", () => ({ AssistantReplyBody: () => null }));
jest.mock("./ComposerDictation", () => ({
  ComposerDictationFrame: ({ children }: { children: ReactNode }) => children,
  ComposerDictationButton: () => null,
  ComposerDictationReview: () => null,
}));
jest.mock("./useAssistantUploads", () => ({
  useAssistantUploads: () => ({ files: [], session: { sending: false } }),
}));
jest.mock("../notifications/useNotifications", () => ({
  useNotificationRuntime: () => ({ active: true }),
  useNotificationsEnabled: () => false,
  useVisibleThreadRead: () => {},
}));

const nyxbot = agentSchema.parse({
  id: "bot",
  name: "NyxBot",
  kind: "nyxbot",
  description: "",
  status: "running",
  activity: [{ conversation_id: "bot-thread", state: "thinking" }],
});
const mockResearcher = agentSchema.parse({
  id: "research",
  name: "Research",
  kind: "specialist",
  description: "Research documents",
  status: "running",
  activity: [],
  pending_acknowledgements: 2,
});
const reviewer = agentSchema.parse({
  id: "review",
  name: "Review",
  kind: "specialist",
  description: "Review documents",
  status: "idle",
  activity: [{ conversation_id: "review-thread", state: "approval" }],
});
const pastAgent = (name: string, status: "failed" | "completed") =>
  agentSchema.parse({
    id: name,
    name,
    kind: "specialist",
    description: "Past work",
    status: "idle",
    last_reply: {
      seq: 1,
      status,
      text: "Recorded result",
      created_at: "2026-11-01T00:00:00Z",
    },
  });
const noop = () => {};

configure({ defaultIncludeHiddenElements: true });
beforeEach(() => {
  mockLanguage = "en";
  Object.defineProperty(require("react-native").AppState, "currentState", {
    value: "active",
    configurable: true,
  });
  jest.mocked(agentsApi.list).mockResolvedValue({
    agents: [
      nyxbot,
      mockResearcher,
      reviewer,
      pastAgent("Reply", "completed"),
      pastAgent("Report", "failed"),
    ],
    limits: { max_live_subagents: 9, max_live_subagents_limit: 20 },
  });
  jest.mocked(groupsApi.list).mockResolvedValue([]);
});

function withRoster(ui: ReactElement, enabled = true) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  client.setQueryData(["workspace", "agents", "group-labels", 0], {
    agents: [mockResearcher],
  });
  return render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 400, height: 800 },
        insets: { top: 0, right: 0, bottom: 0, left: 0 },
      }}
    >
      <QueryClientProvider client={client}>
        <AgentAvatarsFlag value={enabled}>{ui}</AgentAvatarsFlag>
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

test.each([true, false])(
  "the agents list preserves its original status text with avatars=%s",
  async (enabled) => {
    mockLanguage = "zh-TW";
    await withRoster(<AgentsScreen />, enabled);
    const active = await screen.findByLabelText(
      enabled ? "管理 Research，閒置" : "Manage Research",
    );
    expect(within(active).getByText(/● Working/u)).toBeTruthy();
    expect(within(active).queryByTestId(/avatar-activity-/u)).toBeNull();
    const approval = screen.getByLabelText(
      enabled ? "管理 Review，等待核准" : "Manage Review",
    );
    expect(within(approval).getByText(/● Ready/u)).toBeTruthy();
    expect(
      Boolean(within(approval).queryByTestId("avatar-activity-approval")),
    ).toBe(enabled);
  },
);

test.each([true, false])(
  "NyxBot home preserves roster and away status text with avatars=%s",
  async (enabled) => {
    await withRoster(
      <NyxBotHome
        onChat={noop}
        onOpenCrew={noop}
        onNewAgent={noop}
        onNewCrew={noop}
      />,
      enabled,
    );
    const research = await screen.findByLabelText(
      enabled ? "Chat with Research, Idle" : "Chat with Research",
    );
    expect(within(research).getByText("Needs you")).toBeTruthy();
    expect(within(research).queryByTestId(/avatar-activity-/u)).toBeNull();
    const approval = screen.getByLabelText(
      enabled ? "Chat with Review, Waiting for approval" : "Chat with Review",
    );
    expect(within(approval).getByText("Idle")).toBeTruthy();
    expect(
      Boolean(within(approval).queryByTestId("avatar-activity-approval")),
    ).toBe(enabled);
    expect(
      within(screen.getByLabelText("Open Research's latest work")).getByText(
        "Running",
      ),
    ).toBeTruthy();
    expect(
      within(screen.getByLabelText("Open Reply's latest work")).getByText(
        "Replied",
      ),
    ).toBeTruthy();
    expect(
      within(screen.getByLabelText("Open Report's latest work")).getByText(
        "Failed",
      ),
    ).toBeTruthy();
    expect(
      Boolean(screen.queryAllByTestId("avatar-activity-thinking").length),
    ).toBe(enabled);
  },
);

test.each([true, false])(
  "the header keeps its original visible status and accessibility label with avatars=%s",
  async (enabled) => {
    mockLanguage = "zh-TW";
    const destroyed = { ...reviewer, status: "destroyed" as const };
    await withRoster(
      <>
        <ChatIdentity
          agent={destroyed}
          name="Review"
          detail="Preview"
          mood="stopping"
          status="Running"
          onPress={noop}
        />
        <ChatIdentity
          agent={destroyed}
          name="History"
          detail="Earlier"
          mood="failure"
          status="Needs you"
          onPress={noop}
        />
      </>,
      enabled,
    );
    const stopping = screen.getByLabelText(
      "Chatting with Review, Preview, Running. Choose agent",
    );
    const failure = screen.getByLabelText(
      "Chatting with History, Earlier, Needs you. Choose agent",
    );
    expect(within(stopping).getByText("Running")).toBeTruthy();
    expect(within(failure).getByText("Needs you")).toBeTruthy();
    expect(
      Boolean(within(stopping).queryByTestId("avatar-activity-stopping")),
    ).toBe(enabled);
    expect(
      Boolean(within(failure).queryByTestId("avatar-activity-failure")),
    ).toBe(enabled);
  },
);

test.each([true, false])(
  "the working crew keeps its original copy and live region with avatars=%s",
  async (enabled) => {
    mockLanguage = "zh-TW";
    await withRoster(
      <GroupChatScreen groupId="crew" onBack={noop} onSettings={noop} />,
      enabled,
    );
    const working = await screen.findByLabelText(
      "Agents working. Research is working…",
    );
    expect(working.props.accessibilityLiveRegion).toBe("polite");
    expect(within(working).getByText("Research is working…")).toBeTruthy();
    expect(
      Boolean(within(working).queryByTestId("avatar-activity-thinking")),
    ).toBe(enabled);
  },
);
