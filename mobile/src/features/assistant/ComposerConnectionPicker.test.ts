import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import { serviceConnectionSchema } from "../services/serviceContracts";
import {
  composerConnectionAvailable,
  composerConnectionGroups,
} from "./composerConnection";

const key = (id: string, extra: any = {}) =>
  serviceConnectionSchema.parse({
    id,
    label: `Account ${id}`,
    slug: `github-${id}`,
    status: "active",
    credential_type: "api_key",
    auto_connected: false,
    is_active: true,
    catalog_service_id: "github",
    catalog_service_name: "GitHub",
    ...extra,
  });
const nodes = (n: any): any[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const plain = (v: any) => JSON.parse(JSON.stringify(v));
function setup() {
  let scope = "thread-a",
    epoch = 1,
    locked = false,
    active = true;
  const query = {
    data: [
      key("a"),
      key("b", { catalog_service_id: "other", catalog_service_name: "Linear" }),
    ],
    isPending: false,
    isError: false,
    refetch: () => retries++,
  };
  let retries = 0;
  const h = hookHarness<any>({
    "@tanstack/react-query": { useQuery: () => query },
    "../../lib/api/servicesApi": { servicesApi: {} },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../groups/useGroupRuntime": {
      useGroupAvailability: () => ({ active, available: () => active }),
    },
  });
  const use = h.load(
    fileURLToPath(new URL("./useComposerConnection.ts", import.meta.url)),
  ).useComposerConnection as any;
  h.mount(() => use(scope, locked));
  return {
    h,
    query,
    retries: () => retries,
    scope: (s: string) => {
      scope = s;
      h.render();
    },
    account: () => {
      epoch++;
      h.render();
    },
    lock: () => {
      locked = true;
      h.render();
    },
    offline: () => {
      active = false;
      h.render();
    },
  };
}
test("connection filtering/grouping matches web enabled/organization authority and name/owner search", () => {
  const org = {
    type: "org",
    org_id: "org",
    org_name: "Research",
    role: "member",
    allowed: true,
  };
  const rows = [
    key("a"),
    key("b", { credential_source: org }),
    key("c", { is_active: false }),
    key("d", { credential_source: { ...org, allowed: false } }),
  ];
  assert.deepEqual(
    rows.filter(composerConnectionAvailable).map((row) => row.id),
    ["a", "b"],
  );
  assert.equal(
    composerConnectionGroups(rows.slice(0, 2), "research")[0]!.connections[0]!
      .id,
    "b",
  );
  assert.equal(composerConnectionGroups(rows.slice(0, 2), "github").length, 1);
  assert.equal(
    composerConnectionGroups(rows.slice(0, 2), "personal")[0]!.connections[0]!
      .id,
    "a",
  );
  assert.equal(composerConnectionGroups(rows, "missing").length, 0);
  assert.deepEqual(
    composerConnectionGroups(
      [key("z", { catalog_service_id: null, label: "Zebra" }), key("a")],
      "",
    ).map((g) => g.name),
    ["GitHub", "Zebra"],
  );
});
test("real connection hook picks one message account, prefixes exact text and clears only after send", () => {
  const x = setup();
  x.h.value().choose();
  x.h.flush();
  assert.equal(x.h.value().open, true);
  x.h.value().select("a");
  x.h.flush();
  assert.equal(x.h.value().open, false);
  assert.equal(x.h.value().prefix("Hello\nWorld"), "/service a\nHello\nWorld");
  const completed = x.h.value().sent;
  x.h.value().select("b");
  x.h.flush();
  completed();
  x.h.flush();
  assert.equal(
    x.h.value().connection.id,
    "b",
    "old send cannot consume a later selection",
  );
  x.h.value().sent();
  x.h.flush();
  assert.equal(x.h.value().connection, undefined);
  assert.equal(x.h.value().prefix("plain"), "plain");
  x.h.unmount();
});
test("a revoked/disabled/unreadable key remains visible as unavailable and blocks submission until removed", () => {
  for (const error of [false, true]) {
    const x = setup();
    x.h.value().select("a");
    x.h.flush();
    if (error) x.query.isError = true;
    else x.query.data = [];
    x.h.render();
    assert.equal(x.h.value().connection.id, "a");
    assert.equal(x.h.value().unavailable, true);
    assert.throws(
      () => x.h.value().prefix("text"),
      /selected connection is unavailable/,
    );
    x.h.value().remove();
    x.h.flush();
    assert.equal(x.h.value().prefix("text"), "text");
    x.h.unmount();
  }
});
test("account/thread/draft scope changes clear selection and fence retained menu actions", () => {
  for (const kind of ["scope", "account"]) {
    const x = setup();
    x.h.value().select("a");
    x.h.flush();
    const old = x.h.value();
    if (kind === "scope") x.scope("new-draft");
    else x.account();
    old.choose();
    old.select("a");
    x.h.flush();
    assert.equal(x.h.value().connection, undefined);
    assert.equal(x.h.value().open, false);
    assert.throws(() => old.prefix("draft"), /unavailable/);
    x.scope("thread-a");
    assert.equal(x.h.value().connection, undefined);
    x.h.unmount();
  }
});
test("active/sending/disabled/offline locks prevent selection or removal without losing its chip", () => {
  for (const offline of [false, true]) {
    const x = setup();
    x.h.value().select("a");
    x.h.flush();
    const old = x.h.value();
    if (offline) x.offline();
    else x.lock();
    old.choose();
    old.remove();
    old.select("b");
    x.h.flush();
    assert.equal(x.h.value().connection.id, "a");
    assert.equal(x.h.value().open, false);
    assert.equal(x.h.value().locked, true);
    x.h.unmount();
  }
});
test("real native picker shows web copy, search, selected rows, unavailable chip, removal and explicit retry", () => {
  const calls: any[] = [];
  const state: any = {
    connection: key("a"),
    connections: [key("a"), key("b")],
    unavailable: false,
    open: true,
    loading: false,
    error: false,
    locked: false,
    close: () => calls.push(["close"]),
    select: (id: string) => calls.push(["select", id]),
    remove: () => calls.push(["remove"]),
    retry: () => calls.push(["retry"]),
  };
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      Modal: "Modal",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      View: "View",
    },
    "lucide-react-native": { Check: "Check", Link2: "Link2", X: "X" },
    "../../components/workspace/WorkspaceUI": {
      Label: "Label",
      Field: "Field",
    },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const C = h.load(
    fileURLToPath(new URL("./ComposerConnectionPicker.tsx", import.meta.url)),
  ).ComposerConnectionPicker as any;
  h.mount(() => C({ state }));
  const find = (label: string) =>
    nodes(h.value()).find((n) => n.props.accessibilityLabel === label)!;
  assert.equal(
    find("Account a, Personal, github-a").props.accessibilityState.selected,
    true,
  );
  assert.equal(find("Account a, Personal, github-a").props.style.minHeight, 44);
  find("Account b, Personal, github-b").props.onPress();
  assert.deepEqual(calls.at(-1), ["select", "b"]);
  nodes(h.value())
    .find((n) => n.type === "Field")
    .props.onChangeText("no match");
  h.flush();
  assert.ok(
    nodes(h.value()).some(
      (n) => n.props.children === "No matching connections.",
    ),
  );
  state.unavailable = true;
  h.render();
  assert.ok(
    nodes(h.value()).some((n) =>
      JSON.stringify(n.props.children).includes(
        "Unavailable — choose another connection or remove",
      ),
    ),
  );
  find("Remove connection for this message").props.onPress();
  assert.deepEqual(calls.at(-1), ["remove"]);
  state.error = true;
  h.render();
  find("Retry").props.onPress();
  assert.deepEqual(calls.at(-1), ["retry"]);
  find("Close connections").props.onPress();
  assert.deepEqual(calls.at(-1), ["close"]);
  state.locked = true;
  h.render();
  assert.equal(find("Remove connection for this message").props.disabled, true);
  h.unmount();
});
