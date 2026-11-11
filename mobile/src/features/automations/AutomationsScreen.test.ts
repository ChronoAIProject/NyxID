import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as presentation from "./automationPresentation";
import * as time from "./automationTime";
import { automationSchema } from "./automationContracts";
import { lightColors } from "../../theme/mobileTheme";

const id = "12345678-1234-4123-8123-123456789abc";
const runId = "22345678-1234-4123-8123-123456789abc";
const at = "2026-10-27T01:00:00Z";
const run = {
  id: runId,
  scheduled_at: at,
  started_at: at,
  finished_at: at,
  outcome: "completed",
  reason: null,
  thread_id: "nyxa-thread",
  duration_ms: 450,
};
const trigger = automationSchema.parse({
  id,
  user_id: "owner",
  label: "Morning release check",
  user_service_id: null,
  status: "active",
  source: "schedule",
  schedule: { kind: "cron", expression: "0 8 * * 1-5", timezone: "UTC" },
  verification: { mode: "schedule" },
  delivery: {
    type: "assistant",
    agent_id: "researcher",
    instruction: "Review the release checklist.",
    thread_policy: "dedicated",
    confirmation_policy: "destructive",
    deliver_to: { type: "thread" },
  },
  last_run: run,
  next_run_at: at,
  next_runs: [at],
  inbound_url: null,
  created_at: at,
  updated_at: at,
});
type Element = { type: any; props: Record<string, any> };
const ROOT_STATE = { key: "root" };
/** Walk the tree, rendering the screen's own hook-free helper components. */
function elements(root: Element): Element[] {
  if (!root || typeof root !== "object" || !root.props) return [];
  const rendered =
    typeof root.type === "function" ? elements(root.type(root.props)) : [];
  return [
    root,
    ...rendered,
    ...[root.props.children].flat(Infinity).flatMap(elements),
  ];
}
type Page =
  | "AutomationsScreen"
  | "AutomationAgentFilterScreen"
  | "AutomationDetailScreen"
  | "AutomationRunsScreen"
  | "AutomationRunDetailScreen";
function setup(
  page: Page,
  {
    zone = "UTC",
    deviceZone = "UTC",
    params = { id, runId } as Record<string, unknown>,
  }: {
    zone?: string;
    deviceZone?: string;
    params?: Record<string, unknown>;
  } = {},
) {
  const routes: any[] = [];
  const menus: any[] = [];
  const drafts = new Map<string, string>();
  const dispatched: any[] = [];
  const pops: any[] = [];
  const chats: any[] = [];
  const queries: any[] = [];
  let nextPages = 0;
  let refetches = 0;
  const cursor = { before: at, before_id: id };
  const response = (data: unknown) => ({
    data,
    isPending: false,
    isError: false,
    refetch: async () => {
      refetches++;
    },
  });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      Pressable: "Pressable",
      StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 1 },
      TextInput: "TextInput",
      View: "View",
    },
    "lucide-react-native": new Proxy({}, { get: (_target, name) => name }),
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../components/DynamicTypeText": { DynamicTypeText: "Text" },
    "../../components/AgentAvatar": { AgentAvatar: "AgentAvatar" },
    "../../components/Skeleton": { SkeletonCards: "SkeletonCards" },
    "../../components/settings/SettingsList": {
      ...Object.fromEntries(
        [
          "SettingsCheckRow",
          "SettingsGroup",
          "SettingsPage",
          "SettingsRow",
        ].map((name) => [name, name]),
      ),
      usePopupMenu: (options: any) => {
        menus.push(options);
        return { anchor: { current: null }, open: () => {}, popup: null };
      },
    },
    "../../app/WorkspaceHeader": {
      useOpenWorkspaceChat: () => (params: any) => chats.push(params),
      useWorkspaceSheet: () => null,
    },
    "../../app/workspaceChatNavigation": {
      workspaceChatNavigationAction: (state: unknown, target: any) =>
        state === ROOT_STATE ? { type: "RESET_TO_CHAT", target } : null,
    },
    "../assistant/assistantDraftStore": {
      assistantDraftKey: (agentId?: string, _c?: string, draftId?: string) =>
        JSON.stringify(["private", agentId ?? "nyxbot", draftId ?? "new"]),
      assistantDraftStore: {
        set: (key: string, value: string) => drafts.set(key, value),
      },
    },
    "../../components/workspace/WorkspaceUI": { Notice: "Notice" },
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/automationsApi": { automationsApi: {} },
    "./automationPresentation": presentation,
    "./automationTime": { ...time, deviceTimezone: () => deviceZone },
    "./AutomationUI": {
      AUTOMATIONS: ["workspace", "automations"],
      AUTOMATION_SETTINGS: ["workspace", "automations", "settings"],
      AutomationLoading: "AutomationLoading",
      automationError: (error: any) => String(error),
      useAutomationRuntime: () => ({ active: true, online: true }),
      usePullRefresh: (refetch: () => Promise<unknown>) => ({
        refreshing: false,
        onRefresh: () => void refetch(),
      }),
    },
    "@tanstack/react-query": {
      useQuery: (options: any) => {
        queries.push(options);
        return response(
          options.queryKey.includes("agents")
            ? {
                agents: [
                  {
                    id: "researcher",
                    name: "researcher",
                    display_name: "Researcher",
                  },
                  { id: "gone", name: "gone", status: "destroyed" },
                ],
              }
            : options.queryKey.includes("list")
              ? {
                  triggers: [
                    trigger,
                    {
                      ...trigger,
                      id: "other",
                      label: "Webhook reminder",
                      status: "disabled",
                      delivery: { type: "notification" },
                    },
                  ],
                }
              : options.queryKey.includes("settings")
                ? { timezone: zone }
                : trigger,
        );
      },
      useInfiniteQuery: (options: any) => {
        queries.push(options);
        return {
          ...response({ pages: [{ runs: [run], next_cursor: cursor }] }),
          hasNextPage: true,
          isFetchingNextPage: false,
          fetchNextPage: async () => {
            nextPages++;
          },
        };
      },
    },
  });
  const source = harness.load(
    fileURLToPath(new URL("./AutomationsScreen.tsx", import.meta.url)),
  );
  harness.mount(() =>
    (source[page] as any)({
      route: { params },
      navigation: {
        // Copy into this realm so deepEqual compares values, not prototypes.
        push: (name: any, params: any) =>
          routes.push({ name, params: { ...params } }),
        popTo: (name: any, params: any, options: any) =>
          pops.push([name, { ...params }, { ...options }]),
        getParent: () => ({ getState: () => ROOT_STATE }),
        dispatch: (action: any) => dispatched.push(action),
      },
    }),
  );
  const all = () => elements(harness.value());
  const rows = () =>
    all().filter((item) =>
      ["SettingsRow", "SettingsCheckRow"].includes(item.type),
    );
  const control = (label: string) => {
    const item = all().find(
      (item) =>
        item.props.label === label || item.props.accessibilityLabel === label,
    );
    assert.ok(item, `Missing ${label}`);
    return item;
  };
  const text = () =>
    all()
      .flatMap((item) => [
        item.props.label,
        item.props.detail,
        item.props.value,
        item.props.message,
        item.props.footer,
        ...(item.type === "Text" ? [item.props.children].flat(Infinity) : []),
      ])
      .filter((value) => typeof value === "string")
      .join("\n");
  return {
    harness,
    all,
    rows,
    control,
    text,
    routes,
    pops,
    chats,
    menus,
    drafts,
    dispatched,
    queries,
    cursor,
    nextPages: () => nextPages,
    refetches: () => refetches,
  };
}

/** An automation card, found by the label it starts its spoken summary with. */
function card(env: ReturnType<typeof setup>, label: string) {
  return env
    .all()
    .find(
      (item) =>
        item.type === "Pressable" &&
        String(item.props.accessibilityLabel ?? "").startsWith(`${label},`),
    );
}

test("the list is cards with a status menu, an agent chip and search, opening details read-only", () => {
  const env = setup("AutomationsScreen", { params: {} });
  try {
    const row = card(env, "Morning release check")!;
    assert.ok(row);
    assert.match(row.props.accessibilityLabel, /^Morning release check, Active\./);
    assert.match(row.props.accessibilityLabel, /Researcher · Scheduled/);
    row.props.onPress();
    assert.deepEqual(env.routes[0], {
      name: "AutomationDetail",
      params: { id },
    });
    // Status is one chip opening the shared in-app menu, not a segmented bar.
    assert.ok(env.all().every((item) => item.type !== "SegmentControl"));
    const status = env.menus.at(-1);
    assert.deepEqual(
      Array.from(status.options, (option: any) => option.label),
      ["All", "Active", "Paused"],
    );
    status.onChange("disabled");
    env.harness.flush();
    assert.match(card(env, "Webhook reminder")!.props.accessibilityLabel, /Paused/);
    assert.equal(card(env, "Morning release check"), undefined);
    assert.equal(env.control("Status, Paused").props.active, true);
    env.menus.at(-1).onChange("all");
    env.harness.flush();
    const agent = env.control("Agent, All agents");
    agent.props.onPress();
    assert.deepEqual(env.routes[1], {
      name: "AutomationAgentFilter",
      params: { agentId: undefined },
    });
    env.control("Search automations").props.onChangeText("not found");
    env.harness.flush();
    assert.match(env.text(), /No matching automations/);
    for (const label of [
      "Create automation",
      "Automation preferences",
      "Run now",
    ])
      assert.throws(() => env.control(label));
    assert.ok(env.all().every((item) => item.type !== "Card"));
  } finally {
    env.harness.unmount();
  }
});

test("an agent passed in the route filters the list and names the agent chip", () => {
  const env = setup("AutomationsScreen", {
    params: { agentId: "researcher" },
  });
  try {
    const agent = env.control("Agent, Researcher");
    assert.ok(card(env, "Morning release check"));
    assert.equal(card(env, "Webhook reminder"), undefined);
    agent.props.onPress();
    assert.deepEqual(env.routes[0].params, { agentId: "researcher" });
  } finally {
    env.harness.unmount();
  }
});

test("the agent filter is a checkmark list that returns its choice to the list", () => {
  const env = setup("AutomationAgentFilterScreen", {
    params: { agentId: "researcher" },
  });
  try {
    const choices = env.rows();
    assert.deepEqual(
      choices.map((row) => [row.props.label, !!row.props.selected]),
      [
        ["All agents", false],
        ["Researcher", true],
      ],
    );
    choices[1]!.props.onPress();
    choices[0]!.props.onPress();
    assert.deepEqual(env.pops, [
      ["Automations", { agentId: "researcher" }, { merge: true }],
      ["Automations", { agentId: undefined }, { merge: true }],
    ]);
  } finally {
    env.harness.unmount();
  }
});

test("details are label/value groups with the task, target and confirmation policy, and navigation rows", () => {
  const env = setup("AutomationDetailScreen");
  try {
    assert.equal(env.control("Status").props.value, "Active");
    assert.equal(env.control("Runs for").props.value, "Researcher");
    assert.equal(env.control("Thread").props.value, "dedicated");
    assert.equal(
      env.control("Confirmation").props.value,
      "Destructive actions",
    );
    assert.equal(
      env.control("Review the release checklist.").props.selectable,
      true,
    );
    assert.equal(env.control("Calendar expression").props.value, "0 8 * * 1-5");
    assert.equal(env.control("Latest run").props.value, "Completed");
    env.control("Run history").props.onPress();
    assert.deepEqual(env.routes[0], { name: "AutomationRuns", params: { id } });
    env.control("Agent permissions").props.onPress();
    assert.deepEqual(env.routes[1], {
      name: "AgentDetail",
      params: { agentId: "researcher" },
    });
    for (const label of [
      "Edit automation",
      "Automation actions",
      "Run now",
      "Pause automation",
      "Delete automation",
    ])
      assert.throws(() => env.control(label));
  } finally {
    env.harness.unmount();
  }
});

test("automation times read in the phone's local time, never an account time zone", () => {
  const deviceZone = "Asia/Singapore";
  for (const page of [
    "AutomationsScreen",
    "AutomationDetailScreen",
    "AutomationRunsScreen",
    "AutomationRunDetailScreen",
  ] as const) {
    // An account zone elsewhere must not change what the phone shows.
    const env = setup(page, { zone: "America/Los_Angeles", deviceZone });
    const text = env.text();
    assert.equal(text.includes("Times shown in"), false, page);
    assert.equal(text.includes("America/Los_Angeles"), false, page);
    assert.ok(text.includes(time.formatAutomationTime(at, deviceZone)), page);
    if (page === "AutomationsScreen")
      assert.ok(text.includes(`Next ${time.formatAutomationTime(at, deviceZone)}`));
    // 8:00 AM UTC on weekdays is 4:00 PM the same days in Singapore.
    if (page === "AutomationsScreen" || page === "AutomationDetailScreen")
      assert.match(text, /Weekdays at 4:00 PM/);
    if (page === "AutomationDetailScreen")
      assert.match(text, /Shown in your local time\. This automation follows UTC time/);
    env.harness.unmount();
  }
});

test("Ask NyxBot opens a new main NyxBot chat whose composer holds a one-blank setup message", () => {
  const env = setup("AutomationsScreen", { params: {} });
  try {
    const ask = env.control("Ask NyxBot");
    assert.equal(ask.type, "Pressable");
    ask.props.onPress();
    ask.props.onPress();
    // Not a chat pushed over this page: the root chat reset the drawer uses.
    assert.equal(env.chats.length, 0);
    assert.equal(env.dispatched.length, 2);
    const params = env.dispatched.map((action) => action.target.params);
    for (const [index, action] of env.dispatched.entries()) {
      assert.equal(action.type, "RESET_TO_CHAT");
      assert.equal(action.target.kind, "new");
      const chat = params[index];
      assert.equal(chat.home, false);
      assert.equal(chat.draft, true);
      assert.equal(chat.agentId, undefined);
      assert.equal(chat.conversationId, undefined);
      assert.equal(
        env.drafts.get(`text:${JSON.stringify(["private", "nyxbot", chat.draftId])}`),
        "Hi NyxBot, I'd like help setting up an automation that runs every ",
      );
    }
    assert.notEqual(params[0].draftId, params[1].draftId);
  } finally {
    env.harness.unmount();
  }
});

test("Ask NyxBot on a detail page names the automation in its starter message", () => {
  const env = setup("AutomationDetailScreen");
  try {
    const guidance = env
      .all()
      .find((item) => item.type?.name === "AutomationGuidance")!;
    const group = guidance.type(guidance.props);
    const ask = [group.props.children]
      .flat()
      .find((child: any) => child?.props?.label === "Ask NyxBot");
    ask.props.onPress();
    const chat = env.dispatched[0].target.params;
    assert.equal(
      env.drafts.get(`text:${JSON.stringify(["private", "nyxbot", chat.draftId])}`),
      'Hi NyxBot, I\'d like to change my automation "Morning release check": ',
    );
  } finally {
    env.harness.unmount();
  }
});

test("run history rows preserve cursor paging and open the exact run", () => {
  const env = setup("AutomationRunsScreen");
  try {
    const row = env
      .rows()
      .find(
        (item) => item.props.label === time.formatAutomationTime(at, "UTC"),
      )!;
    assert.equal(row.props.value, "Completed");
    assert.match(row.props.detail, /0.5 seconds/);
    row.props.onPress();
    assert.deepEqual(env.routes[0], {
      name: "AutomationRunDetail",
      params: { id, runId },
    });
    const query = env.queries.find((item) => item.getNextPageParam);
    assert.deepEqual(
      query.getNextPageParam({ next_cursor: env.cursor }),
      env.cursor,
    );
    const older = env.control("Load older runs");
    assert.equal(older.props.tone, "action");
    older.props.onPress();
    assert.equal(env.nextPages(), 1);
  } finally {
    env.harness.unmount();
  }
});

test("run detail shares the history cache and opens its exact thread", () => {
  const env = setup("AutomationRunDetailScreen");
  try {
    const query = env.queries.find((item) => item.getNextPageParam);
    assert.deepEqual(
      [...query.queryKey],
      ["workspace", "automations", id, "runs"],
    );
    assert.equal(env.control("Outcome").props.value, "Completed");
    assert.equal(env.control("Duration").props.value, "0.5 seconds");
    env.control("Open Thread").props.onPress();
    assert.equal(env.chats[0].conversationId, "nyxa-thread");
  } finally {
    env.harness.unmount();
  }
  const missing = setup("AutomationRunDetailScreen", {
    params: { id, runId: "unknown" },
  });
  try {
    assert.match(missing.text(), /no longer in the recent run history/);
    assert.throws(() => missing.control("Open Thread"));
  } finally {
    missing.harness.unmount();
  }
});

test("pull to refresh refetches the visible page", () => {
  const env = setup("AutomationsScreen", { params: {} });
  try {
    const page = env.all().find((item) => item.type === "SettingsPage")!;
    page.props.onRefresh();
    assert.equal(env.refetches(), 1);
  } finally {
    env.harness.unmount();
  }
});
