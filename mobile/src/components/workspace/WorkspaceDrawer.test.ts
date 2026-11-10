import { WORKSPACE_HEADER_ROW_HEIGHT } from "./workspaceHeaderMetrics";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { sidebarBottomSpace } from "./workspaceSidebarModel";
import { lightColors } from "../../theme/mobileTheme";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
type E = { type: unknown; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const MARKERS = { conversations: { [`nyxa-${"a".repeat(32)}`]: 2 }, groups: {} };
function setup(
  notifications: {
    enabled: boolean;
    unread?: number;
    needsDecision?: number;
  } = { enabled: false },
) {
  const events: string[] = [];
  const indicatorCalls: boolean[] = [];
  const h = hookHarness<E>({
    "../../features/notifications/useNotifications": {
      useNotificationIndicator: (active: boolean) => {
        indicatorCalls.push(active);
        const unread = notifications.unread ?? 0;
        return {
          enabled: notifications.enabled,
          live: false,
          unread,
          needsDecision: notifications.needsDecision ?? 0,
          badge: unread > 9 ? "9+" : unread > 0 ? String(unread) : "",
          markers: MARKERS,
        };
      },
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Text: "Text",
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (v: unknown) => v, hairlineWidth: 0.5 },
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
    },
    "react-native-safe-area-context": {
      SafeAreaView: "Safe",
      useSafeAreaInsets: () => ({ bottom: 34 }),
    },
    "expo-linear-gradient": { LinearGradient: "Gradient" },
    "lucide-react-native": Object.fromEntries(
      [
        "Bell",
        "CalendarClock",
        "Monitor",
        "Plug",
        "Search",
        "ScanQrCode",
        "Terminal",
        "Settings",
        "ShieldCheck",
        "SquarePen",
        "UserPlus",
      ].map((n) => [n, n]),
    ),
    "@tanstack/react-query": { useQuery: () => ({ data: { total: 3 } }) },
    "../../lib/api/mobileApi": {},
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 5 },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../lib/notifications/approvalRefreshSignal": {
      subscribeToApprovalRefreshSignals: () => () => {},
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../brand/NyxidLogo": { NyxidLogo: "Logo" },
    "./WorkspaceChatList": { WorkspaceChatList: "ChatList" },
    "./WorkspaceRecents": { WorkspaceRecents: "Recents" },
    "./workspaceSidebarModel": { sidebarBottomSpace, SIDEBAR_BAR_GAP: 12 },
  });
  const Drawer = h.load(
    fileURLToPath(new URL("./WorkspaceDrawer.tsx", import.meta.url)),
  ).WorkspaceDrawer as any;
  h.mount(() =>
    Drawer({
      visible: true,
      onHome: () => events.push("home"),
      onQuickAction: (kind: string) => events.push(kind),
      onNavigate: (name: string) => events.push(name),
      onBrowse: (name: string) => events.push(name),
      onCreateAgent: () => events.push("create"),
    }),
  );
  const all = () => [
    ...nodes(h.value()),
    ...nodes(
      nodes(h.value()).find((n) => n.type === "ChatList")!.props.scrollHeader,
    ),
  ];
  const label = (name: string) => {
    const node = all().find((n) => n.props.accessibilityLabel === name);
    assert.ok(node, name);
    return node;
  };
  return {
    h,
    events,
    all,
    label,
    indicatorCalls,
    list: () => all().find((n) => n.type === "ChatList")!,
  };
}
test("the Notifications row appears only with the flag, counts unread capped at 9+, turns amber for decisions and navigates", () => {
  const off = setup({ enabled: false, unread: 4 });
  assert.equal(
    off.all().some((n) => n.props.accessibilityLabel?.startsWith("Notifications")),
    false,
    "hidden while the flag is off",
  );
  assert.deepEqual(off.indicatorCalls, [true], "the indicator follows the visible sidebar");
  off.h.unmount();
  const on = setup({ enabled: true, unread: 12, needsDecision: 1 });
  const row = on.label("Notifications, 12 unread, 1 needs your decision");
  const header = nodes(on.list().props.scrollHeader);
  assert.equal(
    header.filter((n) => n.type === "Pressable").map((n) => n.props.accessibilityLabel)[2],
    "Notifications, 12 unread, 1 needs your decision",
    "Notifications leads the Workspace destinations",
  );
  const count = nodes(row).find((n) => n.props.testID === "sidebar-notifications-count")!;
  assert.equal(count.props.children, "9+");
  assert.equal(on.list().props.unread, MARKERS, "thread rows read the indicator's markers, one summary for the sidebar");
  const recents = nodes(on.list().props.scrollFooter).find((n) => n.type === "Recents")!;
  assert.equal(recents.props.unread, MARKERS);
  assert.equal(count.props.style[1].backgroundColor, lightColors.warningTone.bg);
  assert.equal(count.props.style[1].color, lightColors.warningTone.text);
  row.props.onPress();
  assert.deepEqual(on.events, ["Notifications"]);
  on.h.unmount();
  const quiet = setup({ enabled: true, unread: 2 });
  const plain = nodes(quiet.label("Notifications, 2 unread")).find(
    (n) => n.props.testID === "sidebar-notifications-count",
  )!;
  assert.equal(plain.props.children, "2");
  assert.equal(plain.props.style[1].backgroundColor, lightColors.navActive);
  quiet.h.unmount();
  const empty = setup({ enabled: true });
  assert.equal(
    nodes(empty.label("Notifications")).some((n) => n.props.testID === "sidebar-notifications-count"),
    false,
    "no badge without unread rows",
  );
  empty.h.unmount();
});
test("sidebar header uses the logo and circular Search; Home is exclusive to logo and pinned Chat", () => {
  const x = setup();
  const search = x.label("Search");
  assert.equal(search.props.style.width, 44);
  assert.equal(search.props.style.borderRadius, 22);
  search.props.onPress();
  x.label("NyxID home").props.onPress();
  x.label("New chat").props.onPress();
  x.label("Settings").props.onPress();
  assert.deepEqual(x.events, [
    "WorkspaceSearch",
    "home",
    "home",
    "AccountSettings",
  ]);
  assert.equal(
    x
      .all()
      .some((n) =>
        [
          "Close navigation",
          "Home",
          "Chats",
          "Open account settings",
          "Open usage and credits",
        ].includes(n.props.accessibilityLabel),
      ),
    false,
  );
  assert.equal(
    x
      .all()
      .some(
        (n) =>
          n.type === "Text" &&
          ["QUICK ACTIONS", "YOUR WORKSPACE"].includes(n.props.children),
      ),
    false,
  );
  assert.equal(x.all().find((n) => n.type === "Logo")!.props.height, 28);
  x.h.unmount();
});
test("Quick actions precede Workspace with no placeholders and a live Activity pending count", () => {
  const x = setup();
  const header = nodes(x.list().props.scrollHeader);
  const title = header.find((n) => n.props.children === "Workspace")!;
  assert.deepEqual(Object.assign({}, ...title.props.style), {
    ...sidebarSectionTitle,
    paddingHorizontal: 12,
    paddingTop: 8,
    paddingBottom: 10,
    color: lightColors.textPrimary,
  });
  assert.deepEqual(
    header
      .filter((n) => n.type === "Pressable")
      .map((n) => n.props.accessibilityLabel),
    [
      "Scan QR code",
      "Terminal login code",
      "Automations",
      "Machines",
      "Plugins",
      "Activity, 3 pending",
      "Create new agent",
    ],
  );
  assert.equal(
    header.some((n) =>
      /coming soon|Soon|Artifacts/.test(
        n.props.accessibilityLabel ?? n.props.children ?? "",
      ),
    ),
    false,
  );
  x.label("Scan QR code").props.onPress();
  x.label("Terminal login code").props.onPress();
  x.label("Machines").props.onPress();
  x.label("Create new agent").props.onPress();
  assert.deepEqual(x.events, ["scanner", "terminal", "Machines", "create"]);
  const footer = x.list().props.scrollFooter;
  assert.equal(footer.type, "Recents");
  footer.props.onSeeAll();
  assert.equal(x.events.at(-1), "WorkspaceRecents");
  x.h.unmount();
});
test("absolute bottom controls reserve their measured height and safe inset above the last row at max scroll", () => {
  const x = setup();
  const bar = () =>
    x.all().find((n) => n.props.testID === "sidebar-bottom-bar")!;
  assert.equal(bar().props.style[0].position, "absolute");
  assert.equal(bar().props.style[1].bottom, 46);
  for (const height of [52, 76, 104]) {
    bar().props.onLayout({ nativeEvent: { layout: { height } } });
    x.h.flush();
    const padding =
      x.list().props.scrollProps.contentContainerStyle[1].paddingBottom;
    assert.equal(padding, sidebarBottomSpace(height, 34));
    for (const viewport of [600, 874, 1024]) {
      const barTop = viewport - 46 - height;
      const lastRowEndAtMaxScroll = viewport - padding;
      assert.ok(lastRowEndAtMaxScroll <= barTop - 12);
    }
  }
  const pill = x.label("New chat");
  const style = pill.props.style({ pressed: false })[0];
  assert.equal(style.minHeight, 52);
  assert.equal(style.backgroundColor, "#FFFFFF");
  const gear = x.label("Settings");
  assert.ok(gear.props.style[0].height >= 44);
  x.h.unmount();
});

test("scrolling rows fade into an opaque control band while max-scroll rows remain above the fade", () => {
  const x = setup();
  const fade = () =>
    x.all().find((n) => n.props.testID === "sidebar-bottom-fade")!;
  const props = () => x.list().props.scrollProps;
  const scroll = (y: number) => {
    props().onScroll({
      nativeEvent: {
        contentOffset: { y },
        layoutMeasurement: { height: 700 },
        contentSize: { height: 1100 },
      },
    });
    x.h.flush();
  };
  const reserve = sidebarBottomSpace(52, 34);
  assert.equal(fade().props.pointerEvents, "none");
  assert.equal(fade().props.colors[1], lightColors.sidebarBase);
  assert.equal(fade().props.style.height, reserve + 52);
  assert.equal(fade().props.locations[1] * fade().props.style.height, 52);
  scroll(400);
  assert.equal(fade().props.style.height, reserve);
  assert.equal(fade().props.locations[1], 0);
  assert.equal(props().contentContainerStyle[1].paddingBottom, reserve);
  scroll(100);
  assert.equal(fade().props.style.height, reserve + 52);
  x.h.unmount();
});

test("sidebar logo and Search are centered in the shared slab header row", () => {
  const x = setup();
  const row = x
    .all()
    .find((n) => n.props.testID === "workspace-sidebar-header-row")!;
  assert.equal(row.props.style.height, WORKSPACE_HEADER_ROW_HEIGHT);
  assert.equal(row.props.style.paddingTop, undefined);
  assert.equal(row.props.style.paddingBottom, undefined);
  const search = x.label("Search");
  const center =
    (row.props.style.height - search.props.style.height) / 2 +
    search.props.style.height / 2;
  assert.ok(Math.abs(center - WORKSPACE_HEADER_ROW_HEIGHT / 2) <= 0.5);
  x.h.unmount();
});
