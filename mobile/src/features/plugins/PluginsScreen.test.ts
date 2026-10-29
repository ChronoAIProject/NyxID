import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { deriveConnectorItems, matchesPluginQuery } from "./pluginConnectors";
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
  const children = [root.props.children];
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
}: {
  connections?: ServiceConnection[];
  catalog?: CatalogEntry[];
} = {}) {
  const calls: { method: string; args: unknown[] }[] = [];
  const queries = new Map<string, QueryOptions>();
  let back: { active: boolean; onBack: () => void; defaultRole?: string };
  let focused = true;
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
  const harness = hookHarness<Element>({
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
      StyleSheet: { create: (styles: unknown) => styles },
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
      ["ArrowRight", "RefreshCw", "Search", "X"].map((name) => [name, name]),
    ),
    "../../app/WorkspaceHeader": {
      useOpenWorkspaceChat: () => openChat,
      useWorkspaceBack: (options: typeof back) => {
        back = options;
      },
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Label", "Notice", "WorkspacePage"].map(
        (name) => [name, name],
      ),
    ),
    "../../lib/api/servicesApi": {
      servicesApi: {
        pluginConnections: record("pluginConnections"),
        catalog: record("catalog"),
      },
    },
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../services/serviceContracts": {
      connectionOwner: (connection: ServiceConnection) =>
        connection.credential_source?.type === "org" ? "Work" : "Personal",
    },
    "./pluginConnectors": { deriveConnectorItems, matchesPluginQuery },
    "./PluginSkills": { PluginSkills: "PluginSkills" },
  });
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
    list: () => find((item) => item.type === "FlatList").props,
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
    foreground: (value: boolean) => {
      foreground(value ? "active" : "background");
      harness.flush();
    },
    unmount: () => harness.unmount(),
  };
}

test("automatic credentials cannot enter the mobile management picker or counts", () => {
  const env = setup({
    connections: [
      { ...personal, id: "auto-github", auto_connected: true },
      { ...dnsPersonal, is_active: false },
      { ...dnsWork, id: "auto-dns", auto_connected: true },
    ],
  });
  try {
    assert.equal(env.list().data.length, 1);
    env.control("Added connectors, 1");
    assert.deepEqual(Array.from(env.list().data[0].manageKeyIds), [dnsPersonal.id]);
    assert.equal(env.list().data[0].disabled, true);
    env.connector("service:api-cloudflare");
    assert.ok(env.control("Manage Personal Cloudflare"));
    assert.ok(!env.all().some((node) => node.props.accessibilityLabel === "Manage Work Cloudflare"));
    env.connector();
    env.control("Discover connectors, 1").props.onPress();
    assert.deepEqual(Array.from(env.list().data, (item: { name: string }) => item.name), ["Slack"]);
  } finally {
    env.unmount();
  }
});

test("Connectors defaults to Added and Discover displays only available services", () => {
  const env = setup();
  try {
    assert.equal(env.list().accessibilityLabel, "Added connectors");
    assert.deepEqual(
      env
        .all()
        .filter((node) => node.type === "ServiceIcon")
        .map((node) => node.props.slug),
      ["api-github", "api-cloudflare"],
    );
    assert.deepEqual(
      Array.from(env.list().data, (item: { name: string }) => item.name),
      ["GitHub", "Cloudflare"],
    );
    env.control("Discover connectors, 1").props.onPress();
    assert.equal(env.list().accessibilityLabel, "Discover connectors");
    assert.equal(
      env.all().find((node) => node.type === "ServiceIcon")?.props.slug,
      "api-slack",
    );
    assert.deepEqual(
      Array.from(env.list().data, (item: { name: string }) => item.name),
      ["Slack"],
    );
    env.control("Added connectors, 2").props.onPress();
    assert.equal(env.list().data.length, 2);
    assert.equal(
      env.calls.some((call) => call.method === "push"),
      false,
    );
  } finally {
    env.unmount();
  }
});

test("Added and Discover search keep independent inputs and filter their own list", () => {
  const env = setup();
  try {
    env.control("Search added connectors").props.onChangeText("DNS");
    assert.deepEqual(
      Array.from(env.list().data, (item: { name: string }) => item.name),
      ["Cloudflare"],
    );
    env.control("Discover connectors, 1").props.onPress();
    assert.equal(env.control("Search connector catalog").props.value, "");
    env.control("Search connector catalog").props.onChangeText("no-match");
    assert.equal(env.list().data.length, 0);
    env.control("Added connectors, 2").props.onPress();
    assert.equal(env.control("Search added connectors").props.value, "DNS");
    assert.equal(env.list().data[0].name, "Cloudflare");
    env.control("Discover connectors, 1").props.onPress();
    assert.equal(
      env.control("Search connector catalog").props.value,
      "no-match",
    );
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
    env.control("Discover connectors, 1").props.onPress();
    env.control("Connect Slack, Available to add").props.onPress();
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

test("an empty Added library offers discovery without automatically connecting a service", () => {
  const env = setup({ connections: [] });
  try {
    assert.equal(env.list().data.length, 0);
    env.control("Discover connectors").props.onPress();
    assert.equal(env.list().accessibilityLabel, "Discover connectors");
    assert.equal(env.list().data.length, 3);
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
      const discover = env
        .all()
        .find(
          (item) =>
            item.props.accessibilityRole === "tab" &&
            String(item.props.accessibilityLabel).startsWith(
              "Discover connectors",
            ),
        );
      assert.ok(discover);
      discover.props.onPress();
      assert.equal(env.list().data.length, 0);
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
    assert.equal(env.list().data[0].status, "Disabled");
    env.control("Manage GitHub, Disabled").props.onPress();
    assert.equal(env.calls.filter((call) => call.method === "push").length, 1);
    env.control("Discover connectors, 2").props.onPress();
    assert.equal(
      env.list().data.some((item: { name: string }) => item.name === "GitHub"),
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
      env.all().some((item) => item.type === "FlatList"),
      false,
    );
    env.tab("Connectors").props.onPress();
    assert.equal(env.list().accessibilityLabel, "Added connectors");
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
