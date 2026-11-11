import { fireEvent, render, screen } from "@testing-library/react-native";
import { NotificationsScreen } from "./NotificationsScreen";
import { notificationSchema } from "./notificationContracts";

const MOCK_C = `nyxa-${"a".repeat(32)}`;
const MOCK_AGENT = "7c0f6a8e-2b2e-4a1a-9f1f-0d2c5a6b7c8d";
const MOCK_AS_OF = "2026-11-10T08:06:00Z";
const mockReads: unknown[] = [];
const mockOpened: unknown[] = [];
const mockFilters: string[] = [];
const mockFetched: string[] = [];
let mockUnreadCount = 1;
let mockHasNextPage = true;
let mockHeader: { label: string; enabled: boolean; onPress: () => void } | undefined;
const mockRows = [
  notificationSchema.parse({
    id: "n1",
    kind: "chat_update",
    title: "Support chat",
    preview: "Thanks!",
    count: 3,
    source: { type: "channel_chat", id: MOCK_C, channel_id: "ch", agent_id: "agent-one" },
    created_at: "2026-11-10T08:00:00Z",
    updated_at: "2026-11-10T08:05:00Z",
  }),
  notificationSchema.parse({
    id: "n2",
    kind: "needs_decision",
    title: "Delete repo?",
    read: true,
    source: { type: "conversation", id: MOCK_C },
    created_at: "2026-11-10T08:00:00Z",
    updated_at: "2026-11-10T08:04:00Z",
  }),
  notificationSchema.parse({
    id: "n3",
    kind: "chat_update",
    title: "Sales room",
    source: { type: "channel_chat", id: "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", channel_id: "ch", agent_id: MOCK_AGENT },
    url: `https://app.nyxid.test/assistant?agent=${MOCK_AGENT}&details=channels`,
    created_at: "2026-11-10T08:00:00Z",
    updated_at: "2026-11-10T08:03:00Z",
  }),
];

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});
jest.mock("../../lib/env", () => ({
  firstPartyFrontendUrls: () => ["https://app.nyxid.test"],
}));
jest.mock("../../app/WorkspaceHeader", () => ({
  useWorkspaceBack: () => {},
  useOpenWorkspaceChat: () => (params: unknown) => mockOpened.push(params),
}));
jest.mock("../../components/workspace/WorkspaceUI", () => {
  const React = jest.requireActual("react");
  const { View, Text } = jest.requireActual("react-native");
  return {
    WorkspacePage: ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
    Notice: ({ message }: { message: string }) => React.createElement(Text, null, message),
  };
});
jest.mock("../../components/settings/SettingsList", () => {
  const React = jest.requireActual("react");
  const { View, Text } = jest.requireActual("react-native");
  return {
    SettingsGroup: ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
    SettingsRow: ({ label }: { label: string }) => React.createElement(Text, null, label),
    useSettingsHeaderAction: (_navigation: unknown, value: typeof mockHeader) => {
      mockHeader = value;
    },
  };
});
jest.mock("./useNotifications", () => ({
  useNotificationRuntime: () => ({ active: true, online: true }),
  useNotificationsEnabled: () => true,
  useNotificationsLive: () => false,
  useNotificationSummary: () => ({
    data: { unread_count: mockUnreadCount, needs_decision_count: 1, by_source: [], by_source_truncated: false },
    refetch: () => Promise.resolve(),
  }),
  useMarkNotificationsRead: () => ({ mutate: (scope: unknown) => mockReads.push(scope), isPending: false }),
  useNotificationFeed: (filter: string) => {
    mockFilters.push(filter);
    return {
      data: { pages: [{ items: mockRows, next_cursor: "more", as_of: MOCK_AS_OF }] },
      isPending: false,
      isError: false,
      error: undefined,
      hasNextPage: mockHasNextPage,
      isFetchingNextPage: false,
      fetchNextPage: () => {
        mockFetched.push(filter);
        return Promise.resolve();
      },
      refetch: () => Promise.resolve(),
    };
  },
}));

const navigation = { goBack: jest.fn(), push: jest.fn(), setOptions: jest.fn() } as never;

describe("NotificationsScreen", () => {
  beforeEach(() => {
    mockReads.length = 0;
    mockOpened.length = 0;
    mockFilters.length = 0;
    mockFetched.length = 0;
    mockUnreadCount = 1;
    mockHasNextPage = true;
    mockHeader = undefined;
    (navigation as { push: jest.Mock }).push.mockClear();
  });

  it("renders rows with previews, counts and unread state, and tapping marks read then opens the thread", async () => {
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    expect(screen.getByText("Support chat")).toBeTruthy();
    expect(screen.getByText("Thanks!")).toBeTruthy();
    expect(screen.getByText("3 new")).toBeTruthy();
    // The filter tab and the pending card's chip both read "Needs you".
    expect(screen.getAllByText("Needs you").length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByTestId("notification-unread-dot")).toBeNull();
    expect(screen.getByTestId("notification-n1").props.accessibilityState).toEqual({ selected: true });
    await fireEvent.press(screen.getByTestId("notification-n1"));
    expect(mockReads).toEqual([{ ids: ["n1"] }]);
    expect(mockOpened).toEqual([
      { conversationId: MOCK_C, agentId: "agent-one", home: false, draft: false, openLatest: false },
    ]);
    await fireEvent.press(screen.getByTestId("notification-n2"));
    expect(mockReads).toHaveLength(1);
    expect(mockOpened).toHaveLength(2);
  });

  it("opens an organization bot's public transcript on the agent's details page from its canonical link", async () => {
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    await fireEvent.press(screen.getByTestId("notification-n3"));
    expect(mockReads).toEqual([{ ids: ["n3"] }]);
    expect(mockOpened).toEqual([]);
    expect((navigation as { push: jest.Mock }).push).toHaveBeenCalledWith("AgentDetail", { agentId: MOCK_AGENT });
  });

  it("marks everything read behind the server fence and switches mockFilters", async () => {
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    expect(mockHeader?.label).toBe("Mark all notifications as read");
    expect(mockHeader?.enabled).toBe(true);
    mockHeader!.onPress();
    expect(mockReads).toEqual([{ all: true, before: MOCK_AS_OF }]);
    await fireEvent.press(screen.getByRole("tab", { name: /Needs you/ }));
    expect(mockFilters.at(-1)).toBe("needs_you");
  });

  it("keeps the global read action available on an empty filter when other kinds are unread", async () => {
    mockHasNextPage = false;
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    await fireEvent.press(screen.getByRole("tab", { name: "Automations" }));
    expect(screen.getByText("No automation results yet.")).toBeTruthy();
    expect(mockHeader).toMatchObject({
      label: "Mark all notifications as read",
      enabled: true,
    });
    mockHeader!.onPress();
    expect(mockReads).toEqual([{ all: true, before: MOCK_AS_OF }]);
  });

  it("disables the global read action when the summary has no unread notifications", async () => {
    mockUnreadCount = 0;
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    expect(mockHeader).toMatchObject({
      label: "Mark all notifications as read",
      enabled: false,
    });
  });
});
