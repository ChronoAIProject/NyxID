import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as designTokens from "../../theme/designTokens";
import * as historyTypes from "./pluginConnectionHistoryTypes";

type Element = { type: unknown; props: Record<string, any> };
type QueryOptions = {
  queryKey: unknown[];
  enabled: boolean;
  queryFn: (context: {
    signal?: AbortSignal;
    pageParam?: string | null;
  }) => Promise<unknown>;
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

function setup({ groups = [] as unknown[] } = {}) {
  let serviceId = "connection-a";
  const pushes: { name: string; params: any }[] = [];
  let epoch = 1;
  let focused = true;
  let appStateChange: (state: string) => void = () => {};
  let history!: QueryOptions;
  let options!: QueryOptions;
  const calls: { method: string; args: unknown[] }[] = [];
  const actions = {
    data: [
      { value: "updated", label: "Updated" },
      { value: "created", label: "Created" },
      { value: "retired", label: "Retired", disabled: true },
    ],
    isPending: false,
    isError: false,
    refetch: () => {
      calls.push({ method: "retry-actions", args: [] });
    },
  };
  const record =
    (method: string) =>
    async (...args: unknown[]) => {
      calls.push({ method, args });
      return {};
    };
  class ApiError extends Error {}
  const harness = hookHarness<Element>({
    "react-native": {
      ...Object.fromEntries(
        ["ActivityIndicator", "Pressable", "Text", "View"].map((name) => [
          name,
          name,
        ]),
      ),
      AppState: {
        currentState: "active",
        addEventListener: (_: string, callback: typeof appStateChange) => {
          appStateChange = callback;
          return { remove: () => {} };
        },
      },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "@react-navigation/native": {
      useIsFocused: () => focused,
      useNavigation: () => ({
        push: (name: string, params: unknown) => pushes.push({ name, params }),
      }),
    },
    "@tanstack/react-query": {
      useQueryClient: () => ({ cancelQueries: record("cancel") }),
      useInfiniteQuery: (value: QueryOptions) => {
        history = value;
        return {
          data: { pages: [{ groups }] },
          isPending: false,
          isError: false,
        };
      },
      useQuery: (value: QueryOptions) => {
        options = value;
        return actions;
      },
    },
    "lucide-react-native": Object.fromEntries(
      ["ChevronsUpDown", "SlidersHorizontal"].map((name) => [name, name]),
    ),
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsRow: "SettingsRow",
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Notice", "WorkspacePage"].map((name) => [name, name]),
    ),
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionSheet: "WorkspaceSelectionSheet",
    },
    "../../lib/api/ApiError": { ApiError },
    "../../lib/api/pluginConnectionHistoryApi": {
      pluginConnectionHistoryApi: {
        page: record("page"),
        actions: record("actions"),
      },
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../theme/designTokens": designTokens,
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./pluginConnectionHistoryTypes": historyTypes,
  });
  const module = harness.load(
    fileURLToPath(new URL("./PluginConnectionHistory.tsx", import.meta.url)),
  ) as {
    PluginConnectionHistory: (props: { serviceId: string }) => Element;
    ConnectionHistoryDetailScreen: (props: unknown) => Element;
  };
  const { PluginConnectionHistory } = module;
  harness.mount(() => PluginConnectionHistory({ serviceId }));
  const find = (predicate: (item: Element) => boolean) => {
    const result = elements(harness.flush()).find(predicate);
    assert.ok(result, "Expected history control");
    return result;
  };
  return {
    harness,
    module,
    pushes,
    all: () => elements(harness.flush()),
    calls,
    actions,
    history: () => history,
    options: () => options,
    sheet: () => find((item) => item.type === "WorkspaceSelectionSheet").props,
    hasSheet: () =>
      elements(harness.flush()).some(
        (item) => item.type === "WorkspaceSelectionSheet",
      ),
    control: (label: string) =>
      find(
        (item) =>
          item.props.accessibilityLabel === label || item.props.label === label,
      ),
    open: () => {
      find(
        (item) => item.props.accessibilityLabel === "Filter history actions",
      ).props.onPress();
      harness.flush();
    },
    service: (value: string) => {
      serviceId = value;
      harness.render();
    },
    epoch: (value: number) => {
      epoch = value;
      harness.render();
    },
    focused: (value: boolean) => {
      focused = value;
      harness.render();
    },
    foreground: (value: boolean) => {
      appStateChange(value ? "active" : "background");
      harness.flush();
    },
    unmount: () => harness.unmount(),
  };
}

test("history filters load on demand and only Apply changes the paged history request", async () => {
  const env = setup();
  try {
    assert.equal(env.options().enabled, false);
    env.open();
    assert.equal(env.options().enabled, true);
    const cancel = env.sheet();
    assert.equal(cancel.multiple, true);
    assert.equal(cancel.allowEmpty, true);
    assert.equal(cancel.options[2].disabled, true);
    cancel.onClose();
    env.harness.flush();
    assert.equal(env.hasSheet(), false);
    assert.equal(env.options().enabled, false);
    assert.deepEqual(Array.from(env.history().queryKey[5] as string[]), []);
    env.open();
    const apply = env.sheet();
    apply.onChange(["updated", "created"]);
    apply.onClose();
    env.harness.flush();
    assert.deepEqual(Array.from(env.history().queryKey[5] as string[]), [
      "created",
      "updated",
    ]);
    const signal = new AbortController().signal;
    await env.history().queryFn({ pageParam: "cursor-2", signal });
    const call = env.calls.find((item) => item.method === "page")!;
    assert.equal(call.args[0], "connection-a");
    assert.equal(call.args[1], "cursor-2");
    assert.deepEqual(Array.from(call.args[2] as string[]), [
      "created",
      "updated",
    ]);
    assert.equal(call.args[3], signal);
    env.open();
    const clear = env.sheet();
    clear.onChange([]);
    clear.onClose();
    env.harness.flush();
    assert.deepEqual(Array.from(env.history().queryKey[5] as string[]), []);
  } finally {
    env.unmount();
  }
});

test("history sheet keeps loading and Retry visible while blocking incomplete options", () => {
  const env = setup();
  try {
    env.actions.isPending = true;
    env.open();
    assert.equal(env.sheet().readOnly, true);
    assert.equal(env.control("Loading history filters").type, "SkeletonRows");
    env.actions.isPending = false;
    env.actions.isError = true;
    env.harness.render();
    assert.equal(env.sheet().readOnly, true);
    assert.equal(env.sheet().options.length, 0);
    assert.equal(env.control("Retry filters").props.disabled, false);
    env.control("Retry filters").props.onPress();
    assert.equal(
      env.calls.filter((item) => item.method === "retry-actions").length,
      1,
    );
    env.actions.isError = false;
    env.harness.render();
    assert.equal(env.sheet().readOnly, false);
    assert.equal(env.sheet().options.length, 3);
  } finally {
    env.unmount();
  }
});

test("connection and account changes clear applied filters and any open draft; blur/background dismiss drafts", () => {
  const env = setup();
  try {
    env.open();
    const apply = env.sheet();
    apply.onChange(["created"]);
    apply.onClose();
    env.harness.flush();
    env.open();
    const previousScope = env.sheet().scopeKey;
    env.service("connection-b");
    assert.equal(env.hasSheet(), false);
    assert.deepEqual(Array.from(env.history().queryKey[5] as string[]), []);
    env.open();
    assert.notEqual(env.sheet().scopeKey, previousScope);
    env.epoch(2);
    assert.equal(env.hasSheet(), false);
    assert.deepEqual(Array.from(env.history().queryKey[5] as string[]), []);
    env.open();
    env.focused(false);
    assert.equal(env.hasSheet(), false);
    assert.equal(env.options().enabled, false);
    env.focused(true);
    env.open();
    env.foreground(false);
    assert.equal(env.hasSheet(), false);
    assert.equal(env.history().enabled, false);
    assert.equal(env.options().enabled, false);
  } finally {
    env.unmount();
  }
});

const event = (overrides: Record<string, unknown> = {}) => ({
  id: "event-1",
  schema_version: 1,
  action: "service.updated",
  action_label: "Connection updated",
  actor: {
    kind: "api_key",
    id: "actor-1",
    name: "Deploy key",
    person_id: null,
    api_key_id: "key-1",
    app_id: null,
  },
  committed_at: "2026-10-30T10:00:00Z",
  additional_changes: true,
  audit_status: "mismatch",
  changes: [{ field: "is_active", label: "Enabled", before: true, after: false }],
  ...overrides,
});

test("history lists change sets as rows that open a metadata-only detail page", () => {
  const group = { id: "group-1", events: [event(), event({ id: "event-2" })] };
  const env = setup({ groups: [group, { id: "empty", events: [] }] });
  try {
    const rows = env
      .all()
      .filter(
        (item) => item.type === "SettingsRow" && item.props.label === "Connection updated",
      );
    assert.equal(rows.length, 1, "empty groups never render");
    assert.equal(rows[0]!.props.accessibilityLabel, "Connection updated, Deploy key (API key)");
    assert.match(rows[0]!.props.detail, /Deploy key \(API key\) · .* · 2 changes/);
    rows[0]!.props.onPress();
    assert.equal(env.pushes.length, 1);
    assert.equal(env.pushes[0]!.name, "ConnectionHistoryDetail");
    assert.equal(env.pushes[0]!.params.title, "Connection updated");
    assert.equal(env.pushes[0]!.params.group, group);
    assert.equal(
      env.calls.filter((call) => call.method !== "cancel").length,
      0,
      "opening a change reads nothing new",
    );

    const detail = env.module.ConnectionHistoryDetailScreen({
      route: { params: env.pushes[0]!.params },
    });
    const nodes = elements(detail);
    const value = (label: string) =>
      nodes.filter((item) => item.props.label === label).map((item) => item.props.value);
    assert.deepEqual(value("Changed by"), ["Deploy key (API key)", "Deploy key (API key)"]);
    assert.deepEqual(value("Authenticated"), ["API key", "API key"]);
    assert.deepEqual(value("Enabled"), ["Yes → No", "Yes → No"]);
    assert.equal(
      nodes.filter((item) => item.type === "Notice" && item.props.tone === "error").length,
      2,
    );
    assert.ok(
      nodes.some((item) =>
        String(item.props.footer).startsWith("Other service settings updated."),
      ),
    );
  } finally {
    env.unmount();
  }
});
