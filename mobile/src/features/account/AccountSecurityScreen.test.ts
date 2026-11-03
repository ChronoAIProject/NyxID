import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";

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
    },
    "react-native": {
      View: "View",
      AppState: {
        currentState: "active",
        addEventListener: () => ({ remove() {} }),
      },
      Alert: {
        alert: (_title: string, _text: string, actions: any[]) => {
          confirm = actions.at(-1).onPress;
        },
      },
    },
    "expo-clipboard": {},
    "../../app/WorkspaceHeader": { useWorkspaceBack() {} },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice", "WorkspacePage"].map(
        (name) => [name, name],
      ),
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
        disableMfa: async (input: unknown) => {
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
      (node) => node.props.label === label,
    );
    assert.ok(node, `Missing ${label}`);
    return node;
  };
  find("Turn off two-factor authentication").props.onPress();
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
  };
}

test("password MFA disable requires both code and password and failed verification preserves enabled state", async () => {
  const env = setup(true);
  assert.equal(
    env.find("Turn off two-factor authentication").props.disabled,
    true,
  );
  env.find("Current password").props.onChangeText("password");
  env.harness.flush();
  assert.equal(
    env.find("Turn off two-factor authentication").props.disabled,
    true,
  );
  env.find("Authenticator or recovery code").props.onChangeText("123456");
  env.harness.flush();
  env.fail();
  env.find("Turn off two-factor authentication").props.onPress();
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
  env.find("Turn off two-factor authentication").props.onPress();
  env.confirm();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.calls)), [
    { code: "recovery-code" },
  ]);
  assert.equal(env.refreshes(), 1);
  env.harness.unmount();
});
