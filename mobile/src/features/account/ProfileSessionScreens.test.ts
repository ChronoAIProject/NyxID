import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (root: Element): Element[] => [
  root,
  ...[root.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child ? nodes(child) : [],
    ),
];

function setup(file: string, exportName: string, params: Record<string, unknown>) {
  const saved: unknown[] = [];
  const revoked: string[] = [];
  const events: string[] = [];
  let header: (() => Element) | undefined;
  let confirm: (() => void) | undefined;
  let mutation: { fn: (value: any) => Promise<unknown>; onSuccess?: () => unknown };
  const profile = { id: "me", display_name: "Alex", avatar_url: null, email: "a@b.c" };
  const sessions = [
    { id: "other", is_current: false, user_agent: "curl/8.7.1", ip_address: "1.2.3.4", created_at: "2026-11-03T12:00:00Z", expires_at: "2026-12-03T12:00:00Z" },
    { id: "mine", is_current: true, user_agent: "NyxID app", ip_address: null, created_at: "2026-11-03T12:00:00Z", expires_at: "2026-12-03T12:00:00Z" },
  ];
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": {
      Alert: {
        alert: (_title: string, _body: string, actions: any[]) => {
          confirm = actions.at(-1).onPress;
        },
      },
    },
    "@tanstack/react-query": {
      useQueryClient: () => ({
        invalidateQueries: async () => {
          events.push("invalidate");
        },
        clear: () => events.push("clear"),
      }),
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        data: queryKey[1] === "profile" ? profile : sessions,
        isPending: false,
      }),
      useMutation: (options: any) => {
        mutation = { fn: options.mutationFn, onSuccess: options.onSuccess };
        return {
          isPending: false,
          error: null,
          mutate: async (value: unknown) => {
            await options.mutationFn(value);
            await options.onSuccess?.();
          },
        };
      },
    },
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsPage: "SettingsPage",
      SettingsRow: "SettingsRow",
      SettingsTextField: "SettingsTextField",
      useSettingsHeaderAction: (
        _navigation: unknown,
        action: { enabled: boolean; onPress: () => void },
      ) => {
        header = () => ({ type: "HeaderAction", props: action });
      },
    },
    "../../components/workspace/WorkspaceUI": { Label: "Label", Notice: "Notice" },
    "../../hooks/useNetworkStatus": { useNetworkStatus: () => ({ isConnected: true }) },
    "../../lib/api/accountApi": {
      accountApi: {
        updateProfile: async (input: unknown) => {
          saved.push(input);
        },
        revokeSession: async (id: string) => {
          revoked.push(id);
        },
      },
    },
    "../../lib/api/errorMessages": { resolveErrorMessage: () => "error" },
    "../auth/AuthSessionContext": {
      useAuthSession: () => ({
        signOut: async () => {
          events.push("signOut");
        },
      }),
    },
  });
  const Screen = harness.load(fileURLToPath(new URL(file, import.meta.url)))[
    exportName
  ] as (props: unknown) => Element;
  const navigation = { goBack: () => events.push("back") };
  harness.mount(() => Screen({ navigation, route: { params } }));
  const all = () => nodes(harness.value());
  return {
    harness,
    all,
    saved,
    revoked,
    events,
    header: () => header!(),
    confirm: () => {
      assert.ok(confirm);
      confirm();
    },
    mutation: () => mutation,
  };
}

test("a profile value is edited on its own page and Save sends only that field", async () => {
  const env = setup("./ProfileFieldEditScreen.tsx", "ProfileFieldEditScreen", {
    field: "display_name",
  });
  const field = () => env.all().find((node) => node.type === "SettingsTextField")!;
  assert.equal(field().props.value, "Alex");
  assert.equal(env.header().props.enabled, false, "Save waits for a change");
  field().props.onChangeText("  Alex  ");
  env.harness.flush();
  assert.equal(env.header().props.enabled, false, "whitespace is not a change");
  field().props.onChangeText("Alexandra");
  env.harness.flush();
  assert.equal(env.header().props.enabled, true);
  env.header().props.onPress();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.saved)), [
    { display_name: "Alexandra" },
  ]);
  assert.deepEqual(env.events, ["invalidate", "back"]);
  env.harness.unmount();
});

test("an invalid avatar link is refused before any request", async () => {
  const env = setup("./ProfileFieldEditScreen.tsx", "ProfileFieldEditScreen", {
    field: "avatar_url",
  });
  const field = () => env.all().find((node) => node.type === "SettingsTextField")!;
  field().props.onChangeText("not a url");
  env.harness.flush();
  env.header().props.onPress();
  await env.harness.settle();
  assert.equal(env.saved.length, 0);
  assert.ok(env.all().some((node) => node.type === "Notice" && node.props.tone === "error"));
  env.harness.unmount();
});

test("a session page lists its details and signs it out only after confirmation", async () => {
  const other = setup("./SessionDetailScreen.tsx", "SessionDetailScreen", {
    sessionId: "other",
  });
  const rows = () => other.all().filter((node) => node.type === "SettingsRow");
  assert.equal(rows().find((row) => row.props.label === "Network")!.props.value, "1.2.3.4");
  const signOut = rows().find((row) => row.props.destructive)!;
  assert.equal(signOut.props.label, "Sign Out This Session");
  signOut.props.onPress();
  assert.equal(other.revoked.length, 0, "nothing happens before confirmation");
  other.confirm();
  await other.harness.settle();
  assert.deepEqual(other.revoked, ["other"]);
  assert.deepEqual(other.events, ["invalidate", "back"]);
  other.harness.unmount();

  const mine = setup("./SessionDetailScreen.tsx", "SessionDetailScreen", {
    sessionId: "mine",
  });
  const current = mine.all().find(
    (node) => node.type === "SettingsRow" && node.props.destructive,
  )!;
  assert.equal(current.props.label, "Sign Out on This Device");
  current.props.onPress();
  mine.confirm();
  await mine.harness.settle();
  assert.deepEqual(mine.events, ["signOut", "clear"]);
  mine.harness.unmount();
});
