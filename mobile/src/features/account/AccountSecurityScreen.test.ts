import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";

type Element = { type: unknown; props: Record<string, any> };
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
function setup(passwordAccount: boolean) {
  let focus: () => (() => void) | void = () => {};
  let confirm: (() => void) | undefined;
  let failure = false;
  let refreshes = 0;
  let appState: (state: string) => void = () => {};
  let beforeDispatch: Promise<void> | null = null;
  const calls: unknown[] = [];
  const refetch = async () => {};
  const profile = {
    id: "account",
    email: "test@example.test",
    display_name: "Test",
    mfa_enabled: true,
    has_password: passwordAccount,
    social_provider: passwordAccount ? null : "google",
  };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "@react-navigation/native": {
      useFocusEffect: (fn: typeof focus) => {
        focus = fn;
      },
      useNavigation: () => ({ navigate() {} }),
    },
    "../../components/settings/SettingsList": Object.fromEntries(
      ["SettingsGroup", "SettingsPage", "SettingsRow", "SettingsTextField"].map(
        (name) => [name, name],
      ),
    ),
    "./SessionDetailScreen": { formatSessionDate: (value: string) => value },
    "react-native": {
      View: "View",
      AppState: {
        currentState: "active",
        addEventListener: (_event: string, callback: typeof appState) => {
          appState = callback;
          return { remove() {} };
        },
      },
      Alert: {
        alert: (_title: string, _text: string, actions: any[]) => {
          confirm = actions.at(-1).onPress;
        },
      },
    },
    "expo-clipboard": {},
    "../../app/WorkspaceHeader": { useWorkspaceBack() {} },
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      [
        "ActionButton",
        "Card",
        "Field",
        "Label",
        "Notice",
        "Section",
        "WorkspacePage",
      ].map((name) => [name, name]),
    ),
    "@tanstack/react-query": {
      useQueryClient: () => ({
        invalidateQueries: async () => {
          refreshes++;
        },
      }),
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        data: queryKey[1] === "profile" ? profile : [],
        refetch,
      }),
      useMutation: () => ({ isPending: false }),
    },
    "../../lib/api/accountApi": {
      accountApi: {
        disableMfa: async (
          input: unknown,
          options: { authorityGuard: () => void },
        ) => {
          if (beforeDispatch) await beforeDispatch;
          options.authorityGuard();
          calls.push(input);
          if (failure) throw new Error("Invalid code");
        },
      },
    },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../auth/AuthSessionContext": { useAuthSession: () => ({ signOut() {} }) },
    "./AccountSecurityControls": {
      AccountSecurityControls: "AccountSecurityControls",
    },
    "./AccountAuthorizations": {
      AccountAuthorizations: "AccountAuthorizations",
    },
    "./AccountOrganizations": { AccountOrganizations: "AccountOrganizations" },
  });
  const { AccountSecurityScreen } = harness.load(
    fileURLToPath(new URL("./AccountSecurityScreen.tsx", import.meta.url)),
  ) as { AccountSecurityScreen: () => Element };
  harness.mount(AccountSecurityScreen);
  focus();
  harness.flush();
  const find = (label: string) => {
    const node = elements(harness.value()).find(
      (node) =>
        node.props.label === label ||
        (node.type === "SettingsTextField" &&
          node.props.accessibilityLabel === label),
    );
    assert.ok(node, `Missing ${label}`);
    return node;
  };
  find("Turn Off Two-Factor Authentication").props.onPress();
  harness.flush();
  return {
    harness,
    find,
    calls,
    confirm: () => {
      assert.ok(confirm);
      confirm();
    },
    fail: () => {
      failure = true;
    },
    refreshes: () => refreshes,
    state: (value: string) => {
      appState(value);
      harness.flush();
    },
    queueDispatch: (promise: Promise<void>) => {
      beforeDispatch = promise;
    },
  };
}

test("password MFA disable requires both code and password and failed verification preserves enabled state", async () => {
  const env = setup(true);
  assert.equal(
    env.find("Turn Off Two-Factor Authentication").props.disabled,
    true,
  );
  env.find("Current password").props.onChangeText("password");
  env.harness.flush();
  assert.equal(
    env.find("Turn Off Two-Factor Authentication").props.disabled,
    true,
  );
  env.find("Authenticator or recovery code").props.onChangeText("123456");
  env.harness.flush();
  env.fail();
  env.find("Turn Off Two-Factor Authentication").props.onPress();
  env.confirm();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.calls)), [
    { code: "123456", password: "password" },
  ]);
  assert.equal(env.refreshes(), 0);
  assert.ok(
    env.find("Authenticator or recovery code"),
    "bad code keeps the disable form available",
  );
  env.harness.unmount();
});

test("social account can disable MFA with a recovery code and no password field", async () => {
  const env = setup(false);
  assert.equal(
    elements(env.harness.value()).some(
      (node) => node.props.label === "Current password",
    ),
    false,
  );
  env
    .find("Authenticator or recovery code")
    .props.onChangeText("recovery-code");
  env.harness.flush();
  env.find("Turn Off Two-Factor Authentication").props.onPress();
  env.confirm();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.calls)), [
    { code: "recovery-code" },
  ]);
  assert.equal(env.refreshes(), 1);
  env.harness.unmount();
});

test("a transient inactive state (Face ID, Control Center) keeps the MFA disable confirmation", async () => {
  const env = setup(false);
  env
    .find("Authenticator or recovery code")
    .props.onChangeText("recovery-code");
  env.harness.flush();
  env.find("Turn Off Two-Factor Authentication").props.onPress();
  env.state("inactive");
  env.state("active");
  env.confirm();
  await env.harness.settle();
  assert.equal(env.calls.length, 1);
  env.harness.unmount();
});

test("MFA disable confirmation cannot revive after a background round trip", async () => {
  const env = setup(false);
  env
    .find("Authenticator or recovery code")
    .props.onChangeText("recovery-code");
  env.harness.flush();
  env.find("Turn Off Two-Factor Authentication").props.onPress();
  env.state("inactive");
  env.state("background");
  env.state("active");
  env.confirm();
  await env.harness.settle();
  assert.equal(env.calls.length, 0);
  env.harness.unmount();
});

test("MFA disable cannot dispatch after waiting across a background round trip", async () => {
  const env = setup(false);
  const queued = deferred<void>();
  env.queueDispatch(queued.promise);
  env
    .find("Authenticator or recovery code")
    .props.onChangeText("recovery-code");
  env.harness.flush();
  env.find("Turn Off Two-Factor Authentication").props.onPress();
  env.confirm();
  env.state("background");
  env.state("active");
  queued.resolve();
  await env.harness.settle();
  assert.equal(env.calls.length, 0);
  assert.equal(env.refreshes(), 0);
  env.harness.unmount();
});
