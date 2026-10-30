import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { darkColors, lightColors } from "../../theme/mobileTheme";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import * as search from "./workspaceSearchModel";

type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
function setup(
  recents = false,
  initialAgent?: string,
  history = false,
  dark = false,
) {
  let epoch = 7,
    focused = true,
    refreshed = 0,
    agentRefresh = 0,
    more = 0;
  const actions: any[] = [],
    themes: string[] = [],
    scopes: unknown[] = [];
  let keyboardDismissals = 0;
  let queryOptions: any;
  const rows = [
    {
      key: "chat:first",
      title: "Release report",
      activityAt: "2026-10-28T00:00:00Z",
      target: {
        kind: "agent",
        agentId: "helper",
        conversationId: "nyxa-first",
      },
    },
    {
      key: "crew:second",
      title: "Design crew",
      activityAt: "2026-10-27T00:00:00Z",
      target: { kind: "group", groupId: "crew" },
    },
  ];
  const data = {
    rows,
    conversations: [
      {
        id: "nyxa-first",
        title: "Release report",
        agent: { id: "helper", name: "helper-handle" },
        last_message_at: "2026-10-28T00:00:00Z",
        message_count: 2,
      },
    ],
    crews: [
      {
        id: "crew",
        name: "Design crew",
        members: [{ id: "helper", name: "helper-handle", kind: "specialist" }],
        working_agent_ids: [],
        last_message_at: "2026-10-27T00:00:00Z",
        message_count: 2,
      },
    ],
    online: true,
    active: true,
    hasNextPage: true,
    loadingMore: false,
    refresh: async () => refreshed++,
    loadMore: () => more++,
  };
  let route = {
    name: history ? "AssistantHistory" : "WorkspaceRecents",
    params: initialAgent ? { agentId: initialAgent } : undefined,
  };
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "Text",
          "View",
          "Pressable",
          "TextInput",
          "ScrollView",
          "FlatList",
          "SectionList",
          "RefreshControl",
        ].map((v) => [v, v]),
      ),
      StyleSheet: { create: (v: any) => v },
      Keyboard: { dismiss: () => keyboardDismissals++ },
    },
    "lucide-react-native": Object.fromEntries(
      [
        "Search",
        "X",
        "CalendarClock",
        "Monitor",
        "Plug",
        "ShieldCheck",
        "UserPlus",
        "Users",
        "MessagesSquare",
        "ScanQrCode",
        "Terminal",
        "Settings",
        "Bell",
        "SunMoon",
        "LockKeyhole",
        "CircleDollarSign",
        "FileText",
        "Moon",
        "Sun",
        "MessageSquare",
      ].map((name) => [name, name]),
    ),
    "../AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
    "@react-navigation/native": { useIsFocused: () => focused },
    "@tanstack/react-query": {
      useQuery: (options: any) => {
        queryOptions = options;
        return {
          data: {
            agents: [
              {
                id: "helper",
                name: "helper-handle",
                display_name: "Research helper",
                kind: "specialist",
                status: "idle",
              },
              { id: "nyxbot", name: "NyxBot", kind: "nyxbot", status: "idle" },
            ],
          },
          refetch: async () => agentRefresh++,
        };
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/api/agentsApi": { agentsApi: { list: () => ({}) } },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        colors: dark ? darkColors : lightColors,
        setPreference: (v: string) => themes.push(v),
      }),
    },
    "./useWorkspaceRecents": {
      useWorkspaceRecents: (_: boolean, scope: unknown) => {
        scopes.push(scope);
        return data;
      },
    },
    "./WorkspaceRecents": { WorkspaceRecentRow: "Recent" },
    "./WorkspaceUI": { WorkspacePage: "Page" },
    "./workspaceSearchModel": search,
    "../../app/navigationHierarchy": {
      openWorkspacePageAction: (name: string, params: any) => ({
        kind: "page",
        name,
        params,
      }),
      openWorkspaceSheetAction: (name: string, params: any) => ({
        kind: "settings",
        name,
        params,
      }),
    },
    "../../app/workspaceChatNavigation": {
      workspaceChatNavigationAction: (_: any, target: any) => ({
        kind: "chat",
        target,
      }),
    },
  });
  const mod = h.load(
    fileURLToPath(new URL("./WorkspaceBrowseScreen.tsx", import.meta.url)),
  ) as any;
  const navigation = {
    dispatch: (a: any) => actions.push(a),
    getParent: () => ({ getState: () => ({}) }),
  };
  h.mount(() =>
    recents
      ? mod.WorkspaceRecentsScreen({ navigation, route })
      : mod.WorkspaceSearchScreen({ navigation, route }),
  );
  const all = () => nodes(h.value());
  const list = () =>
    all().find((n) => n.type === (recents ? "FlatList" : "SectionList"))!;
  const field = () =>
    all().find(
      (n) =>
        typeof n.type === "function" && n.type.name === "BrowseSearchField",
    )!;
  const type = (value: string) => {
    field().props.onChange(value);
    h.flush();
  };
  const result = (label: string) => {
    const item = list()
      .props.sections.flatMap((s: any) => s.data)
      .find((e: any) => e.label === label);
    assert.ok(item, label);
    return list().props.renderItem({ item });
  };
  return {
    h,
    all,
    list,
    field,
    type,
    result,
    actions,
    themes,
    data,
    scopes,
    query: () => queryOptions,
    setEpoch: () => epoch++,
    blur: () => {
      focused = false;
      h.render();
    },
    changeRoute: (agentId?: string) => {
      route = { ...route, params: agentId ? { agentId } : undefined };
      h.render();
    },
    counts: () => ({ refreshed, agentRefresh, more, keyboardDismissals }),
  };
}

test("Search filters loaded real sources and opens chats, agents, crews, pages and Settings with keyboard dismissal", async () => {
  const x = setup();
  assert.equal(x.field().props.autoFocus, true);
  assert.equal(x.list().props.keyboardShouldPersistTaps, "handled");
  assert.equal(x.list().props.stickySectionHeadersEnabled, false);
  for (const [query, label] of [
    ["release", "Release report"],
    ["helper-handle", "Research helper"],
    ["design", "Design crew"],
    ["machine", "Machines"],
    ["settings text", "Settings: Display"],
    ["theme dark", "Theme: Dark"],
  ]) {
    x.type(query!);
    const row = x.result(label!);
    assert.ok(row.props.style({ pressed: false })[0].minHeight >= 44);
    row.props.onPress();
  }
  assert.deepEqual(
    x.actions.map((a) => a.kind),
    ["chat", "chat", "chat", "page", "settings"],
  );
  assert.equal(x.actions[0].target.conversationId, "nyxa-first");
  assert.equal(x.actions[1].target.agentId, "helper");
  assert.equal(x.actions[1].target.conversationId, undefined);
  assert.equal(x.actions[2].target.groupId, "crew");
  assert.equal(x.actions[3].name, "Machines");
  assert.equal(x.actions[4].params.section, "display");
  assert.deepEqual(x.themes, ["dark"]);
  assert.equal(x.counts().keyboardDismissals, 6);
  await x.list().props.refreshControl.props.onRefresh();
  assert.equal(x.counts().refreshed, 1);
  assert.equal(x.counts().agentRefresh, 1);
  assert.equal(x.query().queryKey[3], 7);
  x.h.unmount();
});

test("Search destinations use their sidebar glyphs and actual agent/crew identity marks", () => {
  const x = setup();
  const expected = new Map([
    ["Automations", "CalendarClock"],
    ["Machines", "Monitor"],
    ["Plugins", "Plug"],
    ["Activity", "ShieldCheck"],
    ["Create new agent", "UserPlus"],
    ["Scan QR code", "ScanQrCode"],
    ["Terminal login code", "Terminal"],
  ]);
  for (const [label, icon] of expected) {
    const row = x.result(label);
    assert.ok(
      nodes(row).some((node) => node.type === icon),
      label,
    );
  }
  for (const [query, label, icon] of [
    ["settings", "Settings: Profile", "Settings"],
    ["settings", "Settings: Account & security", "ShieldCheck"],
    ["settings", "Settings: Display", "SunMoon"],
    ["settings", "Settings: Notifications", "Bell"],
    ["usage", "Usage & credits", "CircleDollarSign"],
  ]) {
    x.type(query!);
    assert.ok(
      nodes(x.result(label!)).some((node) => node.type === icon),
      label,
    );
  }
  x.type("helper-handle");
  const agent = nodes(x.result("Research helper")).find(
    (node) => node.type === "AgentAvatar",
  )!;
  assert.equal(agent.props.agent.id, "helper");
  x.type("design");
  const crew = nodes(x.result("Design crew")).find(
    (node) => node.type === "AgentAvatarStack",
  )!;
  assert.equal(crew.props.agents[0].id, "helper");
  x.type("");
  x.result("Scan QR code").props.onPress();
  x.result("Terminal login code").props.onPress();
  assert.deepEqual(
    x.actions.map((action) => [action.kind, action.name, { ...action.params }]),
    [
      ["settings", "DeviceLogin", { start_scanner: true }],
      ["settings", "AgentKeyLogin", { mint: true }],
    ],
  );
  x.h.unmount();
});

test("every Search group and See all heading shares the sidebar's 16pt semibold primary style in both themes", () => {
  for (const dark of [false, true]) {
    const x = setup(false, undefined, false, dark);
    const colors = dark ? darkColors : lightColors;
    for (const query of ["", "settings", "theme", "helper", "design"]) {
      x.type(query);
      for (const section of x.list().props.sections) {
        const heading = x.list().props.renderSectionHeader({ section });
        const style = Object.assign({}, ...heading.props.style);
        assert.equal(heading.props.accessibilityRole, "header");
        assert.equal(style.fontSize, sidebarSectionTitle.fontSize);
        assert.equal(style.fontFamily, sidebarSectionTitle.fontFamily);
        assert.equal(style.color, colors.textPrimary);
        assert.ok(style.paddingTop >= 20);
      }
    }
    x.h.unmount();
    const all = setup(true, undefined, false, dark);
    const heading = all.list().props.ListHeaderComponent;
    const style = Object.assign({}, ...heading.props.style);
    assert.equal(heading.props.children, "Recent chats");
    assert.equal(style.fontSize, sidebarSectionTitle.fontSize);
    assert.equal(style.fontFamily, sidebarSectionTitle.fontFamily);
    assert.equal(style.color, colors.textPrimary);
    all.h.unmount();
  }
});

test("Search shows no-result/offline states and stale account/focus selections are inert", () => {
  const x = setup();
  x.type("does not exist");
  assert.equal(x.list().props.sections.length, 0);
  assert.equal(x.list().props.ListEmptyComponent.props.children, "No results.");
  x.type("release");
  const stale = x.result("Release report");
  x.setEpoch();
  stale.props.onPress();
  assert.equal(x.actions.length, 0);
  x.h.render();
  x.blur();
  x.result("Release report").props.onPress();
  assert.equal(x.actions.length, 0);
  x.data.online = false;
  x.h.render();
  assert.ok(
    x.all().some((n) => n.props.children === "Offline · searching saved data."),
  );
  x.h.unmount();
});

test("See all searches, filters by agent, loads older pages and opens exact destinations", () => {
  const x = setup(true);
  assert.equal(x.list().props.data.length, 2);
  const helper = x
    .all()
    .find(
      (n) => n.props.accessibilityLabel === "Filter chats: Research helper",
    )!;
  helper.props.onPress();
  x.h.flush();
  assert.equal(x.scopes.at(-1), "helper");
  assert.equal(x.list().props.data.length, 1);
  x.type("release");
  const row = x.list().props.renderItem({ item: x.list().props.data[0] });
  row.props.onSelect(row.props.item.target);
  assert.equal(x.actions[0].target.conversationId, "nyxa-first");
  x.list().props.ListFooterComponent.props.onPress();
  assert.equal(x.counts().more, 1);
  x.changeRoute("nyxbot");
  assert.equal(x.scopes.at(-1), "nyxbot");
  assert.equal(x.field().props.value, "");
  assert.equal(x.list().props.data.length, 0);
  x.changeRoute();
  assert.equal(x.list().props.data.length, 2);
  x.h.unmount();
});

test("Chat History shares the paged browser and New chat retains its originating agent", () => {
  for (const agent of ["nyxbot", "helper"]) {
    const x = setup(true, agent, true);
    assert.equal(x.h.value().props.title, "Chat history");
    assert.equal(x.scopes.at(-1), agent);
    const compose = x
      .all()
      .find((n) => n.props.accessibilityLabel === "New chat")!;
    compose.props.onPress();
    assert.equal(x.actions[0].kind, "chat");
    assert.equal(x.actions[0].target.agentId, agent);
    assert.equal(x.actions[0].target.conversationId, undefined);
    x.h.unmount();
  }
});
