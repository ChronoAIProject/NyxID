import { translate, type TranslationValues } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { renderSegmentControl } from "../../components/SegmentControl.testHarness";
import { deriveConnectorItems, matchesPluginQuery } from "./pluginConnectors";
import * as designTokens from "../../theme/designTokens";
import type {
  CatalogEntry,
  ServiceConnection,
} from "../services/serviceContracts";

type Element = { type: unknown; props: Record<string, any> };
type QueryOptions = {
  queryKey: unknown[];
  enabled: boolean;
  queryFn: (context: { signal: AbortSignal }) => Promise<unknown>;
};

function elements(root: Element): Element[] {
  if (typeof root.type === "function") return elements(root.type(root.props));
  // Settings rows carry brand icons and controls outside their children.
  const children = [
    root.props.children,
    root.props.leading,
    root.props.accessory,
  ];
  if (root.type === "SectionList") {
    children.push(root.props.ListHeaderComponent);
    if (!root.props.sections.length)
      children.push(root.props.ListEmptyComponent);
    for (const section of root.props.sections)
      children.push(
        root.props.renderSectionHeader({ section }),
        section.data.map((item: unknown, index: number) =>
          root.props.renderItem({ item, index, section }),
        ),
        root.props.renderSectionFooter({ section }),
      );
  }
  if (root.type === "FlatList") {
    children.push(
      root.props.ListHeaderComponent,
      root.props.ListFooterComponent,
      root.props.data.length
        ? root.props.data.map((item: unknown, index: number) =>
            root.props.renderItem({ item, index }),
          )
        : root.props.ListEmptyComponent,
    );
  }
  return [
    root,
    ...children
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

function text(root: Element): string {
  return [root.props.children]
    .flat(Infinity)
    .map((child) =>
      child && typeof child === "object" && "props" in child
        ? text(child)
        : typeof child === "string" || typeof child === "number"
          ? String(child)
          : "",
    )
    .join("");
}

const github: CatalogEntry = {
  slug: "api-github",
  name: "GitHub",
  description: "Issues and source code",
  service_type: "http",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  requires_credential: true,
  requires_gateway_url: false,
  provider_type: "oauth2",
};
const cloudflare: CatalogEntry = {
  ...github,
  slug: "api-cloudflare",
  name: "Cloudflare",
  description: "DNS and websites",
};
const slack: CatalogEntry = {
  ...github,
  slug: "api-slack",
  name: "Slack",
  description: "Team messages",
};
const personal: ServiceConnection = {
  id: "github-personal",
  label: "Personal GitHub",
  slug: "github-proxy",
  catalog_service_slug: github.slug,
  credential_type: "oauth2",
  is_active: true,
  auto_connected: false,
  status: "active",
  credential_source: { type: "personal" },
};
const dnsPersonal: ServiceConnection = {
  ...personal,
  id: "cloudflare-personal",
  label: "Personal Cloudflare",
  catalog_service_slug: cloudflare.slug,
};
const dnsWork: ServiceConnection = {
  ...dnsPersonal,
  id: "cloudflare-work",
  label: "Work Cloudflare",
  credential_source: {
    type: "org",
    org_id: "org-opaque",
    org_name: "Work",
    role: "admin",
    allowed: true,
  },
};

test("Added rows render catalog brand slugs for suffixed production instances even before catalog loads", () => {
  const slugs = ["api-github", "api-notion", "api-google-workspace"];
  const view = setup({
    connections: slugs.map((slug, index) => ({
      ...personal,
      id: `production-key-${index}`,
      slug: `${slug}-${index + 2}`,
      catalog_service_slug: slug,
    })),
    catalog: [],
  });
  assert.deepEqual(
    view
      .all()
      .filter((row) => row.type === "ServiceIcon")
      .map((row) => row.props.slug),
    slugs,
  );
  view.connector("service:api-github");
  assert.equal(
    view.all().find((row) => row.type === "ServiceIcon")?.props.slug,
    "api-github",
  );
  view.harness.unmount();
});

function setup({
  connections = [personal, dnsPersonal, dnsWork],
  catalog = [github, cloudflare, slack],
  connected = true,
  language = "en",
}: {
  connections?: ServiceConnection[];
  catalog?: CatalogEntry[];
  connected?: boolean;
  language?: Language;
} = {}) {
  const calls: { method: string; args: unknown[] }[] = [];
  const queries = new Map<string, QueryOptions>();
  let back: { active: boolean; onBack: () => void; defaultRole?: string };
  let focused = true;
  let online = connected;
  let currentLanguage = language;
  let foreground: (state: string) => void = () => {};
  const record =
    (method: string) =>
    (...args: unknown[]) => {
      calls.push({ method, args });
    };
  const response = (data: unknown, name: string) => ({
    data,
    isPending: false,
    isFetching: false,
    isError: false,
    fetchStatus: "idle" as "idle" | "fetching" | "paused",
    error: undefined as Error | undefined,
    refetch: record(`refetch-${name}`),
  });
  const states = {
    plugins: response(connections, "plugins"),
    catalog: response(catalog, "catalog"),
  };
  const navigation = {
    push: record("push"),
    setOptions: record("setOptions"),
  };
  const openChat = record("openChat");
  const mocks: Record<string, unknown> = {
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => {
        const renderedLanguage = currentLanguage;
        return {
          t: (phrase: string, values?: TranslationValues) =>
            translate(renderedLanguage, phrase, values),
        };
      },
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "FlatList",
          "SectionList",
          "Pressable",
          "ScrollView",
          "Text",
          "TextInput",
          "View",
        ].map((name) => [name, name]),
      ),
      AppState: {
        currentState: "active",
        addEventListener: (_: string, callback: typeof foreground) => {
          foreground = callback;
          return { remove: () => {} };
        },
      },
      Keyboard: { dismiss: record("dismissKeyboard") },
      StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 0.5 },
    },
    "@react-navigation/native": {
      useNavigation: () => navigation,
      useIsFocused: () => focused,
    },
    "@tanstack/react-query": {
      useQuery: (options: QueryOptions) => {
        const name = options.queryKey.at(-1) as keyof typeof states;
        queries.set(name, options);
        return states[name];
      },
    },
    "lucide-react-native": Object.fromEntries(
      ["RefreshCw", "Search", "X"].map((name) => [name, name]),
    ),
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsRow: "SettingsRow",
    },
    "../../app/WorkspaceHeader": {
      useOpenWorkspaceChat: () => openChat,
      useWorkspaceBack: (options: typeof back) => {
        back = options;
      },
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "../../components/SegmentControl": {
      SegmentControl: renderSegmentControl,
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["Notice", "WorkspacePage"].map((name) => [name, name]),
    ),
    "../../lib/api/servicesApi": {
      servicesApi: {
        pluginConnections: record("pluginConnections"),
        catalog: record("catalog"),
      },
    },
    "../../theme/designTokens": designTokens,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../services/serviceContracts": {
      connectionOwner: (connection: ServiceConnection) =>
        connection.credential_source?.type === "org" ? "Work" : "Personal",
    },
    "./pluginConnectors": { deriveConnectorItems, matchesPluginQuery },
    "./PluginSkills": { PluginSkills: "PluginSkills" },
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/OfflineBanner": { OfflineBanner: "OfflineBanner" },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: online }),
    },
  };
  const harness = hookHarness<Element>(mocks);
  // The real pull-refresh hook runs on the harness's React.
  mocks["../../hooks/usePullRefresh"] = harness.load(
    fileURLToPath(new URL("../../hooks/usePullRefresh.ts", import.meta.url)),
  );
  const { PluginsScreen } = harness.load(
    fileURLToPath(new URL("./PluginsScreen.tsx", import.meta.url)),
  ) as { PluginsScreen: (props?: unknown) => Element };
  const screenProps: { connectorId?: string } = {};
  harness.mount(() => PluginsScreen(screenProps));
  const all = () => elements(harness.flush());
  const find = (predicate: (element: Element) => boolean) => {
    const result = all().find(predicate);
    assert.ok(result, "Expected a Plugins control");
    return result;
  };
  return {
    harness,
    connector: (id?: string) => {
      screenProps.connectorId = id;
      harness.render();
    },
    calls,
    states,
    queries,
    all,
    back: () => back,
    list: () => find((item) => item.type === "SectionList").props,
    added: () =>
      find((item) => item.type === "SectionList").props.sections.find(
        (section: { key: string }) => section.key === "added",
      ).data,
    discover: () =>
      find((item) => item.type === "SectionList").props.sections.find(
        (section: { key: string }) => section.key === "discover",
      ).data,
    texts: () => all().map(text).join("\n"),
    control: (label: string) =>
      find(
        (item) =>
          item.props.accessibilityLabel === label || item.props.label === label,
      ),
    tab: (title: string) =>
      find(
        (item) =>
          item.props.accessibilityRole === "tab" && text(item) === title,
      ),
    focused: (value: boolean) => {
      focused = value;
      harness.render();
    },
    online: (value: boolean) => {
      online = value;
      harness.render();
    },
    foreground: (value: boolean) => {
      foreground(value ? "active" : "background");
      harness.flush();
    },
    language: (value: Language) => {
      currentLanguage = value;
      harness.render();
    },
    unmount: () => harness.unmount(),
  };
}

const names = (items: readonly { name: string }[]) =>
  Array.from(items, (item) => item.name);

test("automatic credentials cannot enter the mobile management picker or counts", () => {
  const env = setup({
    connections: [
      { ...personal, id: "auto-github", auto_connected: true },
      { ...dnsPersonal, is_active: false },
      { ...dnsWork, id: "auto-dns", auto_connected: true },
    ],
  });
  try {
    assert.equal(env.added().length, 1);
    assert.match(env.texts(), /ADDED · 1/);
    assert.deepEqual(Array.from(env.added()[0].manageKeyIds), [dnsPersonal.id]);
    assert.equal(env.added()[0].disabled, true);
    env.connector("service:api-cloudflare");
    assert.ok(env.control("Manage Personal Cloudflare"));
    assert.ok(
      !env
        .all()
        .some(
          (node) => node.props.accessibilityLabel === "Manage Work Cloudflare",
        ),
    );
    env.connector();
    assert.deepEqual(names(env.discover()), ["Slack"]);
  } finally {
    env.unmount();
  }
});

test("Connectors is one list: Added first, then Discover with only available services, and no nested selector", () => {
  const env = setup();
  try {
    assert.equal(env.list().accessibilityLabel, "Connectors");
    assert.deepEqual(
      env
        .all()
        .filter((node) => node.type === "ServiceIcon")
        .map((node) => node.props.slug),
      ["api-github", "api-cloudflare", "api-slack"],
    );
    assert.deepEqual(names(env.added()), ["GitHub", "Cloudflare"]);
    assert.deepEqual(names(env.discover()), ["Slack"]);
    assert.match(env.texts(), /ADDED · 2[\s\S]*DISCOVER · 1/);
    assert.deepEqual(
      env
        .all()
        .filter((item) => item.props.accessibilityRole === "tab")
        .map(text),
      ["Connectors", "Skills"],
    );
    assert.equal(
      env.calls.some((call) => call.method === "push"),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("one search filters both Added and Discover and explains an empty section", () => {
  const env = setup();
  try {
    env.control("Search connectors").props.onChangeText("DNS");
    assert.deepEqual(names(env.added()), ["Cloudflare"]);
    assert.deepEqual(names(env.discover()), []);
    assert.match(env.texts(), /No available connectors match this search\./);
    env.control("Search connectors").props.onChangeText("slack");
    assert.deepEqual(names(env.added()), []);
    assert.deepEqual(names(env.discover()), ["Slack"]);
    assert.match(env.texts(), /No added connectors match this search\./);
    env.control("Clear connector search").props.onPress();
    assert.equal(env.control("Search connectors").props.value, "");
    assert.equal(env.added().length, 2);
  } finally {
    env.unmount();
  }
});

test("single connections, grouped connections, and catalog services retain their distinct routes", () => {
  const env = setup();
  try {
    env.control("Manage GitHub, Enabled").props.onPress();
    const single = env.calls.filter((call) => call.method === "push").at(-1)!;
    assert.equal(single.args[0], "ServiceSetup");
    assert.equal(
      (single.args[1] as { connectionId: string }).connectionId,
      personal.id,
    );
    env.control("Manage Cloudflare, Enabled").props.onPress();
    env.harness.flush();
    const groupPage = env.calls
      .filter((call) => call.method === "push")
      .at(-1)!;
    assert.equal(groupPage.args[0], "PluginConnections");
    env.connector((groupPage.args[1] as { connectorId: string }).connectorId);
    assert.equal(env.back().defaultRole, "back");
    env.control("Manage Work Cloudflare").props.onPress();
    const grouped = env.calls.filter((call) => call.method === "push").at(-1)!;
    assert.equal(grouped.args[0], "ServiceSetup");
    assert.equal(
      (grouped.args[1] as { connectionId: string }).connectionId,
      dnsWork.id,
    );
    env.connector();
    env.harness.flush();
    assert.equal(env.back().defaultRole, "menu");
    env.control("Connect Slack, OAuth, available to add").props.onPress();
    const available = env.calls
      .filter((call) => call.method === "push")
      .at(-1)!;
    assert.equal(available.args[0], "ServiceSetup");
    assert.equal(
      (available.args[1] as { catalogSlug: string }).catalogSlug,
      slack.slug,
    );
  } finally {
    env.unmount();
  }
});

test("an empty Added library points at Discover without automatically connecting a service", () => {
  const env = setup({ connections: [] });
  try {
    assert.equal(env.added().length, 0);
    assert.match(
      env.texts(),
      /No connectors added yet\. Add one from Discover below\./,
    );
    assert.equal(env.discover().length, 3);
    assert.equal(
      env.calls.some((call) => call.method === "push"),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("a failed connections read fails closed with absent or stale cached connections", () => {
  for (const keepStaleData of [false, true]) {
    const env = setup();
    try {
      Object.assign(env.states.plugins, {
        data: keepStaleData ? env.states.plugins.data : undefined,
        isError: true,
        error: new Error("Connections unavailable"),
      });
      env.harness.render();
      assert.equal(env.discover().length, 0);
      assert.equal(
        env
          .all()
          .some((item) =>
            String(item.props.accessibilityLabel).startsWith("Connect GitHub"),
          ),
        false,
      );
      env.control("Retry loading plugins").props.onPress();
      assert.deepEqual(
        env.calls
          .filter((call) => call.method.startsWith("refetch-"))
          .map((call) => call.method),
        ["refetch-plugins", "refetch-catalog"],
      );
    } finally {
      env.unmount();
    }
  }
});

test("disabled services remain manageable in Added and never appear in Discover", () => {
  const env = setup({ connections: [{ ...personal, is_active: false }] });
  try {
    assert.equal(env.added()[0].status, "Disabled");
    env.control("Manage GitHub, Disabled").props.onPress();
    assert.equal(env.calls.filter((call) => call.method === "push").length, 1);
    assert.equal(
      env.discover().some((item: { name: string }) => item.name === "GitHub"),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("Skills switches to the agent-scoped manager and connector queries pause when inactive", () => {
  const env = setup();
  try {
    env.tab("Skills").props.onPress();
    env.harness.flush();
    assert.equal(
      env.all().some((item) => item.type === "PluginSkills"),
      true,
    );
    assert.equal(
      env.all().some((item) => item.type === "SectionList"),
      false,
    );
    env.tab("Connectors").props.onPress();
    assert.equal(env.list().accessibilityLabel, "Connectors");
    env.focused(false);
    assert.equal(env.queries.get("plugins")?.enabled, false);
    assert.equal(env.queries.get("catalog")?.enabled, false);
    env.focused(true);
    env.foreground(false);
    assert.equal(env.queries.get("plugins")?.enabled, false);
    env.foreground(true);
    assert.equal(env.queries.get("plugins")?.enabled, true);
  } finally {
    env.unmount();
  }
});

test("connector rows tag their state or sign-in method beside the name, never as a wrapping value", () => {
  const env = setup({
    connections: [personal, { ...dnsPersonal, status: "failed" }],
  });
  try {
    const rows = () => env.all().filter((node) => node.type === "SettingsRow");
    assert.deepEqual(
      rows().map((row) => [
        row.props.label,
        row.props.badge,
        row.props.badgeTone,
        row.props.value,
      ]),
      [
        ["GitHub", "Enabled", "default", undefined],
        ["Cloudflare", "Needs attention", "danger", undefined],
        ["Slack", "OAuth", "default", undefined],
      ],
    );
    assert.equal(rows()[0]!.props.detailLines, 2);
    assert.equal(
      env.calls.some((call) => call.method === "push"),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("first load shows a skeleton of connector rows, not a spinner", () => {
  const env = setup();
  try {
    Object.assign(env.states.plugins, { data: undefined, isPending: true });
    env.harness.render();
    const skeleton = env.all().find((node) => node.type === "SkeletonRows");
    assert.equal(skeleton?.props.accessibilityLabel, "Loading plugins");
    assert.equal(
      env.all().some((node) => node.type === "ActivityIndicator"),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("background refetches never show the pull or Refresh spinner; the user's own refresh does", async () => {
  const env = setup();
  try {
    Object.assign(env.states.plugins, { isFetching: true });
    Object.assign(env.states.catalog, { isFetching: true });
    env.harness.render();
    assert.equal(env.list().refreshing, false);
    assert.equal(
      env.all().some((node) => node.type === "ActivityIndicator"),
      false,
    );
    let finish!: () => void;
    env.states.plugins.refetch = (() =>
      new Promise<void>((resolve) => {
        finish = resolve;
      })) as never;
    env.harness.render();
    env.list().onRefresh();
    env.harness.flush();
    assert.equal(env.list().refreshing, true);
    assert.equal(
      env.all().some((node) => node.type === "ActivityIndicator"),
      true,
    );
    finish();
    await env.harness.settle();
    assert.equal(env.list().refreshing, false);
  } finally {
    env.unmount();
  }
});

test("offline cold loads show reconnection guidance instead of an endless pending skeleton", async () => {
  const env = setup({ connected: false });
  try {
    for (const state of Object.values(env.states))
      Object.assign(state, {
        data: undefined,
        isPending: true,
        fetchStatus: "paused",
      });
    env.harness.render();
    assert.equal(env.queries.get("plugins")?.enabled, false);
    assert.equal(env.queries.get("catalog")?.enabled, false);
    assert.equal(
      env.all().some((row) => row.type === "SkeletonRows"),
      false,
    );
    assert.equal(
      env.all().some((row) => row.type === "OfflineBanner"),
      true,
    );
    assert.match(env.texts(), /Reconnect to load/);
    assert.doesNotMatch(
      env.texts(),
      /No connectors added yet|All available connectors are already added/,
    );
    assert.equal(env.control("Refresh plugins").props.disabled, true);
    env.list().onRefresh();
    env.foreground(false);
    env.foreground(true);
    await env.harness.settle();
    assert.equal(env.list().refreshing, false);
    assert.equal(
      env.calls.some((call) => call.method.startsWith("refetch-")),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("offline connectors retain cached rows even when the missing catalog remains pending", () => {
  const env = setup({ connected: false });
  try {
    Object.assign(env.states.catalog, {
      data: undefined,
      isPending: true,
      fetchStatus: "paused",
    });
    env.harness.render();
    assert.deepEqual(names(env.added()), [
      "Personal GitHub",
      "Personal Cloudflare",
    ]);
    assert.ok(env.control("Search connectors"));
    assert.equal(
      env.all().some((row) => row.type === "SkeletonRows"),
      false,
    );
    assert.match(
      env.all().find((row) => row.type === "OfflineBanner")?.props.subtitle,
      /cached/i,
    );
  } finally {
    env.unmount();
  }
});

test("reconnecting enables normal queries and refresh without replaying offline refreshes", async () => {
  const env = setup({ connected: false });
  try {
    for (const state of Object.values(env.states))
      Object.assign(state, {
        data: undefined,
        isPending: true,
        fetchStatus: "paused",
      });
    env.harness.render();
    env.list().onRefresh();
    await env.harness.settle();
    assert.equal(
      env.calls.some((call) => call.method.startsWith("refetch-")),
      false,
    );
    env.online(true);
    assert.equal(env.queries.get("plugins")?.enabled, true);
    assert.equal(env.queries.get("catalog")?.enabled, true);
    assert.equal(
      env.all().some((row) => row.type === "OfflineBanner"),
      false,
    );
    Object.assign(env.states.plugins, {
      data: [personal],
      isPending: false,
      fetchStatus: "idle",
    });
    Object.assign(env.states.catalog, {
      data: [github, slack],
      isPending: false,
      fetchStatus: "idle",
    });
    env.harness.render();
    assert.deepEqual(names(env.added()), ["GitHub"]);
    assert.deepEqual(names(env.discover()), ["Slack"]);
    env.list().onRefresh();
    await env.harness.settle();
    assert.deepEqual(
      env.calls
        .filter((call) => call.method.startsWith("refetch-"))
        .map((call) => call.method),
      ["refetch-plugins", "refetch-catalog"],
    );
  } finally {
    env.unmount();
  }
});

test("offline connector management does not claim an uncached connection was removed", () => {
  const env = setup({ connected: false });
  try {
    for (const state of Object.values(env.states))
      Object.assign(state, {
        data: undefined,
        isPending: true,
        fetchStatus: "paused",
      });
    env.connector("service:api-github");
    assert.equal(
      env.all().some((row) => row.type === "OfflineBanner"),
      true,
    );
    const group = env.all().find((row) => row.type === "SettingsGroup");
    assert.match(group?.props.footer, /Reconnect/);
    assert.doesNotMatch(group?.props.footer, /no longer/);
    assert.equal(env.control("Refresh plugins").props.disabled, true);
  } finally {
    env.unmount();
  }
});

test("offline connector guidance follows app language changes without refreshing cached data", () => {
  const env = setup({ connected: false, language: "zh-CN" });
  try {
    assert.equal(
      env.all().find((row) => row.type === "OfflineBanner")?.props.subtitle,
      "正在显示缓存的插件。重新联网后可刷新。",
    );
    env.language("zh-TW");
    assert.equal(
      env.all().find((row) => row.type === "OfflineBanner")?.props.subtitle,
      "正在顯示快取的外掛。重新連線後可重新整理。",
    );
    assert.deepEqual(names(env.added()), ["GitHub", "Cloudflare"]);
    assert.equal(
      env.calls.some((call) => call.method.startsWith("refetch-")),
      false,
    );
  } finally {
    env.unmount();
  }
});
