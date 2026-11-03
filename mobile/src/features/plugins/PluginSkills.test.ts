import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as skillTypes from "./pluginSkillsTypes";
import type { AssistantAgent } from "../agents/agentContracts";
import type { AssistantParams } from "../../app/navigationTypes";

type Element = { type: unknown; props: Record<string, any>; key?: string };
type QueryOptions = {
  queryKey: unknown[];
  enabled?: boolean;
  queryFn: (context: { signal: AbortSignal }) => Promise<unknown>;
};

function elements(root: Element): Element[] {
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

const first = {
  id: "agent-opaque-a",
  kind: "nyxbot",
  name: "nyxbot",
  display_name: "Nyx Bot",
  status: "idle",
  can_maintain: true,
  description: "Personal agent",
} as AssistantAgent;
const second = {
  ...first,
  id: "agent-opaque-b",
  kind: "specialist",
  name: "dns-helper",
  display_name: "DNS helper",
} as AssistantAgent;
const currentSkills = {
  agent_id: first.id,
  revision: 7,
  skills: [
    {
      source: "ornn",
      skill_id: "pin-original",
      name: "dns-helper",
      version: "1.0.0",
      sha256: "sha-original",
      dependencies: [],
      disabled: false,
    },
  ],
  metadata: {
    "pin-original": {
      description: "Manage DNS records safely",
      size_bytes: 234,
    },
  },
};
const version = {
  version: "2.0.0-literal",
  sha256: "sha-exact",
  deprecated: false,
};
const preview = {
  description: "Review the exact selected package",
  size_bytes: 456,
  reference: {
    source: "ornn",
    skill_id: "catalog-opaque",
    name: "weekly_report",
    version: version.version,
    sha256: version.sha256,
    dependencies: [],
    disabled: false,
  },
};

function setup({
  wrapperOnly = false,
  agentId,
  initialPage,
}: { wrapperOnly?: boolean; agentId?: string; initialPage?: any } = {}) {
  const calls: { method: string; args: any[] }[] = [];
  const queries = new Map<string, QueryOptions>();
  const state = new Map<string, any>();
  let focused = true;
  let epoch = 1;
  let requestedAgent = agentId;
  let manager: Element | undefined;
  let routePage: any = initialPage;
  let routeView = initialPage ? "versions" : "detail";
  const routes: { name: string; params: any }[] = [];
  const chats: AssistantParams[] = [];
  const navigation = {
    push: (name: string, params: any) => {
      routes.push(JSON.parse(JSON.stringify({ name, params })));
      routePage = params;
      routeView = (
        {
          SkillDetail: "detail",
          SkillVersions: "versions",
          SkillHistory: "history",
        } as Record<string, string>
      )[name]!;
      harness.render();
    },
    goBack: () => {},
    popTo: (name: string, params: any) => navigation.push(name, params),
  };
  const record =
    (method: string, value?: unknown) =>
    async (...args: any[]) => {
      calls.push({ method, args });
      return value;
    };
  const response = (data?: unknown) => ({
    data,
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: async () => {},
  });
  state.set("agents", response({ agents: [first, second] }));
  state.set(first.id, response(currentSkills));
  state.set(
    second.id,
    response({
      ...currentSkills,
      agent_id: second.id,
      skills: [],
      metadata: {},
    }),
  );
  state.set(
    "catalog",
    response({
      items: [
        {
          id: "catalog-opaque",
          name: "weekly_report",
          description: "Build a weekly sales report.",
        },
      ],
      page: 1,
      total_pages: 4,
    }),
  );
  state.set(
    "versions",
    response({ items: [version], page: 1, total_pages: 1 }),
  );
  state.set("preview", response(preview));
  state.set("test", response());
  state.set("history", response({ items: [] }));
  const client = { invalidateQueries: record("invalidate") };
  const harness = hookHarness<Element>({
    "../../components/AgentAvatar": {
      AgentAvatar: "AgentAvatar",
      AgentAvatarStack: "AgentAvatarStack",
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown, key?: string) => ({
        type,
        props,
        key,
      }),
      jsxs: (type: unknown, props: unknown, key?: string) => ({
        type,
        props,
        key,
      }),
      Fragment: "Fragment",
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "Modal",
          "Pressable",
          "ScrollView",
          "Switch",
          "Text",
          "View",
        ].map((name) => [name, name]),
      ),
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove: () => {} }),
      },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "@react-navigation/native": {
      useIsFocused: () => focused,
      useNavigation: () => navigation,
    },
    "@react-native-community/netinfo": { addEventListener: () => () => {} },
    "@tanstack/react-query": {
      useQueryClient: () => client,
      useQuery: (options: QueryOptions) => {
        const name =
          options.queryKey[1] === "agents"
            ? "agents"
            : String(options.queryKey[3]);
        queries.set(name, options);
        return state.get(name) ?? response();
      },
    },
    "lucide-react-native": Object.fromEntries(
      [
        "BookOpen",
        "Check",
        "ChevronDown",
        "ChevronLeft",
        "ChevronRight",
        "X",
      ].map((name) => [name, name]),
    ),
    "react-native-safe-area-context": {
      SafeAreaProvider: "SafeAreaProvider",
      SafeAreaView: "SafeAreaView",
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionSheet: "WorkspaceSelectionSheet",
    },
    "../../lib/api/agentsApi": {
      agentsApi: {
        list: record("agents", { agents: [first, second] }),
        detail: record("agent-detail", { agent: first }),
      },
    },
    "../../lib/api/assistantApi": { assistantApi: {} },
    "../../lib/api/pluginSkillsApi": {
      pluginSkillsApi: {
        current: record("current", currentSkills),
        search: record("search", state.get("catalog").data),
        versions: record("versions", { items: [version] }),
        preview: record("preview", preview),
        write: record("write"),
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
    "../../app/WorkspaceHeader": {
      useWorkspaceBack: () => {},
      useOpenWorkspaceChat: () => (params: AssistantParams) =>
        chats.push(params),
    },
    "./pluginSkillsTypes": skillTypes,
    "./SkillLibrary": { SkillLibrary: "SkillLibrary" },
  });
  const { PluginSkills } = harness.load(
    fileURLToPath(new URL("./PluginSkills.tsx", import.meta.url)),
  ) as {
    PluginSkills: (props: unknown) => Element;
  };
  harness.mount(() => {
    const wrapper = PluginSkills({
      agentId: requestedAgent,
    });
    manager = elements(wrapper).find(
      (item) =>
        typeof item.type === "function" &&
        item.type.name === "AgentSkillsManager",
    );
    if (wrapperOnly || !manager) return wrapper;
    // Execute the real private child with the same hook runtime. The wrapper
    // remains mounted and its hook order is stable for each manager fixture.
    return (manager.type as (props: unknown) => Element)({
      ...manager.props,
      skillPage: routePage,
      view: routeView,
    });
  });
  const find = (predicate: (item: Element) => boolean) => {
    const item = elements(harness.flush()).find(predicate);
    assert.ok(item, "Expected a matching Skills control");
    return item;
  };
  return {
    harness,
    calls,
    queries,
    state,
    routes,
    chats,
    navigation: navigation.push,
    elements: () => elements(harness.flush()),
    manager: () => manager!,
    sheet: () => find((item) => item.type === "WorkspaceSelectionSheet"),
    hasSheet: () =>
      elements(harness.flush()).some(
        (item) => item.type === "WorkspaceSelectionSheet",
      ),
    library: () => find((item) => item.type === "SkillLibrary"),
    hasModal: () =>
      elements(harness.flush()).some((item) => item.type === "Modal"),
    control: (label: string) =>
      find(
        (item) =>
          item.props.accessibilityLabel === label || item.props.label === label,
      ),
    focused: (value: boolean) => {
      focused = value;
      harness.render();
    },
    epoch: (value: number) => {
      epoch = value;
      harness.render();
    },
    requestedAgent: (value: string | undefined) => {
      requestedAgent = value;
      harness.render();
    },
    unmount: () => harness.unmount(),
  };
}

test("manager defaults to Added, passes descriptions and exact pin state, and keeps catalog disabled", () => {
  const env = setup();
  try {
    const library = env.library().props;
    assert.equal(library.tab, "added");
    assert.equal(library.query, "");
    assert.equal(library.agentName, "Nyx Bot");
    assert.equal(library.added.items[0].id, "pin-original");
    assert.equal(
      library.added.items[0].description,
      "Manage DNS records safely",
    );
    assert.equal(library.added.items[0].version, "1.0.0");
    assert.equal(library.added.items[0].disabled, false);
    assert.equal(library.added.known, true);
    assert.equal(env.queries.get("catalog")?.enabled, false);
    library.onQueryChange("DNS words");
    env.harness.flush();
    assert.equal(env.library().props.query, "DNS words");
    assert.equal(env.queries.get("catalog")?.enabled, false);
    assert.equal(
      env.calls.some(
        (call) => call.method === "search" || call.method === "write",
      ),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("Added and Discover retain separate search inputs and clear only their own scope", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const env = setup();
  try {
    env.library().props.onQueryChange("local DNS");
    env.harness.flush();
    env.library().props.onTabChange("discover");
    env.harness.flush();
    assert.equal(env.library().props.query, "");
    assert.equal(env.queries.get("catalog")?.enabled, true);
    env.library().props.onQueryChange("weekly report");
    env.harness.flush();
    t.mock.timers.tick(300);
    env.harness.flush();
    env.library().props.onTabChange("added");
    env.harness.flush();
    assert.equal(env.library().props.query, "local DNS");
    assert.equal(env.queries.get("catalog")?.enabled, false);
    env.library().props.onQueryChange("");
    env.harness.flush();
    env.library().props.onTabChange("discover");
    env.harness.flush();
    assert.equal(env.library().props.query, "weekly report");
    env.library().props.onQueryChange("");
    env.harness.flush();
    t.mock.timers.tick(300);
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.queryKey[4], "");
    assert.equal(env.queries.get("catalog")?.enabled, true);
  } finally {
    env.unmount();
  }
});

test("Discover debounces by 300ms, hides stale results, resets paging and calls the actual search adapter with the latest text", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const env = setup();
  try {
    env.library().props.onTabChange("discover");
    env.harness.flush();
    env.library().props.onPageChange(3);
    env.harness.flush();
    env.library().props.onQueryChange("  week  ");
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.enabled, false);
    assert.equal(env.library().props.catalog.items.length, 0);
    assert.equal(env.library().props.catalog.known, false);
    assert.equal(env.library().props.catalog.loading, true);
    t.mock.timers.tick(299);
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.queryKey[4], "");
    env.library().props.onQueryChange("  weekly report  ");
    env.harness.flush();
    t.mock.timers.tick(299);
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.enabled, false);
    t.mock.timers.tick(1);
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.enabled, true);
    assert.equal(env.queries.get("catalog")?.queryKey[4], "weekly report");
    assert.equal(env.queries.get("catalog")?.queryKey[5], 1);
    assert.equal(env.library().props.page, 1);
    assert.equal(env.library().props.catalog.known, true);
    const signal = new AbortController().signal;
    await env.queries.get("catalog")!.queryFn({ signal });
    const search = env.calls.find((call) => call.method === "search")!;
    assert.equal(search.args[0], "weekly report");
    assert.equal(search.args[1], 1);
    assert.equal(search.args[2], signal);
  } finally {
    env.unmount();
  }
});

test("pagination is part of the server query and catalog requests stop when unfocused or on Added", () => {
  const env = setup();
  try {
    env.library().props.onTabChange("discover");
    env.harness.flush();
    env.library().props.onPageChange(2);
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.queryKey[5], 2);
    assert.equal(env.library().props.page, 2);
    assert.equal(env.library().props.totalPages, 4);
    env.focused(false);
    assert.equal(env.queries.get("catalog")?.enabled, false);
    assert.equal(env.queries.get(first.id)?.enabled, false);
    env.focused(true);
    assert.equal(env.queries.get("catalog")?.enabled, true);
    env.library().props.onTabChange("added");
    env.harness.flush();
    assert.equal(env.queries.get("catalog")?.enabled, false);
  } finally {
    env.unmount();
  }
});

test("manager reports query failure and unknown data separately from an empty library and delegates scoped retry", async () => {
  const env = setup();
  try {
    const retried: string[] = [];
    Object.assign(env.state.get(first.id), {
      data: undefined,
      isPending: false,
      isError: true,
      error: new Error("Skill pins failed"),
      refetch: async () => {
        retried.push("added");
      },
    });
    Object.assign(env.state.get("catalog"), {
      data: undefined,
      isPending: false,
      isError: true,
      error: new Error("Catalog failed"),
      refetch: async () => {
        retried.push("catalog");
      },
    });
    env.harness.render();
    assert.equal(env.library().props.added.known, false);
    assert.ok(env.library().props.added.error);
    assert.equal(env.library().props.added.items.length, 0);
    env.library().props.added.onRetry();
    env.library().props.onTabChange("discover");
    env.harness.flush();
    assert.equal(env.library().props.catalog.known, false);
    assert.ok(env.library().props.catalog.error);
    env.library().props.catalog.onRetry();
    await env.harness.settle();
    assert.deepEqual(retried, ["added", "catalog"]);
  } finally {
    env.unmount();
  }
});

test("skills agent sheet applies a new keyed manager and Cancel preserves the current context", () => {
  const env = setup({ wrapperOnly: true });
  try {
    assert.equal(env.manager().key, first.id);
    assert.equal(env.manager().props.agent.id, first.id);
    env.control("Agent for skills: Nyx Bot").props.onPress();
    env.harness.flush();
    const sheet = env.sheet().props;
    assert.deepEqual(Array.from(sheet.value), [first.id]);
    assert.equal(sheet.allowEmpty, false);
    assert.deepEqual(
      Array.from(sheet.options, (option: { value: string }) => option.value),
      [first.id, second.id],
    );
    assert.equal(env.manager().props.agent.id, first.id);
    sheet.onClose();
    env.harness.flush();
    assert.equal(env.manager().props.agent.id, first.id);
    assert.equal(env.hasSheet(), false);
    env.control("Agent for skills: Nyx Bot").props.onPress();
    env.harness.flush();
    const applying = env.sheet().props;
    applying.onChange([second.id]);
    applying.onClose();
    env.harness.flush();
    assert.equal(env.manager().key, second.id);
    assert.equal(env.manager().props.agent.id, second.id);
    assert.equal("query" in env.manager().props, false);
  } finally {
    env.unmount();
  }
  const explicit = setup({ agentId: second.id });
  try {
    assert.equal(explicit.library().props.agentName, "DNS helper");
    assert.equal(explicit.library().props.query, "");
    assert.equal(explicit.library().props.tab, "added");
    assert.equal(explicit.library().props.added.items.length, 0);
    assert.equal(explicit.queries.get(second.id)?.enabled, true);
  } finally {
    explicit.unmount();
  }
});

test("skills choices exclude unavailable agents and discard drafts when route, owner, account, or focus changes", () => {
  const env = setup({ wrapperOnly: true });
  const open = () => {
    env
      .control(`Agent for skills: ${env.manager().props.agent.display_name}`)
      .props.onPress();
    env.harness.flush();
    assert.equal(env.hasSheet(), true);
  };
  try {
    env.state.get("agents").data.agents = [
      first,
      {
        ...second,
        owner_kind: "org",
        owner_id: "org-a",
        owner_name: "DNS team",
      },
      { ...first, id: "destroyed", status: "destroyed" },
      { ...first, id: "readonly", can_maintain: false },
    ];
    env.harness.render();
    open();
    assert.deepEqual(
      Array.from(
        env.sheet().props.options,
        (option: { value: string }) => option.value,
      ),
      [first.id, second.id],
    );
    assert.equal(env.sheet().props.options[1].description, "DNS team");
    env.requestedAgent(second.id);
    assert.equal(env.hasSheet(), false);
    open();
    const oldScope = env.sheet().props.scopeKey;
    env.state.get("agents").data.agents[1] = {
      ...env.state.get("agents").data.agents[1],
      owner_id: "org-b",
    };
    env.harness.render();
    assert.equal(env.hasSheet(), false);
    open();
    assert.notEqual(env.sheet().props.scopeKey, oldScope);
    env.epoch(2);
    assert.equal(env.hasSheet(), false);
    open();
    env.focused(false);
    assert.equal(env.hasSheet(), false);
  } finally {
    env.unmount();
  }
});

test("catalog review opens original identity without mutating and exact attach retains revision plus sibling pins", async () => {
  const env = setup();
  try {
    env.library().props.onTabChange("discover");
    env.harness.flush();
    env
      .library()
      .props.onOpenSkill(
        "catalog-opaque",
        "weekly_report",
        "Build a weekly sales report.",
      );
    env.harness.flush();
    assert.equal(env.hasModal(), false);
    assert.deepEqual(env.routes.at(-1), {
      name: "SkillDetail",
      params: {
        agentId: first.id,
        skillId: "catalog-opaque",
        name: "weekly_report",
        description: "Build a weekly sales report.",
      },
    });
    assert.equal(env.queries.get("versions")?.queryKey[4], "catalog-opaque");
    assert.equal(env.queries.get("versions")?.enabled, true);
    assert.equal(
      env.calls.length,
      0,
      "opening a catalog result must not attach it",
    );
    env.control("Attach exact version").props.onPress();
    assert.equal(env.routes.at(-1)?.name, "SkillVersions");
    env.harness.flush();
    assert.equal(env.control("Attach skill").props.disabled, true);
    env.control(`Select version ${version.version}`).props.onPress();
    env.harness.flush();
    assert.equal(env.queries.get("preview")?.queryKey[4], "catalog-opaque");
    assert.equal(env.queries.get("preview")?.queryKey[5], version.version);
    assert.equal(env.control("Attach skill").props.disabled, false);
    env.control("Attach skill").props.onPress();
    await env.harness.settle();
    const written = env.calls.find((call) => call.method === "write")!;
    assert.ok(written);
    assert.equal(env.routes.at(-1)?.name, "SkillDetail");
    assert.equal(written.args[0], first.id);
    assert.equal(written.args[1].expected_revision, 7);
    assert.equal(written.args[1].skills.length, 2);
    assert.equal(written.args[1].skills[0].skill_id, "pin-original");
    assert.equal(written.args[1].skills[1].skill_id, "catalog-opaque");
    assert.equal(written.args[1].skills[1].version, "2.0.0-literal");
    assert.equal(written.args[1].skills[1].sha256, "sha-exact");
    assert.equal(
      env.calls.filter((call) => call.method === "current").length,
      1,
    );
    assert.equal(
      env.calls.filter((call) => call.method === "agent-detail").length,
      1,
    );
  } finally {
    env.unmount();
  }
});

test("skill detail is read-only with version choice, newest supported pin and agent removal", async () => {
  const env = setup();
  try {
    env.library().props.onOpenSkill("pin-original", "dns-helper");
    env.harness.flush();
    assert.throws(() => env.control("Edit / improve skill"));
    assert.throws(() => env.control("Review draft"));
    assert.equal(
      env
        .elements()
        .some((row) => row.type === "Switch" || row.type === "Field"),
      false,
    );
    assert.ok(
      env
        .elements()
        .some(
          (row) => row.props.children === "To change this skill, ask NyxBot.",
        ),
    );
    env.state.get("versions").data.items = [
      { ...version, version: "deprecated", deprecated: true },
      version,
    ];
    env.harness.render();
    env.control(`Use latest (v${version.version})`).props.onPress();
    assert.equal(env.routes.at(-1)?.name, "SkillVersions");
    assert.equal(
      env.control(`Select version ${version.version}`).props.accessibilityState
        .checked,
      true,
    );
    assert.equal(env.queries.get("preview")?.queryKey[5], version.version);
    assert.equal(
      env.calls.filter((call) => call.method === "write").length,
      0,
      "review before committing the exact pin",
    );
    env.navigation("SkillDetail", {
      agentId: first.id,
      skillId: "pin-original",
      name: "dns-helper",
    });
    env.control("Remove from agent").props.onPress();
    env.control("Confirm remove from agent").props.onPress();
    await env.harness.settle();
    const written = env.calls.find((call) => call.method === "write")!;
    assert.equal(written.args[0], first.id);
    assert.equal(written.args[1].expected_revision, 7);
    assert.deepEqual(Array.from(written.args[1].skills), []);
  } finally {
    env.unmount();
  }
});

test("skill guidance opens a fresh normal NyxBot chat while retaining the exact caller", () => {
  const env = setup({ agentId: second.id });
  try {
    env.library().props.onOpenSkill("catalog-opaque", "weekly_report");
    const caller = env.routes.at(-1);
    env.control("Ask NyxBot").props.onPress();
    env.control("Ask NyxBot").props.onPress();
    assert.equal(env.chats.length, 2);
    for (const chat of env.chats) {
      assert.equal(chat.home, false);
      assert.equal(chat.agentId, undefined, "authoring belongs to NyxBot");
      assert.equal(chat.conversationId, undefined);
      assert.equal(chat.draft, true);
      assert.equal(chat.openLatest, false);
    }
    assert.notEqual(env.chats[0]!.draftId, env.chats[1]!.draftId);
    assert.equal(env.routes.at(-1), caller);
    assert.equal(env.routes.length, 1);
    assert.equal(
      env.calls.length,
      0,
      "opening chat must not send or change pins",
    );
  } finally {
    env.unmount();
  }
});

test("skill details distinguish loading or failed pin reads from a known unattached skill and retry only that read", async () => {
  const env = setup();
  const text = (value: string) =>
    env.elements().some((item) => item.props.children === value);
  try {
    const query = env.state.get(first.id);
    Object.assign(query, { data: undefined, isPending: true });
    env.library().props.onOpenSkill("catalog-opaque", "weekly_report");
    assert.ok(text("Loading attached version…"));
    assert.equal(text("Not attached"), false);
    assert.equal(env.control("Attach exact version").props.disabled, true);

    let retries = 0;
    Object.assign(query, {
      isPending: false,
      isError: true,
      error: new Error("Pin read unavailable"),
      refetch: async () => {
        retries++;
        Object.assign(query, { isError: false, data: currentSkills });
      },
    });
    env.harness.render();
    assert.ok(text("Attachment status unavailable"));
    assert.equal(text("Not attached"), false);
    assert.ok(
      env
        .elements()
        .some((item) => item.type === "Notice" && item.props.tone === "error"),
    );
    assert.equal(env.control("Attach exact version").props.disabled, true);
    assert.equal(env.control("Ask NyxBot").props.disabled, false);
    env.control("Retry attached skills").props.onPress();
    await env.harness.settle();
    env.harness.render();
    assert.equal(retries, 1);
    assert.ok(text("Not attached"));
    assert.equal(env.control("Attach exact version").props.disabled, false);
    assert.equal(env.calls.length, 0);
  } finally {
    env.unmount();
  }
});

test("cached pin details survive refresh and a failed refresh blocks pin changes until a successful read", async () => {
  const env = setup();
  const text = (value: string) =>
    env.elements().some((item) => item.props.children === value);
  try {
    env.library().props.onOpenSkill("pin-original", "dns-helper");
    const query = env.state.get(first.id);
    Object.assign(query, { isFetching: true });
    env.harness.render();
    assert.ok(text("1.0.0"));
    assert.equal(text("Loading attached version…"), false);
    Object.assign(query, {
      isFetching: false,
      isError: true,
      error: new Error("Refresh failed"),
    });
    env.harness.render();
    assert.ok(text("1.0.0"));
    assert.ok(text("Last loaded version"));
    assert.equal(env.control("Remove from agent").props.disabled, true);
    assert.equal(env.control("Choose exact version").props.disabled, true);
    assert.equal(
      env.control(`Use latest (v${version.version})`).props.disabled,
      true,
    );

    env.navigation("SkillVersions", {
      agentId: first.id,
      skillId: "pin-original",
      name: "dns-helper",
    });
    env.control(`Select version ${version.version}`).props.onPress();
    assert.equal(env.control("Update exact version").props.disabled, true);
    env.control("Update exact version").props.onPress();
    await env.harness.settle();
    assert.equal(env.calls.length, 0, "failed pin reads cannot start a write");

    Object.assign(query, { isError: false });
    env.harness.render();
    assert.equal(env.control("Update exact version").props.disabled, false);
  } finally {
    env.unmount();
  }
});

test("Use latest carries its literal version across a real versions-page remount", () => {
  const env = setup({
    initialPage: {
      agentId: first.id,
      skillId: "catalog-opaque",
      name: "weekly_report",
      version: version.version,
    },
  });
  try {
    assert.equal(
      env.control(`Select version ${version.version}`).props.accessibilityState
        .checked,
      true,
    );
    assert.equal(env.queries.get("preview")?.queryKey[5], version.version);
    assert.equal(env.control("Attach skill").props.disabled, false);
    assert.equal(env.calls.length, 0);
  } finally {
    env.unmount();
  }
});

test("the latest pinned version is badged and does not offer a redundant latest write", () => {
  const env = setup();
  try {
    env.library().props.onOpenSkill("pin-original", "dns-helper");
    env.harness.flush();
    env.state.get("versions").data.items = [
      { ...version, version: "1.0.0", sha256: "sha-original" },
    ];
    env.harness.render();
    assert.ok(
      env
        .elements()
        .some(
          (row) =>
            typeof row.type === "function" && row.type.name === "LatestBadge",
        ),
    );
    assert.throws(() => env.control("Use latest (v1.0.0)"));
    env.control("Choose exact version").props.onPress();
    assert.ok(
      env
        .elements()
        .some(
          (row) =>
            typeof row.type === "function" && row.type.name === "LatestBadge",
        ),
    );
    assert.equal(env.calls.filter((call) => call.method === "write").length, 0);
  } finally {
    env.unmount();
  }
});
