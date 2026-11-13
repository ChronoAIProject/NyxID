import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { NotificationsScreen } from "./NotificationsScreen";
import { notificationSchema } from "./notificationContracts";

const MOCK_C = `nyxa-${"a".repeat(32)}`;
const MOCK_AS_OF = "2026-11-10T08:06:00Z";
const mockRow = notificationSchema.parse({
  id: "n1",
  kind: "chat_update",
  title: "Support chat",
  source: { type: "conversation", id: MOCK_C },
  created_at: "2026-11-10T08:00:00Z",
  updated_at: "2026-11-10T08:05:00Z",
});
type MockPage = { items: (typeof mockRow)[]; next_cursor: string | null; as_of: string };
let mockFeed: { pages: MockPage[]; hasNextPage: boolean; fetching: boolean } = {
  pages: [],
  hasNextPage: false,
  fetching: false,
};
const mockFetches: string[] = [];
let mockRerender: (() => void) | undefined;

jest.mock("../../theme/ThemeContext", () => {
  const { lightColors } = jest.requireActual("../../theme/mobileTheme");
  return { useTheme: () => ({ colors: lightColors, mode: "light" }) };
});
jest.mock("../../lib/env", () => ({ firstPartyFrontendUrls: () => ["https://app.nyxid.test"] }));
jest.mock("../../app/WorkspaceHeader", () => ({
  useWorkspaceBack: () => {},
  useOpenWorkspaceChat: () => () => {},
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
  const { View, Text, Pressable } = jest.requireActual("react-native");
  return {
    SettingsGroup: ({ children }: { children: React.ReactNode }) => React.createElement(View, null, children),
    SettingsRow: ({ label, onPress }: { label: string; onPress?: () => void }) =>
      React.createElement(Pressable, { accessibilityRole: "button", accessibilityLabel: label, onPress }, React.createElement(Text, null, label)),
    useSettingsHeaderAction: () => {},
  };
});
jest.mock("./useNotifications", () => ({
  useNotificationRuntime: () => ({ active: true, online: true }),
  useNotificationsEnabled: () => true,
  useNotificationsLive: () => false,
  useNotificationSummary: () => ({
    data: { unread_count: 0, needs_decision_count: 0, by_source: [], by_source_truncated: false },
    refetch: () => Promise.resolve(),
  }),
  useMarkNotificationsRead: () => ({ mutate: () => {}, isPending: false }),
  useNotificationFeed: (filter: string) => {
    const React = jest.requireActual("react");
    const [, bump] = React.useReducer((n: number) => n + 1, 0);
    mockRerender = bump;
    return {
      data: { pages: mockFeed.pages },
      isPending: false,
      isError: false,
      error: undefined,
      hasNextPage: mockFeed.hasNextPage,
      isFetchingNextPage: mockFeed.fetching,
      fetchNextPage: () => {
        mockFetches.push(filter);
        return Promise.resolve();
      },
      refetch: () => Promise.resolve(),
    };
  },
}));

const navigation = { goBack: jest.fn(), push: jest.fn(), setOptions: jest.fn() } as never;
const route = { key: "r", name: "Notifications", params: undefined } as never;
const page = (items: (typeof mockRow)[], next_cursor: string | null): MockPage => ({ items, next_cursor, as_of: MOCK_AS_OF });
const serverReplies = async (next: typeof mockFeed) => {
  await act(async () => {
    mockFeed = next;
    mockRerender?.();
  });
};

describe("NotificationsScreen paging through pages the server filtered empty", () => {
  beforeEach(() => {
    mockFetches.length = 0;
    mockRerender = undefined;
  });

  it("keeps fetching past an empty first page with a cursor and shows the row it finds, never the final empty copy", async () => {
    mockFeed = { pages: [page([], "p2")], hasNextPage: true, fetching: false };
    await render(<NotificationsScreen navigation={navigation} route={route} />);
    expect(mockFetches).toEqual(["all"]);
    expect(screen.getByLabelText("Looking for updates")).toBeTruthy();
    expect(screen.queryByText("You're all caught up.")).toBeNull();
    expect(screen.queryByText("Load more")).toBeNull();
    await serverReplies({ pages: [page([], "p2"), page([mockRow], null)], hasNextPage: false, fetching: false });
    expect(screen.getByText("Support chat")).toBeTruthy();
    expect(screen.queryByLabelText("Looking for updates")).toBeNull();
    expect(screen.queryByText("You're all caught up.")).toBeNull();
    expect(mockFetches).toEqual(["all"]);
  });

  it("shows the final empty copy only once the server is exhausted", async () => {
    mockFeed = { pages: [page([], null)], hasNextPage: false, fetching: false };
    await render(<NotificationsScreen navigation={navigation} route={route} />);
    expect(screen.getByText("You're all caught up.")).toBeTruthy();
    expect(screen.queryByLabelText("Looking for updates")).toBeNull();
    expect(mockFetches).toEqual([]);
  });

  it("does not loop on an unchanged cursor and offers Load more instead", async () => {
    mockFeed = { pages: [page([], "same")], hasNextPage: true, fetching: false };
    await render(<NotificationsScreen navigation={navigation} route={route} />);
    expect(mockFetches).toEqual(["all"]);
    await serverReplies({ pages: [page([], "same"), page([], "same")], hasNextPage: true, fetching: false });
    expect(mockFetches).toEqual(["all"]);
    expect(screen.queryByLabelText("Looking for updates")).toBeNull();
    expect(screen.queryByText("You're all caught up.")).toBeNull();
    await fireEvent.press(screen.getByRole("button", { name: "Load more" }));
    expect(mockFetches).toEqual(["all", "all"]);
  });
});
