import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as presentation from "./automationPresentation";
import * as time from "./automationTime";
import { automationSchema } from "./automationContracts";

const id = "12345678-1234-4123-8123-123456789abc";
const at = "2026-10-27T01:00:00Z";
const run = {
  id,
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
function elements(root: Element): Element[] {
  if (!root || typeof root !== "object" || !root.props) return [];
  return [root, ...[root.props.children].flat(Infinity).flatMap(elements)];
}
function setup(
  page: "AutomationsScreen" | "AutomationDetailScreen" | "AutomationRunsScreen",
  zone = "UTC",
) {
  const routes: any[] = [];
  const chats: any[] = [];
  const queries: any[] = [];
  let nextPages = 0;
  const cursor = { before: at, before_id: id };
  const response = (data: unknown) => ({
    data,
    isPending: false,
    isError: false,
    refetch: async () => {},
  });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": { Pressable: "Pressable", View: "View" },
    "../../components/AgentAvatar": { AgentAvatar: "AgentAvatar" },
    "lucide-react-native": { ChevronRight: "ChevronRight", Clock3: "Clock3" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../app/WorkspaceHeader": {
      useOpenWorkspaceChat: () => (params: any) => chats.push(params),
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice", "WorkspacePage"].map(
        (name) => [name, name],
      ),
    ),
    "../../lib/api/agentsApi": { agentsApi: {} },
    "../../lib/api/automationsApi": { automationsApi: {} },
    "./automationPresentation": presentation,
    "./automationTime": time,
    "./AutomationUI": {
      AUTOMATIONS: ["workspace", "automations"],
      AUTOMATION_SETTINGS: ["workspace", "automations", "settings"],
      AutomationLoading: "AutomationLoading",
      AutomationStatus: "AutomationStatus",
      ChoiceField: "ChoiceField",
      automationError: (error: any) => String(error),
      automationStyles: {},
      useAutomationRuntime: () => ({ active: true, online: true }),
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
      route: { params: { id } },
      navigation: {
        push: (name: any, params: any) => routes.push({ name, params }),
      },
    }),
  );
  const all = () => elements(harness.value());
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
      .filter((item) => item.type === "Label")
      .map((item) => [item.props.children].flat().join(""))
      .join("\n");
  return {
    harness,
    source,
    all,
    control,
    text,
    routes,
    chats,
    queries,
    cursor,
    nextPages: () => nextPages,
  };
}

test("read-only list keeps agent/status/search filters and opens details with no authoring controls", () => {
  const env = setup("AutomationsScreen");
  try {
    env.control("Open Morning release check").props.onPress();
    assert.equal(env.routes[0].name, "AutomationDetail");
    env.control("Status").props.onChange("disabled");
    env.harness.flush();
    assert.ok(env.control("Open Webhook reminder"));
    assert.throws(() => env.control("Open Morning release check"));
    env.control("Status").props.onChange("all");
    env.control("Agent").props.onChange("researcher");
    env.harness.flush();
    assert.throws(() => env.control("Open Webhook reminder"));
    env.control("Search automations").props.onChangeText("not found");
    env.harness.flush();
    assert.match(env.text(), /No matching automations/);
    for (const label of [
      "Create automation",
      "Automation preferences",
      "Run now",
    ])
      assert.throws(() => env.control(label));
  } finally {
    env.harness.unmount();
  }
});

test("read-only details show task, target and confirmation policy and retain history navigation", () => {
  const env = setup("AutomationDetailScreen");
  try {
    const card = env
      .all()
      .find((item) => item.type.name === "AutomationDeliveryCard")!;
    const rendered = elements(card.type(card.props));
    const text = rendered
      .filter((item) => item.type === "Label")
      .map((item) => [item.props.children].flat().join(""))
      .join("\n");
    assert.match(text, /Review the release checklist/);
    assert.match(text, /Researcher/);
    assert.match(text, /Thread: dedicated/);
    assert.match(text, /Confirmation: Destructive actions/);
    env.control("View run history").props.onPress();
    assert.equal(env.routes[0].name, "AutomationRuns");
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

test("automation next, latest and history times use the owner timezone with one display-zone label", () => {
  const zone = "Asia/Singapore";
  for (const page of [
    "AutomationsScreen",
    "AutomationDetailScreen",
    "AutomationRunsScreen",
  ] as const) {
    const env = setup(page, zone);
    const text = env.text();
    assert.equal(text.split(`Times shown in ${zone}`).length - 1, 1);
    assert.ok(text.includes(time.formatAutomationTime(at, zone)));
    if (page === "AutomationsScreen") assert.ok(text.includes(`Next: ${time.formatAutomationTime(at, zone)}`));
    if (page !== "AutomationRunsScreen")
      assert.match(text, /Weekdays at 8:00 AM \(UTC\)/);
    env.harness.unmount();
  }
});

test("NyxBot guidance starts an empty normal NyxBot chat with a fresh draft identity", () => {
  const env = setup("AutomationsScreen");
  try {
    const guidance = env
      .all()
      .find((item) => item.type.name === "AutomationGuidance")!;
    const rows = elements(guidance.type(guidance.props));
    assert.ok(
      rows.some(
        (item) =>
          item.props.children ===
          "To create, run or change automations, ask NyxBot.",
      ),
    );
    const ask = rows.find((item) => item.props.label === "Ask NyxBot")!;
    ask.props.onPress();
    ask.props.onPress();
    for (const chat of env.chats) {
      assert.equal(chat.home, false);
      assert.equal(chat.draft, true);
      assert.equal(chat.agentId, undefined);
      assert.equal(chat.conversationId, undefined);
      assert.equal(chat.openLatest, false);
      assert.equal(chat.message, undefined);
    }
    assert.notEqual(env.chats[0].draftId, env.chats[1].draftId);
  } finally {
    env.harness.unmount();
  }
});

test("run history preserves cursor paging, status/time and exact thread navigation", () => {
  const env = setup("AutomationRunsScreen");
  try {
    assert.match(env.text(), /Scheduled:/);
    assert.match(env.text(), /0.5 seconds/);
    env.control("Open thread").props.onPress();
    assert.equal(env.chats[0].conversationId, "nyxa-thread");
    const query = env.queries.find((item) => item.getNextPageParam);
    assert.deepEqual(
      query.getNextPageParam({ next_cursor: env.cursor }),
      env.cursor,
    );
    env.control("Load older runs").props.onPress();
    assert.equal(env.nextPages(), 1);
  } finally {
    env.harness.unmount();
  }
});
