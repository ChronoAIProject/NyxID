import { fireEvent, render, screen } from "@testing-library/react-native";
import { NotificationsScreen, NotificationRow } from "./NotificationsScreen";
import { notificationSentence } from "./notificationPresentation";
import { notificationSchema } from "./notificationContracts";
import { lightColors } from "../../theme/mobileTheme";

const MOCK_C = `nyxa-${"a".repeat(32)}`;
const MOCK_AGENT = "7c0f6a8e-2b2e-4a1a-9f1f-0d2c5a6b7c8d";
const MOCK_AS_OF = "2026-11-10T08:06:00Z";
const mockReads: unknown[] = [];
const mockOpened: unknown[] = [];
const mockFilters: string[] = [];
const mockFetched: string[] = [];
let mockUnreadCount = 1;
let mockHasNextPage = true;
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
    const footer = screen.getByRole("button", { name: "Mark all notifications as read" });
    expect(footer).toBeEnabled();
    await fireEvent.press(footer);
    expect(mockReads).toEqual([{ all: true, before: MOCK_AS_OF }]);
    await fireEvent.press(screen.getByRole("tab", { name: /Needs you/ }));
    expect(mockFilters.at(-1)).toBe("needs_you");
  });

  it("keeps the global read action available on an empty filter when other kinds are unread", async () => {
    mockHasNextPage = false;
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    await fireEvent.press(screen.getByRole("tab", { name: "Automations" }));
    expect(screen.getByText("No automation results yet.")).toBeTruthy();
    const footer = screen.getByRole("button", { name: "Mark all notifications as read" });
    expect(footer).toBeEnabled();
    await fireEvent.press(footer);
    expect(mockReads).toEqual([{ all: true, before: MOCK_AS_OF }]);
  });

  it("disables the global read action when the summary has no unread notifications", async () => {
    mockUnreadCount = 0;
    await render(<NotificationsScreen navigation={navigation} route={{ key: "r", name: "Notifications", params: undefined }} />);
    expect(screen.getByRole("button", { name: "Mark all notifications as read" })).toBeDisabled();
  });
});


it.each([
  ["automation_result", "Morning digest finished", "automation"],
  ["automation_problem", "Morning digest failed", "automation"],
  ["needs_decision", "Researcher needs your decision in Launch plan", "agent"],
  ["agent_update", "Researcher reported back in Launch plan", "agent"],
  ["chat_update", "Researcher replied in Launch plan on Lark", "agent"],
  ["group_mention", "Researcher mentioned you in Launch plan", "crew"],
])("renders the %s source avatar, badge and sentence", async (kind, sentence, avatar) => {
  const agent = { type: "agent", id: "researcher", name: "Researcher", kind: "specialist" };
  const item = notificationSchema.parse({ id: "source-row", kind, actor: avatar === "automation" ? { type: "automation", name: "Morning digest" } : agent,
    participants: avatar === "crew" ? [agent] : [], platform: "lark",
    source: { type: avatar === "crew" ? "collaboration_group" : "conversation", id: MOCK_C, label: "Launch plan" },
    count: 3, preview: "The release checklist is ready.", created_at: MOCK_AS_OF, updated_at: MOCK_AS_OF });
  await render(<NotificationRow item={item} now={Date.parse(MOCK_AS_OF)} onPress={() => {}} />);
  expect(screen.getByTestId(`notification-avatar-${avatar}`)).toBeTruthy();
  expect(screen.getByTestId(`notification-badge-${kind}`)).toBeTruthy();
  const badge = {
    automation_result: lightColors.successTone.text,
    automation_problem: lightColors.dangerTone.text,
    needs_decision: lightColors.warningTone.text,
    agent_update: lightColors.textMuted,
    group_mention: lightColors.textMuted,
  } as const;
  if (kind in badge) {
    const [icon] = screen.getByTestId(`notification-badge-${kind}`).queryAll(
      (node) => node.props["data-testid"] === `notification-kind-icon-${kind}`,
    );
    expect(icon?.props.stroke).toBe(badge[kind as keyof typeof badge]);
  }
  expect(screen.getByTestId("notification-source-row").props.accessibilityLabel).toContain(sentence);
  expect(screen.getByText("3 new")).toBeTruthy();
  expect(screen.queryByTestId("notification-unread-dot")).toBeNull();
});


it.each(["telegram", "lark", "feishu", "discord", "slack", "whatsapp", "x", "twitter", "unknown"])(
  "renders a single bare %s platform glyph without a nested bot badge",
  async (platform) => {
    const item = notificationSchema.parse({ id: "platform-row", kind: "chat_update", platform,
      actor: { type: "agent", name: "Researcher", id: "researcher", kind: "specialist" },
      source: { type: "channel_chat", id: MOCK_C, label: "Launch room" }, created_at: MOCK_AS_OF, updated_at: MOCK_AS_OF });
    await render(<NotificationRow item={item} now={Date.parse(MOCK_AS_OF)} onPress={() => {}} />);
    const badge = screen.getByTestId("notification-badge-chat_update");
    // Composite catalog marks render a wrapper plus a second SVG for Bot.
    const svgs = badge.queryAll(node => typeof node.type === "string" && node.type.startsWith("RNSVGSvg"));
    expect(svgs).toHaveLength(1);
    expect(badge.queryAll(node => node.props.testID === `channel-platform-${platform}` || node.props["data-testid"] === `channel-platform-${platform}`).length).toBeGreaterThan(0);
    expect(badge.queryAll(node => String(node.props["data-slug"] ?? "").startsWith("api-"))).toHaveLength(0);
  },
);

it.each([
  ["channel chat", "chat_update", { title: "Launch · support bot · Guest group chat", source: { type: "channel_chat", id: MOCK_C, channel_id: "ch", label: "Guest group chat" } }, "Launch · support bot"],
  ["linked chat", "chat_update", { source: { type: "conversation", id: MOCK_C, label: "Morgan · Telegram" } }, "Researcher"],
  ["matching bot label", "chat_update", { title: "Launch plan · Launch plan", source: { type: "channel_chat", id: MOCK_C, channel_id: "ch", label: "Launch plan" } }, null],
  ["thread", "agent_update", {}, null],
  ["decision", "needs_decision", {}, null],
  ["crew", "agent_update", { source: { type: "group", id: "g", group_id: "g", label: " Launch plan " } }, null],
  ["mention", "group_mention", { source: { type: "collaboration_group", id: "g", group_id: "g", label: "Launch crew" } }, null],
  ["automation with a named runner", "automation_result", {}, "Researcher"],
  ["automation without a named runner", "automation_result", { actor: { type: "automation", name: "Morning digest" } }, null],
  ["automation problem without a named runner", "automation_problem", { actor: { type: "automation", name: "Morning digest" } }, null],
] as const)("keeps only additional chip context for %s", async (_scenario, kind, extra, context) => {
  const item = notificationSchema.parse({ id: "context-row", kind,
    actor: { type: "agent", name: "researcher", display_name: "Researcher", id: "researcher", kind: "specialist" },
    source: { type: "conversation", id: MOCK_C, label: "Launch plan" }, created_at: MOCK_AS_OF, updated_at: MOCK_AS_OF, ...extra });
  await render(<NotificationRow item={item} now={Date.parse(MOCK_AS_OF)} onPress={() => {}} />);
  const chip = screen.queryByTestId("notification-context");
  if (context) {
    expect(chip).toHaveTextContent(context);
    expect(context).not.toBe(notificationSentence(item).target);
  } else expect(chip).toBeNull();
});
