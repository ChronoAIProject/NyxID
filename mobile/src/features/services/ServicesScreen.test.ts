import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { renderSegmentControl } from "../../components/SegmentControl.testHarness";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as connectionPresentation from "./connectionPresentation";
import * as serviceContracts from "./serviceContracts";
import * as designTokens from "../../theme/designTokens";
import type { CatalogEntry, ServiceConnection } from "./serviceContracts";

type Element = { type: unknown; props: Record<string, any> };
type ApiCall = { method: string; args: unknown[] };
class ApiError extends Error {
  errorCode?: number;
}

const renderElement = (type: unknown, props: unknown) =>
  type === renderSegmentControl
    ? renderSegmentControl(
        props as Parameters<typeof renderSegmentControl>[0],
      )
    : { type, props };

function elements(root: Element): Element[] {
  return [
    root,
    // Settings rows carry their icon and inline control outside children.
    ...[root.props.children, root.props.leading, root.props.accessory]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child as Element)
          : [],
      ),
  ];
}

// Groups and rows carry their visible text in props.
const TEXT_PROPS = ["title", "label", "detail", "value", "footer", "message"];
function textContent(root: Element): string {
  return [
    ...TEXT_PROPS.map((name) =>
      typeof root.props[name] === "string" ? `${root.props[name]}\n` : "",
    ),
    root.props.children,
    root.props.accessory,
  ]
    .flat(Infinity)
    .map((child) =>
      child && typeof child === "object" && "props" in child
        ? textContent(child as Element)
        : typeof child === "string" || typeof child === "number"
          ? String(child)
          : "",
    )
    .join("");
}

const connection: ServiceConnection = {
  id: "existing-connection",
  label: "Cloudflare Personal",
  slug: "cloudflare-personal-2",
  status: "active",
  connection_status: "active",
  is_active: true,
  auto_connected: false,
  credential_type: "oauth2",
  credential_source: { type: "personal" },
  catalog_service_slug: "cloudflare",
  catalog_service_name: "Cloudflare",
  api_key_id: "stored-credential",
  granted_scopes: ["account:read", "zone:read", "account:read"],
  proxy_url: "https://nyxid.example/api/v1/proxy/cloudflare-personal-2",
};

const entry: CatalogEntry = {
  slug: "cloudflare",
  name: "Cloudflare",
  description: "Manage your Cloudflare account and zones.",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  service_type: "http",
  provider_type: "oauth2",
  provider_config_id: "cloudflare-provider",
  requires_credential: true,
  requires_gateway_url: false,
  scope_catalog: [
    { scope: "account:read", label: "Read account details", required: true },
    { scope: "zone:read", label: "Read zones" },
    { scope: "zone:write", label: "Edit zones" },
  ],
};

function setup({
  row = connection,
  catalogEntry = entry,
  add = false,
  sharedGrant = false,
}: {
  row?: ServiceConnection;
  catalogEntry?: CatalogEntry;
  add?: boolean;
  /** The first removal reports a provider grant shared with other services. */
  sharedGrant?: boolean;
} = {}) {
  let shared = sharedGrant;
  const calls: ApiCall[] = [];
  const alerts: {
    title: string;
    message: string;
    buttons: { text: string; style?: string; onPress?: () => void }[];
  }[] = [];
  const browsers: string[] = [];
  const copied: string[] = [];
  const queryClient = {
    invalidateQueries: async () => {},
    setQueryData: () => {},
  };
  const navigation = { setOptions: () => {} };
  const record =
    (method: string, result?: unknown) =>
    async (...args: unknown[]) => {
      calls.push({ method, args });
      return result;
    };
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: renderElement,
      jsxs: renderElement,
    },
    "../../components/SegmentControl": {
      SegmentControl: renderSegmentControl,
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove: () => {} }),
      },
      Keyboard: { dismiss: () => {} },
      Linking: { openURL: async () => {} },
      Alert: {
        alert: (title: string, message: string, buttons: never) =>
          alerts.push({ title, message, buttons }),
      },
      ...Object.fromEntries(
        [
          "ActivityIndicator",
          "Pressable",
          "Switch",
          "Text",
          "TextInput",
          "View",
        ].map((name) => [name, name]),
      ),
      StyleSheet: { create: (styles: unknown) => styles, hairlineWidth: 0.5 },
    },
    "@react-navigation/native": {
      useIsFocused: () => true,
      useNavigation: () => navigation,
    },
    "@tanstack/react-query": {
      useQueryClient: () => queryClient,
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        isPending: false,
        isFetching: false,
        isError: false,
        refetch: async () => {},
        data:
          queryKey[2] === "connections"
            ? [row]
            : queryKey[2] === "catalog"
              ? [catalogEntry]
              : queryKey[2] === "connection"
                ? row
                : catalogEntry,
      }),
    },
    "expo-web-browser": {
      openBrowserAsync: async (url: string) => {
        browsers.push(url);
        return { type: "dismiss" };
      },
    },
    "expo-clipboard": {
      setStringAsync: async (value: string) => {
        copied.push(value);
      },
    },
    "lucide-react-native": Object.fromEntries(
      ["ChevronRight", "ChevronDown", "KeyRound", "PlugZap", "ShieldCheck"].map(
        (name) => [name, name],
      ),
    ),
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["Notice", "WorkspacePage"].map((name) => [name, name]),
    ),
    "../../components/settings/SettingsList": Object.fromEntries(
      ["SettingsGroup", "SettingsRow", "SettingsTextField"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../lib/api/ApiError": { ApiError },
    "../../lib/api/servicesApi": {
      servicesApi: {
        initiateOAuth: record("initiateOAuth", {
          authorization_url: "https://provider.example/authorize?state=bound",
        }),
        connection: async (id: string) => {
          calls.push({ method: "connection", args: [id] });
          return {
            ...row,
            id,
            status: "active",
            last_authorized_at: "2026-10-15T06:00:00Z",
          };
        },
        create: record("create", { ...row, id: "created-connection" }),
        setActive: record("setActive"),
        replaceCredential: record("replaceCredential"),
        remove: async (...args: unknown[]) => {
          calls.push({ method: "remove", args });
          if (shared) {
            shared = false;
            throw Object.assign(new ApiError("shared"), { errorCode: 11500 });
          }
          return { deleted: true };
        },
      },
    },
    "../../lib/env": { getFrontendUrl: () => "https://nyxid.example" },
    "../../app/WorkspaceHeader": { useWorkspaceBack: () => {} },
    "../../theme/designTokens": designTokens,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./serviceContracts": serviceContracts,
    "./connectionPresentation": connectionPresentation,
    "../plugins/PluginConnectionHistory": {
      PluginConnectionHistory: "PluginConnectionHistory",
    },
  });
  const { ServicesScreen } = harness.load(
    fileURLToPath(new URL("./ServicesScreen.tsx", import.meta.url)),
  ) as { ServicesScreen: (props: unknown) => Element };
  harness.mount(() =>
    ServicesScreen(
      add
        ? { initialCatalogSlug: catalogEntry.slug }
        : { initialConnectionId: row.id },
    ),
  );
  const find = (predicate: (item: Element) => boolean) => {
    const item = elements(harness.flush()).find(predicate);
    assert.ok(item, "Expected a matching control in the rendered screen");
    return item;
  };
  // Actions are tinted settings rows; their label is the visible text.
  const isAction = (item: Element, label: string) =>
    item.type === "SettingsRow" &&
    item.props.label === label &&
    typeof item.props.onPress === "function";
  const button = (label: string) => find((item) => isAction(item, label));
  return {
    harness,
    calls,
    alerts,
    browsers,
    copied,
    button,
    find,
    text: () => textContent(harness.flush()),
    hasButton: (label: string) =>
      elements(harness.flush()).some((item) => isAction(item, label)),
    header: (label: string) =>
      find((item) => item.props.accessibilityLabel === label),
    tab: (label: string) =>
      find(
        (item) =>
          item.props.accessibilityRole === "tab" &&
          item.props.accessibilityLabel === label,
      ).props.onPress(),
  };
}

test("a healthy OAuth connection explains provider access without asking to reauthorize", () => {
  const env = setup();
  assert.equal(env.hasButton("Reconnect"), false);
  assert.equal(env.hasButton("Continue authentication"), false);
  assert.equal(env.hasButton("Authorize again"), false);
  assert.match(env.text(), /2 granted permissions/);
  assert.match(env.text(), /Service permissions/);
  assert.match(env.text(), /does not decide which agents/);
  const permissions = env.header("Provider permissions");
  assert.equal(permissions.props.accessibilityState.expanded, false);
  permissions.props.onPress();
  assert.equal(
    env.header("Provider permissions").props.accessibilityState.expanded,
    true,
  );
  assert.match(env.text(), /Read account details/);
  assert.match(env.text(), /Read zones/);
  assert.doesNotMatch(env.text(), /Edit zones/);
  assert.equal(env.hasButton("Manage permissions in Web"), true);
  assert.deepEqual(env.calls, []);
  env.harness.unmount();
});

test("failed and missing OAuth credentials offer recovery even with active connection metadata", () => {
  for (const override of [
    { status: "failed" },
    { status: "refresh_failed" },
    { credential_missing: true },
  ]) {
    const env = setup({ row: { ...connection, ...override } });
    assert.equal(env.hasButton("Reconnect"), true);
    assert.equal(env.hasButton("Continue authentication"), false);
    assert.notEqual(
      env.find(
        (item) => item.type === "SettingsRow" && item.props.label === "Status",
      ).props.value,
      "Connected",
    );
    assert.deepEqual(env.calls, []);
    env.harness.unmount();
  }
  const pending = setup({ row: { ...connection, status: "pending_auth" } });
  assert.equal(pending.hasButton("Continue authentication"), true);
  assert.equal(pending.hasButton("Reconnect"), false);
  pending.harness.unmount();
});

test("readonly and platform connections cannot launch credential recovery", () => {
  for (const override of [
    { can_edit_configuration: false },
    { auto_connected: true },
    { credential_binding: "platform" as const },
    {
      credential_source: {
        type: "org" as const,
        org_id: "org-1",
        org_name: "Team",
        role: "viewer" as const,
        allowed: true,
      },
    },
  ]) {
    const env = setup({
      row: { ...connection, status: "failed", ...override },
    });
    assert.equal(env.hasButton("Reconnect"), false);
    assert.equal(env.hasButton("Continue authentication"), false);
    assert.equal(env.hasButton("Repair connection in Web"), false);
    assert.deepEqual(env.calls, []);
    env.harness.unmount();
  }
});

test("repair authorizes the existing organization connection and preserves its grant", async () => {
  const env = setup({
    row: {
      ...connection,
      status: "failed",
      granted_scopes: ["custom:legacy", "zone:read", "zone:read"],
      credential_source: {
        type: "org",
        org_id: "org-1",
        org_name: "Team",
        role: "admin",
        allowed: true,
      },
    },
  });
  env.button("Reconnect").props.onPress();
  await env.harness.settle();
  assert.deepEqual(env.calls, [
    {
      method: "initiateOAuth",
      args: [
        "cloudflare-provider",
        "existing-connection",
        "org-1",
        ["custom:legacy", "zone:read", "account:read"],
      ],
    },
    { method: "connection", args: ["existing-connection"] },
  ]);
  assert.deepEqual(env.browsers, [
    "https://provider.example/authorize?state=bound",
  ]);
  assert.ok(
    elements(env.harness.flush()).some(
      (item) =>
        item.type === "Notice" &&
        item.props.message ===
          "Authorization completed. Your connection is ready.",
    ),
  );
  env.harness.unmount();
});

test("a missing credential uses Web repair instead of initiating OAuth on a missing key", async () => {
  const env = setup({ row: { ...connection, credential_missing: true } });
  env.button("Reconnect").props.onPress();
  await env.harness.settle();
  assert.deepEqual(env.browsers, [
    "https://nyxid.example/keys/existing-connection?tab=overview",
  ]);
  assert.deepEqual(env.calls, []);
  env.harness.unmount();
});

test("new OAuth setup still creates first and leaves provider scope defaults untouched", async () => {
  const env = setup({ add: true });
  const form = env.find((item) => typeof item.type === "function");
  form.props.onSubmit({
    label: "  New Cloudflare  ",
    credential: "",
    endpointUrl: "",
    fields: {},
    platform: false,
  });
  await env.harness.settle();
  assert.equal(env.calls[0]?.method, "create");
  assert.deepEqual(JSON.parse(JSON.stringify(env.calls[0]?.args)), [
    { service_slug: "cloudflare", label: "New Cloudflare" },
  ]);
  assert.deepEqual(env.calls[1], {
    method: "initiateOAuth",
    args: ["cloudflare-provider", "created-connection", undefined, undefined],
  });
  env.harness.unmount();
});

test("a no-auth service omits provider permission controls", () => {
  const env = setup({
    row: { ...connection, credential_type: "none", granted_scopes: [] },
    catalogEntry: { ...entry, provider_type: null, requires_credential: false },
  });
  assert.equal(
    elements(env.harness.flush()).some(
      (item) => item.props.accessibilityLabel === "Provider permissions",
    ),
    false,
  );
  assert.doesNotMatch(env.text(), /Last authorized/);
  assert.equal(env.hasButton("Reconnect"), false);
  env.harness.unmount();
});

test("paused and restricted connections explain their access prerequisite before Chat", () => {
  const paused = setup({ row: { ...connection, is_active: false } });
  assert.match(
    paused.text(),
    /Turn on Connection enabled above before using it in Chat/,
  );
  assert.equal(paused.header("Connection enabled").props.value, false);
  assert.deepEqual(paused.calls, []);
  paused.harness.unmount();

  const restricted = setup({
    row: {
      ...connection,
      credential_source: {
        type: "org",
        org_id: "org-1",
        org_name: "Team",
        role: "viewer",
        allowed: false,
      },
    },
  });
  assert.match(
    restricted.text(),
    /Ask an organization admin to grant you access/,
  );
  assert.doesNotMatch(restricted.text(), /Select this connection and save/);
  assert.deepEqual(restricted.calls, []);
  restricted.harness.unmount();
});

test("developer and destructive actions stay in Advanced and passive navigation never writes", async () => {
  const env = setup({
    row: {
      ...connection,
      credential_type: "bearer",
      granted_scopes: [],
    },
    catalogEntry: { ...entry, provider_type: "api_key" },
  });
  assert.equal(env.hasButton("Copy proxy address"), false);
  assert.equal(env.hasButton("Remove connection"), false);
  env.tab("Advanced");
  env.button("Copy proxy address").props.onPress();
  await env.harness.settle();
  assert.deepEqual(env.copied, [connection.proxy_url]);
  env.header("Replace API key or credential").props.onPress();
  const field = env.find(
    (item) =>
      item.type === "SettingsTextField" &&
      item.props.accessibilityLabel === "New credential",
  );
  assert.equal(field.props.secureTextEntry, true);
  field.props.onChangeText("temporary-credential-draft");
  const remove = env.button("Remove connection");
  assert.equal(remove.props.centered, true);
  assert.equal(remove.props.destructive, true);
  remove.props.onPress();
  assert.equal(env.alerts.length, 1, "removal asks for confirmation first");
  assert.match(env.alerts[0]!.title, /Remove “Cloudflare Personal”\?/);
  assert.deepEqual(
    JSON.parse(
      JSON.stringify(env.alerts[0]!.buttons.map((item) => [item.text, item.style])),
    ),
    [
      ["Cancel", "cancel"],
      ["Remove", "destructive"],
    ],
  );
  env.alerts[0]!.buttons[0]!.onPress!();
  await env.harness.settle();
  env.tab("Overview");
  assert.equal(env.hasButton("Copy proxy address"), false);
  assert.equal(env.hasButton("Remove connection"), false);
  env.tab("Advanced");
  assert.equal(
    env.header("Replace API key or credential").props.accessibilityState
      .expanded,
    false,
  );
  assert.equal(env.hasButton("Save new credential"), false);
  assert.deepEqual(env.calls, []);
  env.harness.unmount();
});

test("confirmed removal deletes the connection; a shared grant switches to removing only this service", async () => {
  const env = setup({
    row: { ...connection, credential_type: "bearer", granted_scopes: [] },
    catalogEntry: { ...entry, provider_type: "api_key" },
    sharedGrant: true,
  });
  env.tab("Advanced");
  env.button("Remove connection").props.onPress();
  env.alerts.at(-1)!.buttons[1]!.onPress!();
  await env.harness.settle();
  assert.deepEqual(env.calls.at(-1), {
    method: "remove",
    args: ["existing-connection", false],
  });
  assert.match(env.text(), /remove only this service below/);
  env.button("Remove only this service").props.onPress();
  const tokenOnly = env.alerts.at(-1)!;
  assert.equal(tokenOnly.title, "Remove only this service?");
  assert.equal(tokenOnly.buttons[1]!.text, "Remove only this service");
  tokenOnly.buttons[1]!.onPress!();
  await env.harness.settle();
  assert.deepEqual(env.calls.at(-1), {
    method: "remove",
    args: ["existing-connection", true],
  });
  env.harness.unmount();
});

test("overview reads as label/value rows with the enable switch inline", () => {
  const env = setup();
  const row = (label: string) =>
    env.find((item) => item.type === "SettingsRow" && item.props.label === label);
  assert.equal(row("Status").props.value, "Connected");
  assert.equal(row("Last used").props.value, "Never");
  assert.equal(row("Granted permissions").props.value, "2 granted permissions");
  assert.equal(row("Connection enabled").props.accessory.type, "Switch");
  assert.equal(row("Refresh status").props.tone, "action");
  assert.deepEqual(env.calls, []);
  env.harness.unmount();
});
