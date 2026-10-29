import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";

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

function setup() {
  let serviceId = "connection-a";
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
        ["ActivityIndicator", "Pressable", "View"].map((name) => [name, name]),
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
    "@react-navigation/native": { useIsFocused: () => focused },
    "@tanstack/react-query": {
      useQueryClient: () => ({ cancelQueries: record("cancel") }),
      useInfiniteQuery: (value: QueryOptions) => {
        history = value;
        return {
          data: { pages: [{ groups: [] }] },
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
      ["ChevronDown", "ChevronRight", "SlidersHorizontal"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Label", "Notice"].map((name) => [name, name]),
    ),
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
    "../../theme/designTokens": { fonts: {} },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./pluginConnectionHistoryTypes": {},
  });
  const { PluginConnectionHistory } = harness.load(
    fileURLToPath(new URL("./PluginConnectionHistory.tsx", import.meta.url)),
  ) as {
    PluginConnectionHistory: (props: { serviceId: string }) => Element;
  };
  harness.mount(() => PluginConnectionHistory({ serviceId }));
  const find = (predicate: (item: Element) => boolean) => {
    const result = elements(harness.flush()).find(predicate);
    assert.ok(result, "Expected history control");
    return result;
  };
  return {
    harness,
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
    env.control("Loading history filters");
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
