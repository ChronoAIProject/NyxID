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
function setup() {
  const events: string[] = [];
  const h = hookHarness<E>({
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
    list: () => all().find((n) => n.type === "ChatList")!,
  };
}
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
