import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import {
  agentSchema,
  type AssistantAgent,
} from "../../features/agents/agentContracts";
import { conversationSchema } from "../../features/assistant/assistantTypes";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { groupSchema } from "../../features/groups/groupContracts";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { lightColors } from "../../theme/mobileTheme";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import * as listHelpers from "./workspaceChatListModel";

type Element = { type: unknown; props: Record<string, any> };
type QueryOptions = { queryKey: unknown[]; enabled: boolean };
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const conversationId = (value: number) =>
  `nyxa-${value.toString(16).padStart(32, "0")}`;

const agents = [
  agentSchema.parse({
    id: "nyxbot",
    name: "nyxbot",
    display_name: "NyxBot",
    kind: "nyxbot",
    description: "Personal assistant",
    status: "idle",
    can_use: true,
  }),
  ...Array.from({ length: 8 }, (_, index) =>
    agentSchema.parse({
      id: `agent-${index + 1}`,
      name: `helper-${index + 1}`,
      display_name: `Agent ${index + 1}`,
      kind: "specialist",
      description: "Assistant",
      status: "idle",
      can_use: index !== 7,
    }),
  ),
];
const groups = Array.from({ length: 8 }, (_, index) =>
  groupSchema.parse({
    id: `nyxg-${(index + 1).toString(16).padStart(32, "0")}`,
    name: `Group ${index + 1}`,
    members: [],
    lead_agent_id: "nyxbot",
    created_at: `2026-10-${String(index + 1).padStart(2, "0")}T00:00:00Z`,
    message_count: index + 1,
  }),
);
function chatsFor(agentId: string) {
  const owner = agents.find((agent) => agent.id === agentId);
  assert.ok(owner);
  const agentIndex = agents.indexOf(owner);
  return Array.from({ length: 8 }, (_, index) =>
    conversationSchema.parse({
      id: conversationId(agentIndex * 100 + index + 1),
      title: `${owner.display_name} chat ${index + 1}`,
      model: "nyxagent/chat",
      message_count: 2,
      active_turn: null,
      created_at: "2026-10-01T00:00:00Z",
      last_message_at: `2026-10-${String(16 - index).padStart(2, "0")}T00:00:00Z`,
      agent: {
        id: owner.id,
        name: owner.name,
        display_name: owner.display_name,
        kind: owner.kind,
      },
    }),
  );
}

/** Render the component's stateless rows while leaving its modal as a boundary. */
function elements(root: Element): Element[] {
  if (typeof root.type === "function")
    return elements((root.type as (props: unknown) => Element)(root.props));
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}
function content(root: Element): string {
  return [root.props.children]
    .flat(Infinity)
    .map((child) =>
      typeof child === "string" || typeof child === "number"
        ? String(child)
        : child && typeof child === "object" && "props" in child
          ? content(child)
          : "",
    )
    .join("");
}

type SetupOptions = {
  mode?: "drawer" | "picker";
  history?: "loaded" | "empty" | "pending" | "error";
  agent?: Partial<AssistantAgent>;
  online?: boolean;
  chats?: ReturnType<typeof chatsFor>;
};

function setup(current?: WorkspaceChatTarget, options: SetupOptions = {}) {
  const selections: WorkspaceChatTarget[] = [];
  const creations: string[] = [];
  const historyQueries: QueryOptions[] = [];
  const liveAgents = agents.map((agent) => ({ ...agent }));
  const targetAgentId = current?.kind === "agent" ? current.agentId : undefined;
  if (targetAgentId && options.agent) {
    Object.assign(
      liveAgents.find((agent) => agent.id === targetAgentId)!,
      options.agent,
    );
  }
  const threadOverrides = options.chats;
  const historyState = options.history ?? "loaded";
  const props = {
    mode: options.mode,
    visible: true,
    current,
    onCreateAgent: () => creations.push("agent"),
    onCreateGroup: () => creations.push("group"),
    onSelect: (target: WorkspaceChatTarget) => {
      selections.push(plain(target));
      props.current = target;
    },
  };
  const harness = hookHarness<Element>({
    "../AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ...Object.fromEntries(
        ["ActivityIndicator", "Pressable", "ScrollView", "Text", "View"].map(
          (name) => [name, name],
        ),
      ),
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove: () => {} }),
      },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": Object.fromEntries(
      [
        "Bot",
        "Check",
        "ChevronDown",
        "ChevronRight",
        "MessageSquare",
        "Users",
        "Plus",
      ].map((name) => [name, name]),
    ),
    "@tanstack/react-query": {
      useQuery: ({ queryKey }: QueryOptions) => ({
        data: queryKey.includes("agents") ? { agents: liveAgents } : groups,
        isSuccess: true,
        isPending: false,
        isError: false,
        refetch: async () => {},
      }),
      useInfiniteQuery: (options: QueryOptions) => {
        historyQueries.push(options);
        const agentId = options.queryKey.at(-1);
        return {
          data:
            typeof agentId === "string" &&
            historyState !== "pending" &&
            historyState !== "error"
              ? {
                  pages: [
                    {
                      conversations:
                        historyState === "empty"
                          ? []
                          : (threadOverrides ?? chatsFor(agentId)),
                      next_cursor: null,
                    },
                  ],
                }
              : undefined,
          isSuccess: historyState === "loaded" || historyState === "empty",
          isPending: historyState === "pending",
          isError: historyState === "error",
          hasNextPage: false,
          isFetchingNextPage: false,
          isFetchNextPageError: false,
          refetch: async () => {},
          fetchNextPage: async () => {},
        };
      },
    },
    "../../lib/time": { formatRelativeTimeFromMs },
    "../../features/groups/useGroupsLive": { useGroupsLive: () => false },
    "../../lib/api/assistantLiveTransport": {
      livePollInterval: (value: number) => value,
    },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: options.online !== false }),
    },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/assistantApi": { assistantApi: {} },
    "../../lib/api/groupsApi": { groupsApi: {} },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 17 },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../theme/designTokens": { fonts: {} },
    "./WorkspaceChatBrowser": { WorkspaceChatBrowser: "WorkspaceChatBrowser" },
    "./workspaceChatListModel": listHelpers,
  });
  type Props = typeof props;
  const { WorkspaceChatList } = harness.load(
    fileURLToPath(new URL("./WorkspaceChatList.tsx", import.meta.url)),
  ) as { WorkspaceChatList: (props: Props) => Element };
  harness.mount(() => WorkspaceChatList(props));
  const all = () => elements(harness.flush());
  const one = (predicate: (element: Element) => boolean) => {
    const found = all().find(predicate);
    assert.ok(found, "Expected matching drawer control");
    return found;
  };
  const label = (name: string) =>
    one(
      (row) =>
        row.props.accessibilityLabel === name ||
        row.props.accessibilityLabel?.startsWith(`${name}, `),
    );
  const press = (name: string) => {
    label(name).props.onPress();
    harness.flush();
  };
  const more = (name: string) => {
    one(
      (row) => row.type === "Pressable" && content(row) === name,
    ).props.onPress();
    harness.flush();
  };
  const browser = () => one((row) => row.type === "WorkspaceChatBrowser").props;
  const agentRows = () =>
    all().filter((row) =>
      /^(Expand|Collapse) chats for /.test(row.props.accessibilityLabel ?? ""),
    );
  const chatRows = () =>
    all().filter((row) =>
      /^(Open chat |Open current chat$)/.test(
        row.props.accessibilityLabel ?? "",
      ),
    );
  const groupRows = () =>
    all().filter((row) =>
      /^Open crew /.test(row.props.accessibilityLabel ?? ""),
    );
  const historyQuery = () => {
    const latest = historyQueries.at(-1);
    assert.ok(latest);
    return latest;
  };
  const pressBrowserItem = (id: string) => {
    const model = browser();
    const item = model.items.find((row: { id: string }) => row.id === id);
    assert.ok(item, `Expected modal item ${id}`);
    const control = elements(model.renderItem(item)).find(
      (row) => row.type === "Pressable",
    );
    assert.ok(control);
    control.props.onPress();
    harness.render();
    const closing = all().find(
      (row) =>
        row.type === "WorkspaceChatBrowser" && row.props.visible === false,
    );
    if (closing) {
      closing.props.onDismiss();
      harness.flush();
    }
  };
  return {
    harness,
    setCurrent: (value: WorkspaceChatTarget) => {
      props.current = value;
      harness.render();
    },
    selections,
    creations,
    all,
    label,
    press,
    more,
    browser,
    agentRows,
    chatRows,
    groupRows,
    historyQuery,
    pressBrowserItem,
    patchAgent: (id: string, patch: Partial<AssistantAgent>) => {
      const index = liveAgents.findIndex((row) => row.id === id);
      assert.ok(index >= 0);
      liveAgents[index] = { ...liveAgents[index]!, ...patch };
      harness.render();
    },
  };
}

test("agent and group creation buttons stay separate from section disclosure", () => {
  const env = setup();
  const expandedBefore = env.agentRows().length;
  env.press("New agent");
  env.press("New crew");
  assert.deepEqual(env.creations, ["agent", "group"]);
  assert.deepEqual(env.selections, []);
  assert.equal(env.agentRows().length, expandedBefore);
  assert.equal(env.groupRows().length, 0);
  env.harness.unmount();
});

test("offline drawer creation buttons are disabled while saved previews remain available", () => {
  const env = setup(undefined, { online: false });
  for (const label of ["New agent", "New crew"]) {
    const button = env.label(label);
    assert.equal(button.props.disabled, true);
    assert.equal(button.props.accessibilityState.disabled, true);
  }
  assert.equal(env.agentRows().length, 5);
  env.harness.unmount();
});

test("agent and group previews stay at five rows and their sections collapse independently", () => {
  const env = setup();
  assert.equal(env.agentRows().length, 5);
  assert.equal(
    env.agentRows().filter((row) => row.props.accessibilityState.expanded)
      .length,
    0,
  );
  assert.equal(env.historyQuery().enabled, false);
  assert.equal(env.groupRows().length, 0);
  env.press("Expand crews");
  assert.equal(env.groupRows().length, 5);
  assert.equal(env.agentRows().length, 5);
  env.press("Collapse agents");
  assert.equal(env.agentRows().length, 0);
  assert.equal(env.chatRows().length, 0);
  assert.equal(env.groupRows().length, 5);
  env.press("Collapse crews");
  assert.equal(env.groupRows().length, 0);
  env.press("Expand agents");
  assert.equal(env.agentRows().length, 5);
  env.harness.unmount();
});

test("opening another agent collapses the first and queries only the expanded history", () => {
  const env = setup();
  env.press("Expand chats for Agent 1");
  assert.equal(env.historyQuery().queryKey.at(-1), "agent-1");
  assert.equal(env.historyQuery().enabled, true);
  assert.equal(env.chatRows().length, 5);
  env.press("Expand chats for Agent 2");
  const expanded = env
    .agentRows()
    .filter((row) => row.props.accessibilityState.expanded);
  assert.equal(expanded.length, 1);
  assert.equal(
    expanded[0]?.props.accessibilityLabel,
    "Collapse chats for Agent 2, Personal",
  );
  assert.equal(env.historyQuery().queryKey.at(-1), "agent-2");
  assert.equal(env.chatRows().length, 5);
  assert.ok(
    env
      .chatRows()
      .every((row) => row.props.accessibilityLabel.endsWith("with Agent 2")),
  );
  env.press("Collapse agents");
  assert.equal(env.chatRows().length, 0);
  assert.equal(env.historyQuery().enabled, false);
  env.harness.unmount();
});

test("the all-agents browser reaches hidden agents and chats while both previews remain capped", () => {
  const env = setup();
  assert.equal(
    env
      .agentRows()
      .some((row) => row.props.accessibilityLabel.includes("Agent 7")),
    false,
  );
  env.more("View all agents (9)");
  assert.equal(env.browser().title, "All agents");
  assert.equal(env.browser().items.length, 9);
  env.pressBrowserItem("agent-7");
  assert.equal(env.browser().title, "Agent 7");
  assert.equal(env.browser().scopeKey, "chats:agent-7");
  assert.equal(env.browser().items.length, 8);
  assert.equal(env.agentRows().length, 5);
  assert.ok(
    env
      .agentRows()
      .some(
        (row) =>
          row.props.accessibilityLabel ===
          "Collapse chats for Agent 7, Personal",
      ),
  );
  assert.equal(env.chatRows().length, 5);
  const chosenChat = chatsFor("agent-7")[7]!;
  env.pressBrowserItem(chosenChat.id);
  assert.deepEqual(env.selections, [
    { kind: "agent", agentId: "agent-7", conversationId: chosenChat.id },
  ]);
  assert.equal(
    env.all().some((row) => row.type === "WorkspaceChatBrowser"),
    false,
  );
  assert.equal(env.agentRows().length, 5);
  assert.equal(env.chatRows().length, 5);
  const selected = env
    .chatRows()
    .filter((row) => row.props.accessibilityState.selected);
  assert.equal(selected.length, 1);
  assert.equal(
    selected[0]?.props.accessibilityLabel,
    "Open chat Agent 7 chat 8 with Agent 7, current thread",
  );
  env.harness.unmount();
});

test("a current group outside the first five is pinned and the all-groups browser exposes every group", () => {
  const chosen = groups[0]!;
  const env = setup({ kind: "group", groupId: chosen.id });
  assert.equal(env.groupRows().length, 5);
  assert.ok(
    env
      .groupRows()
      .some(
        (row) =>
          row.props.accessibilityLabel === "Open crew Group 1, current crew" &&
          row.props.accessibilityState.selected,
      ),
  );
  env.more("View all crews (8)");
  assert.equal(env.browser().items.length, 8);
  const hidden = groups[1]!;
  env.pressBrowserItem(hidden.id);
  assert.deepEqual(env.selections, [{ kind: "group", groupId: hidden.id }]);
  assert.equal(env.groupRows().length, 5);
  assert.ok(
    env
      .groupRows()
      .some(
        (row) =>
          row.props.accessibilityLabel === "Open crew Group 2, current crew" &&
          row.props.accessibilityState.selected,
      ),
  );
  env.harness.unmount();
});

test("a missing current chat consumes one of the five preview slots and retains its exact identity", () => {
  const missing = conversationId(9999);
  const env = setup({
    kind: "agent",
    agentId: "agent-1",
    conversationId: missing,
  });
  assert.equal(env.chatRows().length, 5);
  assert.equal(
    env
      .chatRows()
      .filter((row) => row.props.accessibilityLabel === "Open current chat")
      .length,
    1,
  );
  assert.equal(
    env
      .chatRows()
      .filter((row) => row.props.accessibilityLabel.startsWith("Open chat "))
      .length,
    4,
  );
  env.press("Open current chat");
  assert.deepEqual(env.selections, [
    { kind: "agent", agentId: "agent-1", conversationId: missing },
  ]);
  env.harness.unmount();
});

test("resolving an initially unnamed current chat does not render a duplicate standalone row", () => {
  const current = chatsFor("nyxbot")[0]!;
  const env = setup({ kind: "agent", conversationId: current.id });
  assert.equal(env.chatRows().length, 1);
  env.press("Expand chats for NyxBot");
  assert.equal(env.chatRows().length, 5);
  assert.equal(
    env
      .chatRows()
      .filter((row) => row.props.accessibilityLabel === "Open current chat")
      .length,
    0,
  );
  assert.equal(
    env.chatRows().filter((row) => row.props.accessibilityState.selected)
      .length,
    1,
  );
  env.harness.unmount();
});

test("a view-only agent keeps history querying disabled and offers no chat action", () => {
  const env = setup({ kind: "agent", agentId: "agent-8" });
  assert.equal(env.agentRows().length, 5);
  assert.equal(env.historyQuery().queryKey.at(-1), "agent-8");
  assert.equal(env.historyQuery().enabled, false);
  assert.equal(env.chatRows().length, 0);
  assert.ok(
    env
      .all()
      .some(
        (row) =>
          row.type === "Text" &&
          content(row).includes("chat access isn’t available"),
      ),
  );
  assert.equal(
    env
      .all()
      .some((row) => row.props.accessibilityLabel === "New chat with Agent 8"),
    false,
  );
  assert.deepEqual(env.selections, []);
  env.harness.unmount();
});

test("usable agents can start a fresh chat regardless of saved-history state", () => {
  for (const history of ["loaded", "empty", "pending", "error"] as const) {
    const env = setup({ kind: "agent", agentId: "agent-1" }, { history });
    env.press("New chat with Agent 1");
    assert.deepEqual(
      env.selections,
      [{ kind: "agent", agentId: "agent-1" }],
      `new conversation must not depend on ${history} history`,
    );
    env.harness.unmount();
  }
});

test("the agent-history browser keeps New chat separate from its saved history", () => {
  for (const history of ["loaded", "empty", "pending", "error"] as const) {
    const env = setup(undefined, { history });
    env.more("View all agents (9)");
    env.pressBrowserItem("agent-7");
    const primary = env.browser().primaryAction;
    assert.ok(primary, `Missing primary action for ${history} history`);
    assert.equal(primary.label, "New chat");
    assert.equal(primary.accessibilityLabel, "New chat with Agent 7");
    primary.onPress();
    env.harness.flush();
    assert.deepEqual(env.selections, []);
    assert.equal(env.browser().visible, false);
    env.browser().onDismiss();
    env.harness.flush();
    assert.deepEqual(env.selections, [{ kind: "agent", agentId: "agent-7" }]);
    assert.equal(
      env.all().some((row) => row.type === "WorkspaceChatBrowser"),
      false,
      "starting the chat dismisses the history browser",
    );
    env.harness.unmount();
  }
});

test("past and view-only agents explain chat unavailability and offer another agent", () => {
  for (const unavailable of [
    { status: "destroyed" as const },
    { can_use: false },
  ]) {
    const env = setup(
      { kind: "agent", agentId: "agent-1" },
      { agent: unavailable, history: "empty" },
    );
    env.more("View all agents (9)");
    env.pressBrowserItem("agent-1");
    const model = env.browser();
    assert.match(
      `${model.description} ${model.emptyText}`,
      /deleted|destroyed|past agent|read.only|not available|isn.t available|access/i,
    );
    assert.equal(model.primaryAction.label, "Choose another agent");
    assert.equal(
      model.primaryAction.accessibilityLabel,
      "Choose another agent",
    );
    model.primaryAction.onPress();
    env.harness.flush();
    assert.equal(env.browser().title, "All agents");
    assert.deepEqual(env.selections, []);
    env.harness.unmount();
  }
});

test("a destroyed agent retains readable saved conversations without offering a new one", () => {
  const env = setup(
    { kind: "agent", agentId: "agent-1" },
    { agent: { status: "destroyed" } },
  );
  assert.equal(env.historyQuery().enabled, true);
  assert.equal(
    env
      .all()
      .some((row) => row.props.accessibilityLabel === "New chat with Agent 1"),
    false,
  );
  env.more("View all agents (9)");
  env.pressBrowserItem("agent-1");
  assert.equal(env.browser().primaryAction.label, "Choose another agent");
  const saved = chatsFor("agent-1")[0]!;
  env.pressBrowserItem(saved.id);
  assert.deepEqual(env.selections, [
    { kind: "agent", agentId: "agent-1", conversationId: saved.id },
  ]);
  env.harness.unmount();
});

test("stale new-chat callbacks cannot bypass an updated agent status or access grant", () => {
  for (const unavailable of [
    { status: "destroyed" as const },
    { can_use: false },
  ]) {
    const env = setup();
    env.more("View all agents (9)");
    env.pressBrowserItem("agent-7");
    const start = env.browser().primaryAction.onPress;
    env.patchAgent("agent-7", unavailable);
    start();
    env.harness.flush();
    assert.deepEqual(env.selections, []);
    assert.equal(env.browser().primaryAction.label, "Choose another agent");
    env.harness.unmount();
  }
});

test("the child browser completes dismissal before asking the parent to open a chat", () => {
  const env = setup();
  env.more("View all agents (9)");
  env.pressBrowserItem("agent-7");
  const start = env.browser().primaryAction.onPress;
  start();
  env.harness.flush();
  assert.equal(env.browser().visible, false);
  assert.deepEqual(env.selections, []);
  const dismiss = env.browser().onDismiss;
  start();
  env.harness.flush();
  dismiss();
  env.harness.flush();
  dismiss();
  env.harness.flush();
  assert.deepEqual(env.selections, [{ kind: "agent", agentId: "agent-7" }]);
  assert.equal(
    env.all().some((row) => row.type === "WorkspaceChatBrowser"),
    false,
  );
  env.harness.unmount();
});

test("revoking an agent while its browser closes cancels the queued new conversation", () => {
  for (const unavailable of [
    { status: "destroyed" as const },
    { can_use: false },
  ]) {
    const env = setup();
    env.more("View all agents (9)");
    env.pressBrowserItem("agent-7");
    env.browser().primaryAction.onPress();
    env.harness.flush();
    assert.equal(env.browser().visible, false);
    env.patchAgent("agent-7", unavailable);
    env.browser().onDismiss();
    env.harness.flush();
    assert.deepEqual(env.selections, []);
    env.harness.unmount();
  }
});

test("closing the browser without selecting keeps the parent menu and current chat unchanged", () => {
  const current = {
    kind: "agent" as const,
    agentId: "agent-1",
    conversationId: chatsFor("agent-1")[0]!.id,
  };
  const env = setup(current);
  env.more("View all agents (9)");
  env.pressBrowserItem("agent-7");
  env.browser().onClose();
  env.harness.flush();
  assert.equal(env.browser().visible, false);
  assert.deepEqual(env.selections, []);
  const dismiss = env.browser().onDismiss;
  dismiss();
  env.harness.flush();
  dismiss();
  env.harness.flush();
  assert.deepEqual(env.selections, []);
  assert.equal(
    env.all().some((row) => row.type === "WorkspaceChatBrowser"),
    false,
  );
  env.harness.unmount();
});

test("an earlier dismissal callback cannot replay a later browser selection", () => {
  const env = setup();
  env.more("View all agents (9)");
  env.pressBrowserItem("agent-7");
  env.browser().primaryAction.onPress();
  env.harness.flush();
  const firstDismissal = env.browser().onDismiss;
  firstDismissal();
  env.harness.flush();
  assert.deepEqual(env.selections, [{ kind: "agent", agentId: "agent-7" }]);
  env.more("View all agents (9)");
  env.pressBrowserItem("agent-1");
  env.browser().primaryAction.onPress();
  env.harness.flush();
  assert.equal(env.browser().visible, false);
  firstDismissal();
  env.harness.flush();
  assert.equal(env.selections.length, 1);
  assert.equal(env.browser().visible, false);
  env.browser().onDismiss();
  env.harness.flush();
  assert.deepEqual(env.selections, [
    { kind: "agent", agentId: "agent-7" },
    { kind: "agent", agentId: "agent-1" },
  ]);
  env.harness.unmount();
});

test("group selection also waits for the child browser dismissal before opening the parent chat", () => {
  const env = setup();
  env.press("Expand crews");
  env.more("View all crews (8)");
  const model = env.browser();
  const selected = model.items[0];
  const control = elements(model.renderItem(selected)).find(
    (row) => row.type === "Pressable",
  );
  assert.ok(control);
  control.props.onPress();
  env.harness.flush();
  assert.equal(env.browser().visible, false);
  assert.deepEqual(env.selections, []);
  env.browser().onDismiss();
  env.harness.flush();
  assert.deepEqual(env.selections, [{ kind: "group", groupId: selected.id }]);
  env.harness.unmount();
});

test("a saved conversation remains navigable if its agent is deleted while the browser closes", () => {
  const env = setup();
  env.more("View all agents (9)");
  env.pressBrowserItem("agent-7");
  const model = env.browser();
  const selected = model.items[0];
  const control = elements(model.renderItem(selected)).find(
    (row) => row.type === "Pressable",
  );
  assert.ok(control);
  control.props.onPress();
  env.harness.flush();
  env.patchAgent("agent-7", { status: "destroyed" });
  assert.equal(env.browser().visible, false);
  assert.deepEqual(env.selections, []);
  env.browser().onDismiss();
  env.harness.flush();
  assert.deepEqual(env.selections, [
    { kind: "agent", agentId: "agent-7", conversationId: selected.id },
  ]);
  env.harness.unmount();
});

for (const mode of ["drawer", "picker"] as const)
  test(`${mode}: channel bots form counted sub-thread groups, keep the current child visible, and expand more in place`, () => {
    const source = chatsFor("agent-1");
    const channels = source.map((thread, index) =>
      conversationSchema.parse({
        ...thread,
        channel: {
          platform: "telegram",
          channel_agent_id: "bot-1",
          bot_label: "Support bot",
          chat_id: `chat-${index}`,
          chat_title: `Customer ${index + 1}`,
          parent_chat_id: "support-room",
          parent_title: "Support room",
          thread_id: `topic-${index}`,
        },
        pending_acknowledgements: index === 7 ? 1 : 0,
      }),
    );
    const current = {
      kind: "agent" as const,
      agentId: "agent-1",
      conversationId: channels[7]!.id,
    };
    const env = setup(current, { chats: channels, mode });
    const bot = env.label("Collapse chats for Support bot");
    assert.equal(bot.props.accessibilityState.expanded, true);
    assert.match(
      bot.props.accessibilityLabel,
      /Telegram · 8 chats · Needs you/,
    );
    assert.equal(
      env.chatRows().length,
      6,
      "the current channel child stays visible beyond the first five",
    );
    const selected = env.label(
      "Open chat Customer 8 with Agent 1, Needs you, current thread",
    );
    assert.equal(selected.props.accessibilityState.selected, true);
    assert.ok(content(env.harness.flush()).includes("Support room"));
    assert.ok(
      env
        .all()
        .some(
          (row) => row.type === "Text" && content(row).includes("Needs you"),
        ),
    );
    env.more("Show 2 more");
    assert.equal(env.chatRows().length, 8);
    const rows = env.all();
    assert.ok(
      rows.findIndex(
        (row) => row.props.accessibilityLabel === "New chat with Agent 1",
      ) >
        rows
          .map((row) =>
            Boolean(row.props.accessibilityLabel?.startsWith("Open chat ")),
          )
          .lastIndexOf(true),
    );
    env.press("Collapse chats for Support bot");
    assert.equal(env.chatRows().length, 0);
    env.setCurrent({ ...current, conversationId: channels[6]!.id });
    assert.equal(
      env.label("Collapse chats for Support bot").props.accessibilityState
        .expanded,
      true,
      "opening another child unfolds its bot again",
    );
    env.press("Open chat Customer 7 with Agent 1, current thread");
    assert.deepEqual(env.selections, [
      { ...current, conversationId: channels[6]!.id },
    ]);
    env.harness.unmount();
  });

test("thread rows show newest first, relative times, running and needs-you states, and fold with their agent", () => {
  const source = chatsFor("agent-1");
  source[0] = conversationSchema.parse({
    ...source[0],
    active_turn: { turn_id: "turn", started_at: "2026-10-16T00:00:00Z" },
  });
  source[1] = conversationSchema.parse({
    ...source[1],
    pending_acknowledgements: 1,
  });
  const env = setup(
    { kind: "agent", agentId: "agent-1", conversationId: source[0]!.id },
    { chats: source },
  );
  const rows = env.chatRows();
  assert.match(
    rows[0]!.props.accessibilityLabel,
    /chat 1.*Running.*current thread/,
  );
  assert.match(rows[1]!.props.accessibilityLabel, /chat 2.*Needs you/);
  assert.ok(
    content(rows[0]!).includes(
      formatRelativeTimeFromMs(Date.parse(source[0]!.last_message_at)),
    ),
  );
  assert.ok(content(rows[0]!).includes("Current · Running"));
  assert.equal(rows[0]!.props.style({ pressed: false })[0].minHeight, 44);
  env.press("Collapse chats for Agent 1");
  assert.equal(env.chatRows().length, 0);
  assert.equal(env.historyQuery().enabled, false);
  env.press("Expand chats for Agent 1");
  assert.equal(env.chatRows().length, 5);
  env.harness.unmount();
});

test("expanded agent is a native sticky header only while its threads remain in the scroll viewport", () => {
  const env = setup({ kind: "agent", agentId: "agent-1" });
  const scroll = () => env.all().find((row) => row.type === "ScrollView")!;
  const index = scroll().props.stickyHeaderIndices[0];
  assert.equal(typeof index, "number");
  const children = scroll().props.children.flat(Infinity);
  children[index].props.onLayout({ nativeEvent: { layout: { height: 48 } } });
  children[index + 1].props.onLayout({
    nativeEvent: { layout: { y: 200, height: 400 } },
  });
  scroll().props.onScroll({ nativeEvent: { contentOffset: { y: 300 } } });
  env.harness.flush();
  assert.deepEqual(plain(scroll().props.stickyHeaderIndices), [index]);
  scroll().props.onScroll({ nativeEvent: { contentOffset: { y: 555 } } });
  env.harness.flush();
  assert.deepEqual(plain(scroll().props.stickyHeaderIndices), []);
  scroll().props.onScroll({ nativeEvent: { contentOffset: { y: 300 } } });
  env.harness.flush();
  assert.deepEqual(plain(scroll().props.stickyHeaderIndices), [index]);
  env.press("Collapse chats for Agent 1");
  assert.deepEqual(plain(scroll().props.stickyHeaderIndices), []);
  env.harness.unmount();
});

test("agent picker reuses the thread hierarchy without crew or creation sections", () => {
  const env = setup(
    { kind: "agent", agentId: "agent-1", conversationId: conversationId(101) },
    { mode: "picker" },
  );
  assert.equal(env.groupRows().length, 0);
  assert.equal(
    env.all().some((row) => row.props.accessibilityLabel === "New crew"),
    false,
  );
  assert.ok(
    env.chatRows().some((row) => row.props.accessibilityState?.selected),
  );
  assert.ok(
    env
      .all()
      .some(
        (row) => row.type === "AgentAvatar" && row.props.agent.id === "agent-1",
      ),
  );
  assert.ok(
    env.label("Collapse chats for Agent 1").props.accessibilityState.selected,
  );
  env.press("Expand chats for Agent 2");
  assert.equal(env.selections.length, 0);
  assert.equal(env.historyQuery().queryKey.at(-1), "agent-2");
  assert.ok(env.chatRows().length > 0);
  env.press("Collapse chats for Agent 2");
  assert.equal(env.chatRows().length, 0);
  env.press("Expand chats for Agent 2");
  env.press("New chat with Agent 2");
  assert.deepEqual(env.selections.at(-1), {
    kind: "agent",
    agentId: "agent-2",
  });
  env.harness.unmount();
});

test("sidebar owner sections and real channel badges keep personal agents before organizations", () => {
  const env = setup(
    { kind: "agent", agentId: "agent-7" },
    {
      agent: {
        owner_kind: "org",
        owner_id: "team",
        owner_name: "Product team",
        channels: [
          {
            id: "bot",
            platform: "telegram",
            bot_label: "Release bot",
            status: "active",
          },
        ],
      },
    },
  );
  const texts = env
    .all()
    .filter((row) => row.type === "Text")
    .map(content);
  assert.ok(texts.indexOf("Personal") < texts.indexOf("Product team"));
  assert.ok(texts.includes("Telegram"));
  assert.ok(
    env
      .label("Collapse chats for Agent 7")
      .props.accessibilityLabel.includes("Product team"),
  );
  env.harness.unmount();
});

test("grouped sidebar agent subtitles keep persona/channel/status without repeating the owner", () => {
  const env = setup(
    { kind: "agent", agentId: "agent-7" },
    {
      agent: {
        owner_kind: "org",
        owner_id: "team",
        owner_name: "Product team",
        description: "Keeps releases moving",
        pending_acknowledgements: 1,
        channels: [
          {
            id: "bot",
            platform: "telegram",
            bot_label: "Release bot",
            status: "active",
          },
        ],
      },
    },
  );
  const texts = elements(env.label("Collapse chats for Agent 7"))
    .filter((node) => node.type === "Text")
    .map(content);
  assert.ok(texts.includes("Needs you · Keeps releases moving"));
  assert.ok(texts.includes("Telegram"));
  assert.ok(
    texts.every(
      (text) => !text.includes("Product team") && !text.includes("Personal"),
    ),
  );
  const nyxbot = elements(env.label("Expand chats for NyxBot"))
    .filter((node) => node.type === "Text")
    .map(content);
  assert.ok(nyxbot.includes("Your personal agent"));
  assert.ok(!nyxbot.includes("Personal · Your personal agent"));
  for (const title of ["Agents", "Crews"]) {
    const text = env
      .all()
      .find((node) => node.type === "Text" && node.props.children === title)!;
    const style = Object.assign({}, ...text.props.style);
    assert.equal(style.color, lightColors.textPrimary);
    assert.equal(style.fontSize, sidebarSectionTitle.fontSize);
    assert.equal(style.fontFamily, sidebarSectionTitle.fontFamily);
  }
  env.harness.unmount();
});
