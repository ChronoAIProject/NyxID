import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
type Element = { type: unknown; props: Record<string, any> };
const nodes = (root: Element): Element[] => [
  root,
  ...[root.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? nodes(child)
        : [],
    ),
];
function setup(
  component: string,
  props: Record<string, unknown>,
  values: Record<string, unknown>,
) {
  const calls: unknown[][] = [];
  let allowed = true;
  let activation = 0;
  let beforeDispatch: Promise<void> | null = null;
  let afterDispatch: Promise<void> | null = null;
  let stepUpRetry: (() => Promise<void>) | null = null;
  let stepUpError = false;
  let resets = 0;
  let confirm: (() => void) | undefined;
  const refetch = async () => {
    calls.push(["refetch"]);
    return { data: [], error: null };
  };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const api = new Proxy(
    {},
    {
      get:
        (_target, name) =>
        async (...args: unknown[]) => {
          if (beforeDispatch) await beforeDispatch;
          const guard = args.at(-1) as { authorityGuard?: () => void };
          guard?.authorityGuard?.();
          calls.push([name, ...args]);
          if (afterDispatch) await afterDispatch;
          if (stepUpError) throw new Error("Verification required");
          return name === "createOrganization"
            ? { id: "org" }
            : name === "joinOrganization"
              ? { org_id: "org" }
              : {};
        },
    },
  );
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-native": {
      Switch: "Switch",
      Alert: {
        alert: (_title: string, _body: string, actions: any[]) => {
          confirm = actions.at(-1).onPress;
        },
      },
    },
    "expo-web-browser": {
      openBrowserAsync: async (url: string) => {
        calls.push(["browser", url]);
        return { type: "dismiss" };
      },
    },
    "@tanstack/react-query": {
      useQuery: ({ queryKey }: { queryKey: string[] }) => ({
        data: values[queryKey.at(-1)!],
        refetch,
      }),
      useQueryClient: () => ({
        invalidateQueries: async (filter: unknown) => {
          calls.push(["invalidate", filter]);
        },
        resetQueries: async (filter: unknown) => {
          calls.push(["reset", filter]);
        },
      }),
    },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../lib/api/accountApi": { accountApi: api },
    "../../lib/env": { NYXID_FRONTEND_URL: "https://nyxid.example" },
    "../../hooks/useNetworkStatus": {
      useNetworkStatus: () => ({ isConnected: true }),
    },
    "../auth/useMfaStepUp": {
      useMfaStepUp: () => ({
        busy: false,
        prompt: null,
        handle: async (_error: unknown, retry: () => Promise<void>) => {
          if (!stepUpError) return false;
          stepUpRetry = retry;
          return true;
        },
        reset: () => {
          resets++;
        },
      }),
    },
    "./useAccountSecurityRefresh": {
      useAccountSecurityRefresh: () =>
        Object.assign(() => allowed, {
          capture: () => {
            const expected = activation;
            const admitted = allowed;
            const isCurrent = () =>
              admitted && allowed && expected === activation;
            return {
              isCurrent,
              assertCurrent: () => {
                if (!isCurrent())
                  throw Object.assign(new Error("Cancelled"), {
                    name: "AbortError",
                  });
              },
            };
          },
        }),
    },
  });
  const loaded = harness.load(
    fileURLToPath(new URL(`./${component}.tsx`, import.meta.url)),
  ) as Record<string, (props: Record<string, unknown>) => Element>;
  harness.mount(() => loaded[component]!(props));
  const find = (label: string) => {
    const node = nodes(harness.value()).find(
      (node) => node.props.label === label,
    );
    assert.ok(node, `Missing ${label}`);
    return node;
  };
  return {
    harness,
    calls,
    find,
    confirm: () => {
      assert.ok(confirm);
      confirm();
    },
    setAllowed: (value: boolean) => {
      if (!value) activation++;
      allowed = value;
    },
    queueDispatch: (promise: Promise<void> | null) => {
      beforeDispatch = promise;
    },
    holdResponse: (promise: Promise<void> | null) => {
      afterDispatch = promise;
    },
    requireStepUp: () => {
      stepUpError = true;
    },
    retryStepUp: async () => {
      assert.ok(stepUpRetry);
      await stepUpRetry();
    },
    resets: () => resets,
    values,
  };
}

test("password/policy controls submit live API data, request confirmation for weaker policy, and ignore stale screen actions", async () => {
  const env = setup(
    "AccountSecurityControls",
    {
      profile: {
        id: "owner",
        has_password: true,
        mfa_enabled: true,
        profile_config: { security: { skip_mfa_for_approved_sign_ins: false } },
      },
    },
    {},
  );
  for (const [field, value] of [
    ["Current password for password change", "old"],
    ["New password", "new password"],
    ["Confirm new password", "new password"],
  ] as const) {
    env.find(field).props.onChangeText(value);
    env.harness.flush();
  }
  env.find("Change password").props.onPress();
  await env.harness.settle();
  assert.equal(env.calls[0]?.[0], "changePassword");
  assert.equal(env.calls[0]?.[1], "owner");
  assert.equal(env.find("New password").props.value, "");
  const toggle = nodes(env.harness.value()).find(
    (node) => node.type === "Switch",
  )!;
  toggle.props.onValueChange(true);
  assert.equal(
    env.calls.some((call) => call[0] === "saveSecurityPreferences"),
    false,
  );
  env.confirm();
  await env.harness.settle();
  assert.ok(env.calls.some((call) => call[0] === "saveSecurityPreferences"));
  const count = env.calls.length;
  env.setAllowed(false);
  toggle.props.onValueChange(false);
  await env.harness.settle();
  assert.equal(env.calls.length, count);
  env.harness.unmount();
});

test("OAuth/broker revocation confirms the exact source then re-reads server lists", async () => {
  const env = setup(
    "AccountAuthorizations",
    { accountId: "owner" },
    {
      apps: [
        {
          client_id: "client",
          client_name: "Test app",
          scopes: "openid",
          allowed_services: [],
          granted_at: "now",
          expires_at: null,
        },
      ],
      bindings: [
        {
          binding_hash: "hash",
          client_id: "client",
          client_name: "Broker",
          scopes: ["read"],
          created_at: "now",
          last_used_at: null,
        },
      ],
    },
  );
  env.find("Revoke Test app").props.onPress();
  assert.equal(env.calls.length, 0);
  env.confirm();
  await env.harness.settle();
  assert.equal(env.calls[0]?.[0], "revokeConsent");
  assert.equal(env.calls[0]?.[1], "owner");
  assert.equal(env.calls[0]?.[2], "client");
  assert.equal(env.calls[1]?.[0], "reset");
  env.find("Revoke Broker").props.onPress();
  env.confirm();
  await env.harness.settle();
  assert.ok(
    env.calls.some(
      (call) => call[0] === "revokeBrokerBinding" && call[2] === "hash",
    ),
  );
  env.harness.unmount();
});

test("organizations expose native create/join and owner-specific browser actions; revoked admin role removes management controls", async () => {
  const org = {
    id: "org",
    slug: "example",
    display_name: "Team",
    your_role: "admin",
  };
  const env = setup(
    "AccountOrganizations",
    { accountId: "owner", email: "person@example.test" },
    { owner: [org] },
  );
  env.find("Team · admin").props.onPress();
  env.harness.flush();
  env.find("Role permissions on website").props.onPress();
  await env.harness.settle();
  assert.ok(
    env.calls.some(
      (call) =>
        call[0] === "browser" &&
        call[1] === "https://nyxid.example/orgs/org?tab=role-permissions",
    ),
  );
  assert.ok(
    nodes(env.harness.value()).some((node) =>
      /no browser action is assumed completed/.test(node.props.message ?? ""),
    ),
  );
  env.values.owner = [{ ...org, your_role: "viewer" }];
  env.harness.render();
  assert.equal(
    nodes(env.harness.value()).some(
      (node) => node.props.label === "Invitations on website",
    ),
    false,
  );
  env.find("New organization name").props.onChangeText("New Team");
  env.harness.flush();
  env.find("Create organization").props.onPress();
  await env.harness.settle();
  assert.ok(
    env.calls.some(
      (call) =>
        call[0] === "createOrganization" &&
        call[1] === "owner" &&
        call[2] === "New Team",
    ),
  );
  env.find("Organization invitation code").props.onChangeText("nonce");
  env.harness.flush();
  env.find("Accept organization invitation").props.onPress();
  env.confirm();
  await env.harness.settle();
  assert.ok(
    env.calls.some(
      (call) => call[0] === "joinOrganization" && call[2] === "nonce",
    ),
  );
  env.harness.unmount();
});

const securityProfile = {
  id: "owner",
  has_password: true,
  mfa_enabled: true,
  profile_config: { security: { skip_mfa_for_approved_sign_ins: false } },
};
function fillPassword(env: ReturnType<typeof setup>) {
  for (const [field, value] of [
    ["Current password for password change", "old password"],
    ["New password", "new password"],
    ["Confirm new password", "new password"],
  ] as const) {
    env.find(field).props.onChangeText(value);
    env.harness.flush();
  }
}

test("queued password changes cannot dispatch after leaving and returning", async () => {
  const env = setup(
    "AccountSecurityControls",
    { profile: securityProfile },
    {},
  );
  const queued = deferred<void>();
  env.queueDispatch(queued.promise);
  fillPassword(env);
  env.find("Change password").props.onPress();
  env.setAllowed(false);
  env.setAllowed(true);
  queued.resolve();
  await env.harness.settle();
  assert.equal(env.calls.length, 0);
  assert.equal(env.find("Change password").props.loading, false);
  env.harness.unmount();
});

test("late password responses never display success in a new activation", async () => {
  const env = setup(
    "AccountSecurityControls",
    { profile: securityProfile },
    {},
  );
  const response = deferred<void>();
  env.holdResponse(response.promise);
  fillPassword(env);
  env.find("Change password").props.onPress();
  assert.equal(env.calls.length, 1);
  env.setAllowed(false);
  env.setAllowed(true);
  response.resolve();
  await env.harness.settle();
  assert.equal(
    env.calls.length,
    1,
    "old response must not invalidate fresh profile data",
  );
  assert.equal(
    nodes(env.harness.value()).some((node) => node.props.tone === "success"),
    false,
  );
  env.harness.unmount();
});

test("old policy confirmation and step-up retries cannot become new writes after returning", async () => {
  const env = setup(
    "AccountSecurityControls",
    { profile: securityProfile },
    {},
  );
  const toggle = () =>
    nodes(env.harness.value()).find((node) => node.type === "Switch")!;
  toggle().props.onValueChange(true);
  env.setAllowed(false);
  env.setAllowed(true);
  env.confirm();
  await env.harness.settle();
  assert.equal(env.calls.length, 0, "old warning confirmation is discarded");
  env.requireStepUp();
  toggle().props.onValueChange(true);
  env.confirm();
  await env.harness.settle();
  assert.equal(env.calls.length, 1);
  env.setAllowed(false);
  env.setAllowed(true);
  await env.retryStepUp();
  await env.harness.settle();
  assert.equal(
    env.calls.length,
    1,
    "old MFA retry must retain its original ticket",
  );
  env.harness.unmount();
});

test("OAuth and Broker confirmations expire on leaving the screen", async () => {
  const env = setup(
    "AccountAuthorizations",
    { accountId: "owner" },
    {
      apps: [
        {
          client_id: "client",
          client_name: "App",
          scopes: "openid",
          allowed_services: [],
          granted_at: "now",
        },
      ],
      bindings: [
        {
          binding_hash: "hash",
          client_name: "Broker",
          scopes: [],
          created_at: "now",
        },
      ],
    },
  );
  for (const label of ["Revoke App", "Revoke Broker"]) {
    env.find(label).props.onPress();
    env.setAllowed(false);
    env.setAllowed(true);
    env.confirm();
    await env.harness.settle();
  }
  assert.equal(env.calls.length, 0);
  env.harness.unmount();
});

test("organization join confirmation and queued creation retain their originating activation", async () => {
  const env = setup(
    "AccountOrganizations",
    { accountId: "owner", email: "person@example.test" },
    { owner: [] },
  );
  env.find("Organization invitation code").props.onChangeText("nonce");
  env.harness.flush();
  env.find("Accept organization invitation").props.onPress();
  env.setAllowed(false);
  env.setAllowed(true);
  env.confirm();
  await env.harness.settle();
  assert.equal(env.calls.length, 0);
  const queued = deferred<void>();
  env.queueDispatch(queued.promise);
  env.find("New organization name").props.onChangeText("Team");
  env.harness.flush();
  env.find("Create organization").props.onPress();
  env.setAllowed(false);
  env.setAllowed(true);
  queued.resolve();
  await env.harness.settle();
  assert.equal(env.calls.length, 0);
  env.harness.unmount();
});
