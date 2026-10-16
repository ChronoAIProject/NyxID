import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as skillTypes from "./pluginSkillsTypes";
import { skillModalBackAction } from "./skillModalNavigation";
import type { AssistantAgent } from "../agents/agentContracts";

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
}: { wrapperOnly?: boolean; agentId?: string } = {}) {
  const calls: { method: string; args: any[] }[] = [];
  const queries = new Map<string, QueryOptions>();
  const state = new Map<string, any>();
  let focused = true;
  let epoch = 1;
  let requestedAgent = agentId;
  let manager: Element | undefined;
  const changes: boolean[] = [];
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
  state.set("features", response([]));
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
  state.set("proposals", response({ proposals: [] }));
  const client = { invalidateQueries: record("invalidate") };
  const harness = hookHarness<Element>({
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
    "@react-navigation/native": { useIsFocused: () => focused },
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
        features: record("features", []),
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
    "./skillDraftSubmission": { submitSkillDraft: async () => {} },
    "./skillModalNavigation": { skillModalBackAction },
    "./pluginSkillsTypes": skillTypes,
    "./authoredSkillReviewState": {},
    "./SkillLibrary": { SkillLibrary: "SkillLibrary" },
  });
  const { PluginSkills } = harness.load(
    fileURLToPath(new URL("./PluginSkills.tsx", import.meta.url)),
  ) as {
    PluginSkills: (props: unknown) => Element;
  };
  const detailChange = (open: boolean) => changes.push(open);
  harness.mount(() => {
    const wrapper = PluginSkills({
      agentId: requestedAgent,
      onDetailChange: detailChange,
    });
    manager = elements(wrapper).find(
      (item) =>
        typeof item.type === "function" &&
        item.type.name === "AgentSkillsManager",
    );
    if (wrapperOnly || !manager) return wrapper;
    // Execute the real private child with the same hook runtime. The wrapper
    // remains mounted and its hook order is stable for each manager fixture.
    return (manager.type as (props: unknown) => Element)(manager.props);
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
    changes,
    manager: () => manager!,
    sheet: () => find((item) => item.type === "WorkspaceSelectionSheet"),
    hasSheet: () =>
      elements(harness.flush()).some(
        (item) => item.type === "WorkspaceSelectionSheet",
      ),
    library: () => find((item) => item.type === "SkillLibrary"),
    modal: () => find((item) => item.type === "Modal"),
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
    assert.equal(env.modal().props.visible, true);
    assert.equal(env.queries.get("versions")?.queryKey[4], "catalog-opaque");
    assert.equal(env.queries.get("versions")?.enabled, true);
    assert.equal(
      env.calls.length,
      0,
      "opening a catalog result must not attach it",
    );
    env.control("Attach exact version").props.onPress();
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
