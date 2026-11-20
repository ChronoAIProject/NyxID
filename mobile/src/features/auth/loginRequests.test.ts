import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness, deferred } from "../assistant/assistantHookTestHarness";
import { authDevicePreviewSchema } from "../../lib/api/authDeviceSchema";
import {
  agentKeyOptionsSchema,
  agentKeyPreviewSchema,
  type AgentKeyApprove,
  type AgentKeyPreview,
} from "../../lib/api/agentKeyLoginSchema";
import * as keyModel from "./agentKeyLoginModel";
import * as codes from "./deviceUserCode";
import * as times from "./deviceLoginPreview";
import { buildApiError, networkApiError } from "../../lib/api/responseError";
import { resolveAuthDeviceErrorMessage } from "../../lib/api/errorMessages";
import type * as Resolver from "./loginRequestResolver";

type Element = { type: unknown; props: Record<string, unknown> };
const path = (name: string) => fileURLToPath(new URL(name, import.meta.url));
const elements = (root: Element): Element[] => [
  root,
  ...[root.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? elements(child as Element)
        : [],
    ),
];
const preview = (status = "pending", seconds = 60): AgentKeyPreview => ({
  ...authDevicePreviewSchema.parse({
    status,
    seconds_remaining: seconds,
    initiated_at: "2026-11-14T00:00:00Z",
    expires_at: "2099-01-01T00:00:00Z",
    supports_grant_choice: true,
  }),
  requested_profile: null,
  interval: 5,
});
const missing = (kind: string) => ({
  errorCode: kind === "device" ? 11207 : 11907,
});
function setup(sharedResolver?: typeof Resolver, renderManualModal = false) {
  const calls: string[] = [];
  let navigationDisabled = false;
  const approvals: AgentKeyApprove[] = [];
  const renders: Element[] = [];
  const decisionCodes: string[] = [];
  const stepUpCalls: unknown[] = [];
  let keyExpiry: string | null = null;
  let stepUpPrompt: Element | null = null;
  let stepUpScope: string | undefined;
  let routeParams: Record<string, unknown> = {};
  let clock = 100_000;
  let state = "active";
  const listeners = new Set<(state: string) => void>();
  const timers = new Set<() => void>();
  const timeouts = new Map<() => void, number>();
  const blurListeners = new Set<() => void>();
  let renderParams = false;
  let device: () => Promise<ReturnType<typeof preview>> = async () => preview();
  let agent: () => Promise<ReturnType<typeof preview>> = async () => preview();
  let decision: () => Promise<void> = async () => {};
  const stagingBinding = {
    request_id: "11111111-1111-4111-8111-111111111111",
    production_user: "22222222-2222-4222-8222-222222222222",
    staging_user: "33333333-3333-4333-8333-333333333333",
  };
  const api = {
    previewStagingDevice: async (code: string, binding?: typeof stagingBinding) => {
      calls.push(binding ? "staging-refresh" : "staging-preview");
      if (binding) assert.deepEqual(binding, stagingBinding);
      return { ...authDevicePreviewSchema.parse(await device()), stagingBinding };
    },
    decideStagingDevice: async (action: string, code: string, binding: typeof stagingBinding) => {
      calls.push("staging-" + action);
      assert.deepEqual(binding, stagingBinding);
      decisionCodes.push(code);
      return decision();
    },
    previewAuthDevice: async () => {
      calls.push("device");
      return authDevicePreviewSchema.parse(await device());
    },
    approveAuthDevice: async (code: string) => {
      decisionCodes.push(code);
      calls.push("approve");
      return decision();
    },
    denyAuthDevice: async (code: string) => {
      decisionCodes.push(code);
      calls.push("deny");
      return decision();
    },
  };
  const keyApi = {
    options: async () =>
      agentKeyOptionsSchema.parse({
        keys: [
          {
            id: "12345678-1234-4234-8234-123456789012",
            name: "Reviewed key",
            key_prefix: "nyx_test",
            owner_type: "personal",
            owner_id: "owner",
            owner_name: "Your account",
            scopes: "read proxy",
            allow_all_services: false,
            allow_all_nodes: false,
            allowed_service_ids: [],
            allowed_node_ids: [],
            allowed_services: [],
            allowed_nodes: [],
            expires_at: keyExpiry,
            rate_limit_per_second: null,
            rate_limit_burst: null,
            platform: null,
            created_now: false,
          },
        ],
        services: [],
        nodes: [],
        orgs: [],
      }),
    approve: async (input: AgentKeyApprove) => {
      approvals.push(input);
      decisionCodes.push(input.user_code);
      calls.push("approve");
      return decision();
    },
    deny: async (code: string) => {
      decisionCodes.push(code);
      calls.push("deny");
      return decision();
    },
    preview: async (_code: string, flow = "agent-key") => {
      calls.push(flow);
      return agentKeyPreviewSchema.parse(
        await (flow === "device" ? device() : agent()),
      );
    },
  };
  const harness = hookHarness<Element>(
    {
      "react/jsx-runtime": {
        jsx: (type: unknown, props: Element["props"]) =>
          renderManualModal && typeof type === "function"
            ? type(props)
            : { type, props },
        jsxs: (type: unknown, props: Element["props"]) => ({ type, props }),
        Fragment: "Fragment",
      },
      "react-native": {
        ...Object.fromEntries(
          [
            "View",
            "Text",
            "TextInput",
            "Pressable",
            "ScrollView",
            "Modal",
            "KeyboardAvoidingView",
            "ActivityIndicator",
          ].map((n) => [n, n]),
        ),
        Platform: { OS: "ios" },
        AppState: {
          get currentState() {
            return state;
          },
          addEventListener: (
            _event: string,
            listener: (state: string) => void,
          ) => {
            listeners.add(listener);
            return { remove: () => listeners.delete(listener) };
          },
        },
      },
      "react-native-safe-area-context": {
        SafeAreaProvider: "SafeAreaProvider",
        useSafeAreaInsets: () => ({ bottom: 0, top: 0 }),
      },
      "lucide-react-native": Object.fromEntries(
        [
          "ChevronDown",
          "ChevronLeft",
          "ChevronUp",
          "Info",
          "ScanQrCode",
          "X",
          "KeyRound",
          "UserRound",
        ].map((n) => [n, n]),
      ),
      "../../app/WorkspaceHeader": {
        WorkspaceSheetContext: { Provider: "WorkspaceSheetProvider" },
        useWorkspaceSheet: () => ({}),
        useWorkspaceBack: (input: { disabled: boolean }) => {
          navigationDisabled = input.disabled;
        },
      },
      "../../app/navigationHierarchy": { workspaceNavigateAction: () => ({}) },
      "../../components/PrimaryButton": { PrimaryButton: "Button" },
      "./AuthSubmitButton": { AuthSubmitButton: "Button" },
      "../../components/ScreenContainer": {
        ScreenContainer: "ScreenContainer",
      },
      "../../components/workspace/KeyboardViewport": { KeyboardViewport: "KeyboardViewport" },
      "../../lib/accessibility/useReducedMotion": {
        useReducedMotion: () => true,
      },
      "../../lib/api/errorMessages": {
        resolveAuthDeviceErrorMessage,
      },
      "../../lib/api/mobileApi": { mobileApi: api },
      "../../lib/api/agentKeyLoginApi": { agentKeyLoginApi: keyApi },
      "../../lib/api/baseUrl": {
        getApiBaseUrl: () => "https://login.example.test/api/v1",
      },
      "../../lib/api/loginCodeApi": {
        loginCodeApi: {
          options: keyApi.options,
          mint: async () => {
            calls.push("mint");
            await decision();
            return {
              request_id: "minted-request",
              code: "one-time-code",
              expires_at: "2099-01-01T00:00:00Z",
            };
          },
        },
      },
      "./LoginCodeStatusView": { LoginCodeStatusView: "LoginCodeStatusView" },
      "../../components/settings/SettingsList": {
        SettingsGroup: "SettingsGroup",
        SettingsRow: "SettingsRow",
      },
      "../../components/workspace/WorkspaceSelectionField": {
        WorkspaceSelectionField: "WorkspaceSelectionField",
      },
      "./AgentKeyCreateForm": { AgentKeyCreateForm: "AgentKeyCreateForm" },
      "./agentKeyLoginModel": keyModel,
      "../../theme/designTokens": { spacing: { huge: 32 } },
      "./useAuthTheme": { useAuthTheme: () => ({ colors: {} }) },
      "./AuthTextInput": { AuthTextInput: "TextInput" },
      "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
      "./AuthSessionContext": {
        useAuthSession: () => ({ isAuthenticated: true }),
      },
      "./useMfaStepUp": {
        useMfaStepUp: (scope: string) => {
          if (scope !== stepUpScope) stepUpPrompt = null;
          stepUpScope = scope;
          return {
            busy: false,
            prompt: stepUpPrompt,
            handle: async (failure: { errorCode?: number }) => {
              stepUpCalls.push(failure);
              // Match the real hook: changed lifecycle discards even non-MFA errors.
              if (state === "background") return true;
              if (failure?.errorCode !== 2003) return false;
              stepUpPrompt = { type: "StepUpPrompt", props: {} };
              return true;
            },
          };
        },
      },
      "./DeviceCodeScanner": { DeviceCodeScanner: "DeviceCodeScanner" },
      "./LoginRequestTerminalView": { LoginRequestTerminalView: "Terminal" },
      "./deviceUserCode": codes,
      "./deviceLoginPreview": times,
    },
    {
      Error,
      Date: class extends Date {
        static override now() {
          return clock;
        }
      },
      setInterval: (fn: () => void) => {
        timers.add(fn);
        return fn;
      },
      clearInterval: (fn: () => void) => timers.delete(fn),
      setTimeout: (fn: () => void, ms: number) => {
        timeouts.set(fn, clock + ms);
        return fn;
      },
      clearTimeout: (fn: () => void) => timeouts.delete(fn),
    },
  );
  const resolver =
    sharedResolver ??
    (harness.load(path("./loginRequestResolver.ts")) as typeof Resolver);
  harness.mock("./loginRequestResolver", resolver);
  harness.mock(
    "./useLoginRequestHandoff",
    harness.load(path("./useLoginRequestHandoff.ts")),
  );
  harness.mock(
    "./useUncertainLoginPreview",
    harness.load(path("./useUncertainLoginPreview.ts")),
  );
  harness.mock(
    "./useLoginRequestResume",
    harness.load(path("./useLoginRequestResume.ts")),
  );
  harness.mock(
    "./useLoginPreviewCooldown",
    harness.load(path("./useLoginPreviewCooldown.ts")),
  );
  const deviceScreen = harness.load(path("./DeviceLoginScreen.tsx")) as {
    DeviceLoginScreen: (props: unknown) => Element;
  };
  harness.mock("./DeviceLoginScreen", {
    DetailRow: "DetailRow",
    formatLocation: () => "",
    formatNetwork: () => "",
    formatDevice: () => "",
    requesterValue: () => "",
  });
  const agentScreen = harness.load(path("./AgentKeyLoginScreen.tsx")) as {
    AgentKeyLoginScreen: (props: unknown) => Element;
  };
  const navigation = {
    addListener: (event: string, callback: () => void) => {
      if (event === "blur") blurListeners.add(callback);
      return () => {
        blurListeners.delete(callback);
      };
    },
    navigate: (name: string, params?: unknown) =>
      calls.push(`navigate:${name}:${JSON.stringify(params)}`),
    canGoBack: () => true,
    goBack: () => calls.push("back"),
    setParams: (params: Record<string, unknown>) => {
      Object.assign(routeParams, params);
      if (renderParams) harness.render();
    },
  };
  return {
    harness,
    resolver,
    calls,
    approvals,
    renders,
    decisionCodes,
    stepUpCalls,
    setKeyExpiry: (value: string | null) => { keyExpiry = value; },
    navigationDisabled: () => navigationDisabled,
    params: () => routeParams,
    synchronousParams: () => {
      renderParams = true;
    },
    blur: () => {
      for (const listener of blurListeners) listener();
    },
    timeouts: () => timeouts.size,
    setDevice(fn: typeof device) {
      device = fn;
    },
    setAgent(fn: typeof agent) {
      agent = fn;
    },
    setDecision(fn: typeof decision) {
      decision = fn;
    },
    mount(kind = "device", params: object = {}) {
      routeParams = { ...params };
      harness.mount(() => {
        const rendered = kind === "device"
          ? deviceScreen.DeviceLoginScreen({ navigation, route: { params: routeParams } })
          : agentScreen.AgentKeyLoginScreen({ navigation, route: { params: routeParams } });
        renders.push(rendered);
        return rendered;
      });
    },
    nodes: () => elements(harness.value()),
    field(value: string) {
      const field = elements(harness.value()).find(
        (e) => e.type === "TextInput",
      );
      assert.ok(field);
      (field.props.onChangeText as (value: string) => void)(value);
      harness.flush();
    },
    async press(label: string) {
      const node = elements(harness.value()).find(
        (e) => e.props.label === label,
      );
      assert.ok(node, `Missing ${label}`);
      (node.props.onPress as () => void)();
      await harness.settle();
    },
    elapse(ms: number) {
      clock += ms;
    },
    tickBeforeCommit() {
      for (const timer of [...timers]) timer();
    },
    async advanceAsync(ms: number) {
      const target = clock + ms;
      let count = 0;
      while (true) {
        const next = [...timeouts].filter(([, due]) => due <= target).sort((a, b) => a[1] - b[1])[0];
        if (!next) break;
        assert.ok(++count < 100, "fake timers must settle");
        clock = next[1];
        timeouts.delete(next[0]);
        next[0]();
        await harness.settle();
      }
      clock = target;
      for (const timer of [...timers]) timer();
      await harness.settle();
    },
    advance(ms: number) {
      clock += ms;
      for (const timer of [...timers]) timer();
      for (const [timer, due] of [...timeouts]) {
        if (due > clock) continue;
        timeouts.delete(timer);
        timer();
      }
      harness.flush();
    },
    async app(next: string) {
      state = next;
      for (const listener of [...listeners]) listener(next);
      await harness.settle();
    },
  };
}

test("resolver chooses device or Agent Key, only falls back on definitive not-found, and never decides", async () => {
  for (const first of ["device", "agent-key"] as const) {
    const env = setup();
    assert.equal(
      (await env.resolver.resolveLoginRequest("ABCD-EFGH", first)).kind,
      first,
    );
    assert.deepEqual(env.calls, [first]);
    env.calls.length = 0;
    if (first === "device")
      env.setDevice(async () => {
        throw missing(first);
      });
    else
      env.setAgent(async () => {
        throw missing(first);
      });
    assert.equal(
      (await env.resolver.resolveLoginRequest("ABCDEFGH", first)).kind,
      first === "device" ? "agent-key" : "device",
    );
    assert.deepEqual(env.calls, [
      first,
      first === "device" ? "agent-key" : "device",
    ]);
  }
});

test("resolver distinguishes malformed, both not found, terminal, MFA, network and rate errors without extra probes", async () => {
  for (const first of ["device", "agent-key"] as const) {
    const env = setup();
    await assert.rejects(env.resolver.resolveLoginRequest("bad", first), {
      kind: "malformed",
    });
    assert.deepEqual(env.calls, []);
    const base = first === "device" ? 11200 : 11900;
    for (const [error, kind] of [
      [{ errorCode: base + 1 }, "expired"],
      [{ errorCode: base + 4 }, "denied"],
      [{ errorCode: base + 5 }, "completed"],
      [{ errorCode: 2003 }, "mfa_step_up"],
      [networkApiError(new TypeError("Network request failed")), "network"],
    ] as const) {
      env.calls.length = 0;
      const fail = async () => {
        throw error;
      };
      if (first === "device") env.setDevice(fail);
      else env.setAgent(fail);
      await assert.rejects(
        env.resolver.resolveLoginRequest("ABCDEFGH", first),
        { kind },
      );
      assert.deepEqual(env.calls, [first]);
    }
    env.setDevice(async () => {
      throw missing("device");
    });
    env.setAgent(async () => {
      throw missing("agent-key");
    });
    await assert.rejects(env.resolver.resolveLoginRequest("ABCDEFGH", first), {
      kind: "not_found",
    });
    assert.match(env.resolver.loginErrorCopy({ errorCode: 11207 }), /{{host}}/);
    assert.equal(env.resolver.loginServerHost(), "login.example.test");
  }
});

test("HTTP 429 with any code stops fallback and honors Retry-After or the default cooldown", async () => {
  for (const errorCode of [1005, 11203, 11206, 11903, 11906, 11207, 11907, 0]) {
    const env = setup();
    const failure = buildApiError(
      { error: "rate_limited", error_code: errorCode },
      429,
      { get: (name) => (name === "Retry-After" ? "12" : null) },
    );
    env.setDevice(async () => {
      throw failure;
    });
    await assert.rejects(
      env.resolver.resolveLoginRequest("ABCDEFGH", "device"),
      { kind: "rate_limited", retryAfterMs: 12000 },
    );
    await assert.rejects(
      env.resolver.resolveLoginRequest("ABCDEFGH", "device"),
      { kind: "rate_limited" },
    );
    assert.deepEqual(env.calls, ["device"]);
    env.advance(12000);
    env.setDevice(async () => preview());
    await env.resolver.resolveLoginRequest("ABCDEFGH", "device");
    assert.deepEqual(env.calls, ["device", "device"]);
  }
  const env = setup();
  env.setDevice(async () => {
    throw { statusCode: 429 };
  });
  await assert.rejects(env.resolver.resolveLoginRequest("ABCDEFGH", "device"), {
    kind: "rate_limited",
    retryAfterMs: 5000,
  });
});

test("manual device and scanner entries route Agent Key codes with a one-shot preview and no third request", async () => {
  for (const scanner of [false, true]) {
    const env = setup();
    env.setDevice(async () => {
      throw missing("device");
    });
    env.mount("device", { start_scanner: scanner });
    if (scanner) {
      const node = env.nodes().find((e) => e.type === "DeviceCodeScanner")!;
      (node.props.onManualEntry as () => void)();
      env.harness.flush();
    }
    // ManualCodeModal remains a component boundary here, so drive its explicit submit props.
    if (scanner) {
      const modal = env.nodes().find((e) => typeof e.type === "function")!;
      (modal.props.onChange as (s: string) => void)("ABCDEFGH");
      env.harness.flush();
      (
        env.nodes().find((e) => typeof e.type === "function")!.props
          .onSubmit as () => void
      )();
      await env.harness.settle();
    } else {
      env.field("ABCDEFGH");
      await env.press("Continue");
    }
    assert.deepEqual(env.calls.slice(0, 2), ["device", "agent-key"]);
    assert.ok(env.calls.some((c) => c.startsWith("navigate:AgentKeyLogin:")));
    const handoff = env.resolver.takeLoginRequest("ABCDEFGH", "agent-key");
    assert.equal(handoff?.result.kind, "agent-key");
    assert.equal(env.resolver.takeLoginRequest("ABCDEFGH", "agent-key"), null);
    assert.equal(env.calls.filter((c) => c === "agent-key").length, 1);
  }
});

test("Agent Key manual entry resolves device codes; device links preview and Agent Key links prefill", async () => {
  const env = setup();
  env.setAgent(async () => {
    throw missing("agent-key");
  });
  env.mount("agent-key");
  env.field("ABCDEFGH");
  await env.press("Continue");
  assert.deepEqual(env.calls.slice(0, 2), ["agent-key", "device"]);
  assert.ok(env.calls.some((c) => c.startsWith("navigate:DeviceLogin:")));
  for (const kind of ["device", "agent-key"]) {
    const linked = setup();
    linked.mount(kind, { user_code: "ABCDEFGH" });
    await linked.harness.settle();
    assert.deepEqual(linked.calls, kind === "device" ? ["device"] : []);
    linked.harness.unmount();
  }
});

test("handoff expires in ten seconds, consumes once, and review requires a separate throttled decision", async () => {
  const env = setup();
  env.resolver.handoffLoginRequest("ABCDEFGH", {
    kind: "device",
    preview: preview(),
  });
  env.mount("device", { user_code: "ABCDEFGH" });
  assert.deepEqual(env.calls, []);
  await env.press("Approve full account session");
  assert.deepEqual(env.calls, []);
  env.advance(750);
  await env.press("Approve full account session");
  assert.deepEqual(env.calls, ["approve"]);
  const expired = setup();
  expired.resolver.handoffLoginRequest("ABCDEFGH", {
    kind: "device",
    preview: preview(),
  });
  expired.advance(10001);
  assert.equal(expired.resolver.takeLoginRequest("ABCDEFGH", "device"), null);
});

for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} terminal previews, expiry and restart hide every decision control`, async () => {
    for (const status of ["expired", "approved", "delivered", "denied"]) {
      const env = setup();
      env.setDevice(async () => preview(status));
      env.setAgent(async () => preview(status));
      env.mount(kind, { user_code: "ABCDEFGH" });
      if (kind === "device") await env.harness.settle();
      else await env.press("Continue");
      assert.equal(env.harness.value().type, "Terminal");
      assert.equal(
        env.harness.value().props.state,
        status === "approved" || status === "delivered" ? "completed" : status,
      );
      (env.harness.value().props.onRestart as () => void)();
      env.harness.flush();
      assert.ok(env.nodes().some((e) => e.props.label === "Continue"));
      env.harness.unmount();
    }
    const env = setup();
    env.mount(kind);
    env.field("ABCDEFGH");
    await env.press("Continue");
    env.advance(60000);
    assert.equal(env.harness.value().type, "Terminal");
    assert.equal(env.harness.value().props.state, "expired");
    env.harness.unmount();
  });
  test(`${kind} foreground preview runs once per transition and renders terminal status`, async () => {
    const env = setup();
    env.mount(kind);
    env.field("ABCDEFGH");
    await env.press("Continue");
    env.advance(750);
    await env.app("background");
    await env.app("active");
    await env.app("active");
    assert.deepEqual(env.calls, [kind, kind]);
    env.setDevice(async () => preview("denied"));
    env.setAgent(async () => preview("denied"));
    env.advance(750);
    await env.app("background");
    await env.app("active");
    assert.equal(env.harness.value().type, "Terminal");
    assert.equal(env.harness.value().props.state, "denied");
    env.harness.unmount();
  });
}

test("foreground previews preserve the shared 750ms throttle and cancel a delayed refresh on background", async () => {
  for (const kind of ["device", "agent-key"] as const) {
    const env = setup();
    env.mount(kind);
    env.field("ABCDEFGH");
    await env.press("Continue");
    await env.app("background");
    await env.app("active");
    env.advance(749);
    await env.app("active");
    assert.deepEqual(env.calls, [kind]);
    env.advance(1);
    await env.harness.settle();
    assert.deepEqual(env.calls, [kind, kind]);
    await env.press(
      kind === "device" ? "Approve full account session" : "Reject",
    );
    assert.deepEqual(env.calls, [kind, kind]);
    await env.app("background");
    await env.app("active");
    await env.app("background");
    env.advance(750);
    await env.harness.settle();
    assert.deepEqual(env.calls, [kind, kind]);
    env.harness.unmount();
  }
});

test("device terminal decision errors remove all actions and foreground never previews in-flight decisions", async () => {
  for (const code of [11200, 11201, 11204, 11205, 11207]) {
    const env = setup();
    env.mount();
    env.field("ABCDEFGH");
    await env.press("Continue");
    env.advance(750);
    const pending = deferred<void>();
    env.setDecision(() => pending.promise);
    await env.press("Approve full account session");
    await env.app("background");
    await env.app("active");
    assert.deepEqual(env.calls, ["device", "approve"]);
    pending.reject({ errorCode: code });
    await env.harness.settle();
    assert.equal(env.harness.value().type, "Terminal");
    assert.equal(
      env.harness.value().props.state,
      code === 11201
        ? "expired"
        : code === 11204
          ? "denied"
          : code === 11205
            ? "completed"
            : "unavailable",
    );
    env.harness.unmount();
  }
});

test("manual handoffs render the already-fetched review on the receiving screen without another preview", async () => {
  for (const kind of ["device", "agent-key"] as const) {
    const source = setup();
    if (kind === "device")
      source.setAgent(async () => {
        throw missing("agent-key");
      });
    else
      source.setDevice(async () => {
        throw missing("device");
      });
    const result = await source.resolver.resolveLoginRequest(
      "ABCDEFGH",
      kind === "device" ? "agent-key" : "device",
    );
    source.resolver.handoffLoginRequest("ABCDEFGH", result);
    const destination = setup(source.resolver);
    destination.mount(kind, { user_code: "ABCDEFGH" });
    assert.deepEqual(destination.calls, []);
    assert.ok(
      destination
        .nodes()
        .some(
          (node) =>
            node.props.label ===
            (kind === "device"
              ? "Approve full account session"
              : "Choose an Agent Key"),
        ),
    );
    assert.deepEqual(
      source.calls,
      kind === "device" ? ["agent-key", "device"] : ["device", "agent-key"],
    );
    destination.harness.unmount();
  }
});

test("Continue is disabled during a server cooldown on both manual entry pages", async () => {
  for (const kind of ["device", "agent-key"]) {
    const env = setup();
    const failure = async () => {
      throw { statusCode: 429, errorCode: 1005, retryAfterMs: 1500 };
    };
    env.setDevice(failure);
    env.setAgent(failure);
    env.mount(kind);
    env.field("ABCDEFGH");
    await env.press("Continue");
    assert.equal(
      env.nodes().find((node) => node.props.label === "Continue")?.props
        .disabled,
      true,
    );
    assert.deepEqual(env.calls, [kind]);
    assert.ok(
      env
        .nodes()
        .some((node) =>
          String(node.props.children).includes("Too many attempts"),
        ),
    );
    env.harness.unmount();
  }
});

test("Agent Key terminal decision errors hide actions and foreground cannot preview while Reject is in flight", async () => {
  for (const errorCode of [11900, 11901, 11904, 11905, 11907]) {
    const env = setup();
    env.mount("agent-key");
    env.field("ABCDEFGH");
    await env.press("Continue");
    env.advance(750);
    const decision = deferred<void>();
    env.setDecision(() => decision.promise);
    await env.press("Reject");
    await env.app("background");
    await env.app("active");
    assert.deepEqual(env.calls, ["agent-key", "deny"]);
    decision.reject({ errorCode });
    await env.harness.settle();
    assert.equal(env.harness.value().type, "Terminal");
    assert.equal(
      env.harness.value().props.state,
      errorCode === 11901
        ? "expired"
        : errorCode === 11904
          ? "denied"
          : errorCode === 11905
            ? "completed"
            : "unavailable",
    );
    env.harness.unmount();
  }
});

test("Retry-After HTTP dates survive structured error parsing", () => {
  const delay = 30_000;
  const now = Date.now();
  const header = new Date(now + delay).toUTCString();
  const error = buildApiError({}, 429, {
    get: (name) => (name === "Retry-After" ? header : null),
  });
  assert.ok(
    error.retryAfterMs !== undefined &&
      error.retryAfterMs > delay - 2000 &&
      error.retryAfterMs <= delay,
  );
});

test("both login screens recheck an elapsed deadline immediately on foreground without another preview", async () => {
  for (const kind of ["device", "agent-key"]) {
    const env = setup();
    env.mount(kind);
    env.field("ABCDEFGH");
    await env.press("Continue");
    await env.app("background");
    env.elapse(60_000);
    await env.app("active");
    assert.equal(env.harness.value().type, "Terminal");
    assert.equal(env.harness.value().props.state, "expired");
    assert.deepEqual(env.calls, [kind]);
    env.harness.unmount();
  }
});

test("manual entry reports malformed codes locally and unknown valid codes name the configured server", async () => {
  for (const kind of ["device", "agent-key"]) {
    const invalid = setup();
    invalid.mount(kind);
    invalid.field("bad");
    (
      invalid.nodes().find((node) => node.type === "TextInput")!.props
        .onSubmitEditing as () => void
    )();
    await invalid.harness.settle();
    assert.deepEqual(invalid.calls, []);
    assert.ok(
      invalid
        .nodes()
        .some(
          (node) =>
            node.props.children === "Enter a valid eight-character login code.",
        ),
    );
    invalid.harness.unmount();
    const unknown = setup();
    unknown.setDevice(async () => {
      throw missing("device");
    });
    unknown.setAgent(async () => {
      throw missing("agent-key");
    });
    unknown.mount(kind);
    unknown.field("ABCDEFGH");
    await unknown.press("Continue");
    assert.ok(
      unknown
        .nodes()
        .some((node) =>
          String(node.props.children).includes(
            "No login request was found on login.example.test",
          ),
        ),
    );
    unknown.harness.unmount();
  }
});

test("expiry waits for an in-flight decision and restart cannot discard its accepted result", async () => {
  for (const kind of ["device", "agent-key"]) {
    const env = setup();
    env.mount(kind);
    env.field("ABCDEFGH");
    await env.press("Continue");
    env.advance(750);
    const decision = deferred<void>();
    env.setDecision(() => decision.promise);
    await env.press(
      kind === "device" ? "Approve full account session" : "Reject",
    );
    env.advance(60_000);
    assert.notEqual(env.harness.value().type, "Terminal");
    assert.equal(env.navigationDisabled(), true);
    decision.resolve();
    await env.harness.settle();
    assert.equal(
      env.harness.value().props.state,
      kind === "device" ? "approved" : "denied",
    );
    assert.equal(env.navigationDisabled(), false);
    (env.harness.value().props.onRestart as () => void)();
    env.harness.flush();
    env.field("EFGHABCD");
    await env.press("Continue");
    assert.notEqual(env.harness.value().type, "Terminal");
    env.harness.unmount();
  }
});

for (const entry of ["scanner modal", "device inline", "agent-key"] as const) {
  test(`${entry} cooldown keeps editable code and rate-limit copy with an idle disabled Continue`, async () => {
    const env = setup(undefined, entry === "scanner modal");
    const request = deferred<ReturnType<typeof preview>>();
    env.setDevice(() => request.promise);
    env.setAgent(() => request.promise);
    env.mount(entry === "agent-key" ? "agent-key" : "device", {
      start_scanner: entry === "scanner modal",
    });
    if (entry === "scanner modal") {
      (
        env.nodes().find((node) => node.type === "DeviceCodeScanner")!.props
          .onManualEntry as () => void
      )();
      env.harness.flush();
    }
    env.field("ABCDEFGH");
    await env.press("Continue");
    const submit = () =>
      env
        .nodes()
        .find((node) => node.type === "Button" && "loading" in node.props)!;
    assert.equal(
      submit().props.loading,
      true,
      "a live request still shows loading",
    );
    request.reject({ statusCode: 429, errorCode: 1005, retryAfterMs: 1500 });
    await env.harness.settle();
    const assertCooldown = () => {
      assert.equal(submit().props.loading, false);
      assert.equal(submit().props.label, "Continue");
      assert.equal(submit().props.disabled, true);
      assert.equal(
        env.nodes().find((node) => node.type === "TextInput")!.props.editable,
        true,
      );
      assert.ok(
        env
          .nodes()
          .some(
            (node) =>
              node.props.children ===
              "Too many attempts. Wait before trying again.",
          ),
      );
    };
    assertCooldown();
    env.field("EFGHABCD");
    assertCooldown();
    // Native keyboard submission and an old press callback cannot replace the
    // server's rate-limit explanation or issue another request during cooldown.
    (
      env.nodes().find((node) => node.type === "TextInput")!.props
        .onSubmitEditing as () => void
    )();
    await env.press("Continue");
    assertCooldown();
    assert.deepEqual(env.calls, [
      entry === "agent-key" ? "agent-key" : "device",
    ]);
    env.advance(1500);
    assert.equal(submit().props.disabled, false);
    assert.equal(submit().props.loading, false);
    env.harness.unmount();
  });
}

test("Agent Key foreground pending refresh preserves confirm selection, summary and expiry; terminal status ends review", async () => {
  const env = setup();
  env.mount("agent-key");
  env.field("ABCDEFGH");
  await env.press("Continue");
  env.advance(750);
  await env.press("Choose an Agent Key");
  const picker = env
    .nodes()
    .find((node) => node.type === "WorkspaceSelectionField")!;
  const keyId = (picker.props.options as Array<{ value: string }>)[0]!.value;
  (picker.props.onChange as (ids: string[]) => void)([keyId]);
  env.harness.flush();
  const expiry = "2090-01-01T00:00:00Z";
  env.field(expiry);
  const assertConfirm = () => {
    assert.ok(env.nodes().some((node) => node.props.label === "Approve"));
    assert.ok(
      env
        .nodes()
        .some(
          (node) =>
            node.props.label === "Key" && node.props.value === "Reviewed key",
        ),
    );
    assert.equal(
      env
        .nodes()
        .find((node) => node.props.accessibilityLabel === "Credential expiry")!
        .props.value,
      expiry,
    );
  };
  assertConfirm();
  env.advance(750);
  env.setAgent(async () => ({
    ...preview("pending", 120),
    client_label: "Refreshed requester",
  }));
  await env.app("background");
  await env.app("active");
  assertConfirm();
  assert.ok(
    env
      .nodes()
      .some(
        (node) =>
          node.props.label === "Expires" &&
          String(node.props.value).endsWith("(120s)"),
      ),
  );
  await env.press("Back to keys");
  assert.equal(
    env.nodes().find((node) => node.type === "WorkspaceSelectionField")!.props
      .summary,
    "Reviewed key",
  );
  (
    env.nodes().find((node) => node.type === "WorkspaceSelectionField")!.props
      .onChange as (ids: string[]) => void
  )([keyId]);
  env.harness.flush();
  assertConfirm();
  env.advance(750);
  env.setAgent(async () => preview("denied"));
  await env.app("background");
  await env.app("active");
  assert.equal(env.harness.value().type, "Terminal");
  assert.equal(env.harness.value().props.state, "denied");
  assert.equal(env.approvals.length, 0);
  env.harness.unmount();
});

test("Agent Key options and new-key steps survive pending foreground refresh without remounting their review", async () => {
  const env = setup();
  env.mount("agent-key");
  env.field("ABCDEFGH");
  await env.press("Continue");
  env.advance(750);
  await env.press("Choose an Agent Key");
  env.advance(750);
  await env.app("background");
  await env.app("active");
  assert.ok(
    env.nodes().some((node) => node.type === "WorkspaceSelectionField"),
  );
  await env.press("Create a new key");
  const form = env.nodes().find((node) => node.type === "AgentKeyCreateForm")!;
  env.advance(750);
  await env.app("background");
  await env.app("active");
  assert.equal(
    env.nodes().find((node) => node.type === "AgentKeyCreateForm")!.type,
    form.type,
  );
  assert.equal(
    env.nodes().find((node) => node.type === "AgentKeyCreateForm")!.props
      .options,
    form.props.options,
  );
  assert.ok(
    !env.nodes().some((node) => node.props.label === "Choose an Agent Key"),
  );
  env.harness.unmount();
});

test("Device Login pending foreground refresh preserves the expanded raw user agent", async () => {
  const env = setup();
  env.mount();
  env.field("ABCDEFGH");
  await env.press("Continue");
  (
    env
      .nodes()
      .find((node) => node.props.accessibilityLabel === "Show raw user agent")!
      .props.onPress as () => void
  )();
  env.harness.flush();
  env.advance(750);
  await env.app("background");
  await env.app("active");
  assert.ok(
    env
      .nodes()
      .some((node) => node.props.accessibilityLabel === "Hide raw user agent"),
  );
  env.advance(750);
  env.setDevice(async () => preview("expired"));
  await env.app("background");
  await env.app("active");
  assert.equal(env.harness.value().props.state, "expired");
  env.harness.unmount();
});

test("Agent Key restart clears the restricted device flow and resolves the next Agent Key code generically", async () => {
  const env = setup();
  env.mount("agent-key", { flow: "device", user_code: "ABCDEFGH" });
  assert.deepEqual(env.calls, [], "deep links still only prefill");
  env.setDevice(async () => preview("expired"));
  await env.press("Continue");
  assert.equal(env.harness.value().props.state, "expired");
  (env.harness.value().props.onRestart as () => void)();
  env.harness.flush();
  assert.equal(env.params().flow, undefined);
  assert.equal(env.params().user_code, undefined);
  env.advance(750);
  env.field("EFGHABCD");
  await env.press("Continue");
  assert.deepEqual(env.calls, ["device", "agent-key"]);
  assert.ok(
    env.nodes().some((node) => node.props.label === "Choose an Agent Key"),
  );
  env.harness.unmount();
});


for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} preview reads leave navigation available while decisions retain their critical guard`, async () => {
    const env = setup();
    const reading = deferred<ReturnType<typeof preview>>();
    if (kind === "device") env.setDevice(() => reading.promise);
    else env.setAgent(() => reading.promise);
    env.mount(kind);
    env.field("ABCDEFGH");
    const request = env.press("Continue");
    await env.harness.settle();
    assert.equal(env.navigationDisabled(), false, "preview is a read");
    reading.resolve(preview());
    await request;
    await env.harness.settle();
    env.advance(750);
    const writing = deferred<void>();
    env.setDecision(() => writing.promise);
    const deny = env.press("Reject");
    await env.harness.settle();
    assert.equal(
      env.navigationDisabled(),
      true,
      "decision writes cannot be interrupted",
    );
    writing.resolve();
    await deny;
    await env.harness.settle();
    assert.equal(env.navigationDisabled(), false);
    env.harness.unmount();
  });
}

for (const kind of ["device", "agent-key"] as const) {
  for (const outcome of ["approved", "expired", "network"] as const) {
    test(`${kind} linked decision across the deadline settles as ${outcome} and releases the critical guard`, async () => {
      const env = setup();
      env.synchronousParams();
      env.mount(kind, { user_code: "ABCDEFGH" });
      if (kind === "device") await env.harness.settle();
      else await env.press("Continue");
      if (kind === "agent-key") {
        env.advance(750);
        await env.press("Choose an Agent Key");
        const picker = env
          .nodes()
          .find((node) => node.type === "WorkspaceSelectionField")!;
        (picker.props.onChange as (ids: string[]) => void)([
          (picker.props.options as { value: string }[])[0]!.value,
        ]);
        env.harness.flush();
      }
      env.advance(750);
      const result = deferred<void>();
      env.setDecision(() => result.promise);
      await env.press(
        kind === "device" ? "Approve full account session" : "Approve",
      );
      assert.equal(env.navigationDisabled(), true);
      env.advance(60_000);
      await env.app("background");
      await env.app("active");
      assert.notEqual(env.harness.value().type, "Terminal");
      assert.equal(
        env.calls.filter((call) => call === kind).length,
        1,
        "no foreground preview during a write",
      );
      if (outcome === "approved") result.resolve();
      else
        result.reject(
          outcome === "expired"
            ? { errorCode: kind === "device" ? 11201 : 11901 }
            : networkApiError(new TypeError("fetch failed")),
        );
      await env.harness.settle();
      assert.equal(env.navigationDisabled(), false);
      if (outcome === "network") {
        env.advance(2000);
        assert.notEqual(env.harness.value().type, "Terminal");
        assert.ok(
          env.nodes().some((node) => node.props.accessibilityRole === "alert"),
        );
      } else {
        assert.equal(env.harness.value().props.state, outcome);
        assert.equal(env.harness.value().props.disabled, false);
        (env.harness.value().props.onDone as () => void)();
        assert.ok(env.calls.includes("back"));
      }
      env.harness.unmount();
    });
  }
  test(`${kind} a failed foreground read never clears a terminal reached while that read was pending`, async () => {
    for (const failure of [
      networkApiError(new Error("offline")),
      { statusCode: 503 },
      { statusCode: 429 },
    ]) {
      const env = setup();
      env.mount(kind, { user_code: "ABCDEFGH" });
      if (kind === "device") await env.harness.settle();
      else await env.press("Continue");
      env.advance(750);
      const refresh = deferred<ReturnType<typeof preview>>();
      env.setDevice(() => refresh.promise);
      env.setAgent(() => refresh.promise);
      await env.app("background");
      await env.app("active");
      env.advance(60_000);
      if (kind === "agent-key") {
        // A new linked request can become terminal while the old read awaits.
        env.resolver.handoffLoginRequest("EFGHABCD", {
          kind,
          preview: preview("expired"),
        });
        env.params().user_code = "EFGHABCD";
        env.harness.render();
        assert.equal(env.harness.value().props.state, "expired");
        refresh.reject(failure);
        await env.harness.settle();
      } else {
        assert.equal(env.harness.value().props.state, "expired");
        refresh.reject(failure);
        await env.harness.settle();
      }
      assert.equal(env.harness.value().props.state, "expired");
      assert.equal(env.navigationDisabled(), false);
      env.harness.unmount();
    }
  });
}

test("login failures distinguish transport errors from server, parse and unknown-code failures", () => {
  const env = setup();
  for (const failure of [
    buildApiError(null, 503),
    buildApiError(null, 200),
    { statusCode: 400, errorCode: 987654 },
    new SyntaxError("bad JSON"),
    new Error("unknown"),
  ]) {
    const rejected = (failure as { statusCode?: number }).statusCode === 400;
    assert.equal(env.resolver.classifyLoginError(failure), rejected ? "rejected" : "server");
    assert.equal(
      env.resolver.loginErrorCopy(failure),
      "NyxID couldn't complete this request. Try again.",
    );
  }
  const transport = networkApiError(new TypeError("fetch failed"));
  assert.equal(env.resolver.classifyLoginError(transport), "network");
  assert.match(env.resolver.loginErrorCopy(transport), /Check your connection/);
});

test("preview Retry-After cannot lock Continue for more than five minutes", () => {
  for (const retryAfterMs of [300_000, 999_999_999, Number.MAX_VALUE]) {
    const env = setup();
    env.resolver.recordLoginPreviewFailure({ statusCode: 429, retryAfterMs });
    assert.equal(env.resolver.previewCooldownMs(), 300_000);
    env.advance(300_000);
    assert.equal(env.resolver.previewCooldownMs(), 0);
  }
});

test("Agent Key review localizes a cooldown failure instead of showing the rate_limited identifier", async () => {
  const env = setup();
  env.mount("agent-key", { user_code: "ABCDEFGH" });
  await env.press("Continue");
  env.advance(750);
  env.resolver.recordLoginPreviewFailure({ statusCode: 429 });
  await env.app("background");
  await env.app("active");
  assert.ok(
    env
      .nodes()
      .some(
        (node) =>
          node.props.children ===
          "Too many attempts. Wait before trying again.",
      ),
  );
  assert.ok(
    !env.nodes().some((node) => node.props.children === "rate_limited"),
  );
  env.harness.unmount();
});

for (const boundary of ["blur", "unmount", "timeout"] as const) {
  test(`unconsumed navigation handoff is cleared on ${boundary} before a later same-code deep link`, async () => {
    const env = setup();
    env.setDevice(async () => {
      throw missing("device");
    });
    env.mount("device");
    env.field("ABCDEFGH");
    await env.press("Continue");
    assert.ok(
      env.calls.some((call) => call.startsWith("navigate:AgentKeyLogin")),
    );
    if (boundary === "blur") env.blur();
    else if (boundary === "unmount") env.harness.unmount();
    env.advance(boundary === "timeout" ? 10_000 : 0);
    const later = setup(env.resolver);
    later.mount("agent-key", { user_code: "ABCDEFGH" });
    assert.ok(later.nodes().some((node) => node.props.label === "Continue"));
    assert.deepEqual(later.calls, []);
    assert.equal(env.resolver.takeLoginRequest("ABCDEFGH", "agent-key"), null);
    later.harness.unmount();
    if (boundary !== "unmount") env.harness.unmount();
  });
}

for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} stale terminal Done and restart callbacks cannot discard a newer in-flight decision`, async () => {
    const env = setup();
    env.mount(kind, { user_code: "ABCDEFGH" });
    if (kind === "device") await env.harness.settle();
    else await env.press("Continue");
    env.advance(60_000);
    const terminal = env.harness.value();
    assert.equal(terminal.type, "Terminal");
    (terminal.props.onRestart as () => void)();
    env.harness.flush();
    env.field("EFGHABCD");
    await env.press("Continue");
    env.advance(750);
    const result = deferred<void>();
    env.setDecision(() => result.promise);
    await env.press("Reject");
    (terminal.props.onRestart as () => void)();
    (terminal.props.onDone as () => void)();
    env.harness.flush();
    assert.equal(env.navigationDisabled(), true);
    assert.ok(!env.calls.includes("back"));
    assert.notEqual(env.harness.value().type, "Terminal");
    result.resolve();
    await env.harness.settle();
    assert.equal(env.harness.value().props.state, "denied");
    assert.equal(env.navigationDisabled(), false);
    env.harness.unmount();
  });
}

test("a preview finishing after source blur cannot publish a handoff or navigate", async () => {
  const env = setup();
  env.setDevice(async () => {
    throw missing("device");
  });
  const request = deferred<ReturnType<typeof preview>>();
  env.setAgent(() => request.promise);
  env.mount("device");
  env.field("ABCDEFGH");
  await env.press("Continue");
  env.blur();
  request.resolve(preview());
  await env.harness.settle();
  assert.ok(!env.calls.some((call) => call.startsWith("navigate:")));
  assert.equal(env.resolver.takeLoginRequest("ABCDEFGH", "agent-key"), null);
  env.harness.unmount();
});

test("Restricted Agent Key linked denial waits through expiry and clearing user_code releases every write guard", async () => {
  const env = setup();
  env.synchronousParams();
  env.mount("agent-key", { user_code: "ABCDEFGH", flow: "device" });
  await env.press("Continue");
  env.advance(750);
  const decision = deferred<void>();
  env.setDecision(() => decision.promise);
  await env.press("Reject");
  env.advance(60_000);
  assert.notEqual(env.harness.value().type, "Terminal");
  assert.equal(env.navigationDisabled(), true);
  decision.resolve();
  await env.harness.settle();
  assert.equal(env.harness.value().props.state, "denied");
  assert.equal(env.params().user_code, undefined);
  assert.equal(env.navigationDisabled(), false);
  (env.harness.value().props.onRestart as () => void)();
  env.harness.flush();
  assert.equal(env.params().flow, undefined);
  assert.ok(env.nodes().some((node) => node.props.label === "Continue"));
  env.harness.unmount();
});

test("destination layout consumes a manual handoff before the source blur releases ownership", async () => {
  const source = setup();
  source.setDevice(async () => {
    throw missing("device");
  });
  source.mount("device");
  source.field("ABCDEFGH");
  await source.press("Continue");
  const destination = setup(source.resolver);
  destination.mount("agent-key", { user_code: "ABCDEFGH" });
  source.blur();
  source.harness.unmount();
  assert.ok(
    destination
      .nodes()
      .some((node) => node.props.label === "Choose an Agent Key"),
  );
  assert.deepEqual(destination.calls, []);
  assert.equal(source.resolver.takeLoginRequest("ABCDEFGH", "agent-key"), null);
  destination.harness.unmount();
});

for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} a route prefill change cannot reset an in-flight decision or discard its server result`, async () => {
    const env = setup();
    env.synchronousParams();
    env.mount(kind, { user_code: "ABCDEFGH" });
    if (kind === "device") await env.harness.settle();
    else await env.press("Continue");
    env.advance(750);
    const result = deferred<void>();
    env.setDecision(() => result.promise);
    await env.press("Reject");
    env.params().user_code = "EFGHABCD";
    env.harness.render();
    assert.equal(env.navigationDisabled(), true);
    result.resolve();
    await env.harness.settle();
    assert.ok(env.renders.some(render => render.type === "Terminal" && render.props.state === "denied"));
    assert.equal(env.params().user_code, "EFGHABCD");
    assert.equal(env.harness.value().props.state, "denied");
    (env.harness.value().props.onRestart as () => void)();
    env.harness.flush();
    assert.equal(env.nodes().find(node => node.type === "TextInput")?.props.value, "EFGH-ABCD");
    assert.deepEqual(env.calls, [kind, "deny"]);
    assert.equal(env.navigationDisabled(), false);
    env.harness.unmount();
  });
}

for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} server, response-parse and definitive rejection failures preserve their copy without probing another endpoint`, async () => {
    for (const failure of [
      buildApiError(null, 503),
      new SyntaxError("invalid response JSON"),
      { statusCode: 400, errorCode: 987654 },
    ]) {
      const env = setup();
      env.setDevice(async () => {
        throw failure;
      });
      env.setAgent(async () => {
        throw failure;
      });
      env.mount(kind);
      env.field("ABCDEFGH");
      await env.press("Continue");
      assert.deepEqual(env.calls, [kind]);
      assert.ok(
        env
          .nodes()
          .some(
            (node) =>
              node.props.children ===
              "NyxID couldn't complete this request. Try again.",
          ),
      );
      assert.ok(
        !env
          .nodes()
          .some((node) =>
            String(node.props.children).includes("Check your connection"),
          ),
      );
      env.harness.unmount();
    }
  });
}

for (const kind of ["device", "agent-key"] as const) {
  for (const outcome of [
    "approved",
    "expired",
    "denied",
    "completed",
    "unavailable",
    "network",
    "step-up",
  ] as const) {
    for (const exit of ["onDone", "onRestart"] as const)
    test(`${kind} newest deferred link waits for ${outcome} and ${exit} before its own review, without inheriting a decision`, async () => {
      const env = setup();
      env.synchronousParams();
      env.mount(kind, { user_code: "ABCDEFGH" });
      if (kind === "device") await env.harness.settle();
      else await env.press("Continue");
      if (kind === "agent-key") {
        env.advance(750);
        await env.press("Choose an Agent Key");
        const picker = env
          .nodes()
          .find((node) => node.type === "WorkspaceSelectionField")!;
        (picker.props.onChange as (ids: string[]) => void)([
          (picker.props.options as { value: string }[])[0]!.value,
        ]);
        env.harness.flush();
      }
      env.advance(750);
      const result = deferred<void>();
      env.setDecision(() => result.promise);
      await env.press(
        kind === "device" ? "Approve full account session" : "Approve",
      );
      env.params().user_code = "DEFGHABC";
      env.harness.render();
      env.params().user_code = "EFGHABCD";
      env.harness.render();
      env.advance(60_000);
      assert.equal(env.navigationDisabled(), true);
      assert.ok(!env.renders.some((render) => render.type === "Terminal"));
      if (outcome === "approved") result.resolve();
      else
        result.reject(
          outcome === "network"
            ? networkApiError(new TypeError("fetch failed"))
            : outcome === "step-up"
              ? { errorCode: 2003 }
              : { errorCode: (kind === "device" ? 11200 : 11900) +
                  { expired: 1, denied: 4, completed: 5, unavailable: 0 }[outcome] },
        );
      await env.harness.settle();
      if (["approved", "expired", "denied", "completed", "unavailable"].includes(outcome)) {
        assert.ok(
          env.renders.some(
            (render) =>
              render.type === "Terminal" && render.props.state === outcome,
          ),
        );
        if (outcome === "approved")
          assert.ok(
            !env.renders.some((render) => render.props.state === "expired"),
          );
        assert.equal(env.harness.value().props.state, outcome, "hold A until the user exits its outcome");
        (env.harness.value().props[exit] as () => void)();
        env.harness.flush();
      } else {
        assert.ok(
          env.renders.some((render) =>
            elements(render).some((node) =>
              outcome === "step-up"
                ? node.type === "StepUpPrompt"
                : node.props.children ===
                  "Could not reach the login server. Check your connection and try again.",
            ),
          ),
        );
      }
      if (outcome === "network") {
        assert.ok(!env.nodes().some(node => node.props.label === "Continue"));
        assert.equal(env.nodes().find(node => node.props.label === (kind === "device" ? "Expires in" : "Expires"))?.props.value, "Waiting for NyxID…");
        await env.press(exit === "onDone" ? "Done" : "Scan or enter another code");
      }
      assert.equal(env.params().user_code, "EFGHABCD");
      assert.deepEqual(env.decisionCodes.map(codes.normalizeAuthDeviceUserCode), ["ABCDEFGH"]);
      assert.equal(env.navigationDisabled(), false);
      if (kind === "device") {
        await env.harness.settle();
        assert.deepEqual(env.calls, [kind, "approve", kind], "only a public preview follows release");
        assert.ok(env.nodes().some(node => node.props.value === "EFGH-ABCD"));
      } else {
        assert.equal(env.nodes().find(node => node.type === "TextInput")?.props.value, "EFGH-ABCD");
        assert.deepEqual(env.calls, [kind, "approve"]);
        await env.press("Continue");
      }
      // Review B, restart, and open the identical link again.
      env.setDecision(async () => {});
      env.advance(750);
      await env.press("Reject");
      assert.equal(env.harness.value().props.state, "denied");
      (env.harness.value().props.onRestart as () => void)();
      env.harness.flush();
      assert.equal(env.params().user_code, undefined);
      env.params().user_code = "EFGHABCD";
      env.harness.render();
      assert.equal(
        env.nodes().find((node) => node.type === "TextInput")?.props.value,
        "EFGH-ABCD",
      );
      if (kind === "device") {
        assert.deepEqual(env.calls, [kind, "approve", kind, "deny"], "replacement waits for the shared action throttle");
        await env.advanceAsync(750);
      }
      await env.harness.settle();
      assert.deepEqual(env.calls, kind === "device"
        ? [kind, "approve", kind, "deny", kind] : [kind, "approve", kind, "deny"]);
      assert.deepEqual(env.decisionCodes.map(codes.normalizeAuthDeviceUserCode), ["ABCDEFGH", "EFGHABCD"]);
      env.harness.unmount();
    });
  }
  test(`${kind} deferred links consume only a matching one-shot preview handoff`, async () => {
    for (const status of ["pending", "expired"]) {
      const env = setup();
      env.synchronousParams();
      env.mount(kind, { user_code: "ABCDEFGH" });
      if (kind === "device") await env.harness.settle();
      else await env.press("Continue");
      env.advance(750);
      const result = deferred<void>();
      env.setDecision(() => result.promise);
      await env.press("Reject");
      env.resolver.handoffLoginRequest("EFGHABCD", { kind, preview: preview(status) });
      env.params().user_code = "EFGHABCD";
      env.harness.render();
      result.resolve();
      await env.harness.settle();
      assert.ok(env.renders.some((render) => render.props.state === "denied"));
      assert.equal(env.harness.value().props.state, "denied", "matching handoff waits for terminal exit too");
      (env.harness.value().props.onRestart as () => void)();
      env.harness.flush();
      if (status === "pending") {
        assert.ok(env.nodes().some((node) => node.props.label ===
          (kind === "device" ? "Approve full account session" : "Choose an Agent Key")));
        assert.equal(env.params().user_code, "EFGHABCD");
      } else {
        assert.equal(env.harness.value().props.state, "expired");
        assert.equal(env.params().user_code, kind === "agent-key" ? undefined : "EFGHABCD");
      }
      assert.equal(env.resolver.takeLoginRequest("EFGHABCD", kind), null);
      assert.deepEqual(env.calls, [kind, "deny"]);
      assert.equal(env.navigationDisabled(), false);
      env.harness.unmount();
    }
  });
}

for (const expiry of ["tomorrow", "2099-01-01T00:00:00Z"]) {
  test(`Agent Key local credential-expiry validation (${expiry}) never suppresses the deadline without dispatch`, async () => {
    const env = setup();
    env.setKeyExpiry("2090-01-01T00:00:00Z");
    env.mount("agent-key", { user_code: "ABCDEFGH" });
    await env.press("Continue");
    env.advance(750);
    await env.press("Choose an Agent Key");
    const picker = env
      .nodes()
      .find((node) => node.type === "WorkspaceSelectionField")!;
    (picker.props.onChange as (ids: string[]) => void)([
      (picker.props.options as { value: string }[])[0]!.value,
    ]);
    env.harness.flush();
    env.field(expiry);
    env.advance(58_000);
    await env.press("Approve");
    const message =
      expiry === "tomorrow"
        ? "Enter an expiry as YYYY-MM-DD or an RFC 3339 timestamp with a timezone."
        : "Credential expiry cannot exceed key expiry.";
    assert.ok(env.nodes().some((node) => node.props.children === message));
    assert.equal(env.approvals.length, 0);
    assert.deepEqual(env.calls, ["agent-key"]);
    assert.equal(env.navigationDisabled(), false);
    env.advance(2000);
    assert.equal(env.harness.value().props.state, "expired");
    env.harness.unmount();
  });
}

async function startReviewedDecision(
  env: ReturnType<typeof setup>,
  kind: "device" | "agent-key",
  action: "approve" | "deny",
) {
  env.synchronousParams();
  env.mount(kind, { user_code: "ABCDEFGH" });
  if (kind === "device") await env.harness.settle();
  else await env.press("Continue");
  if (kind === "agent-key" && action === "approve") {
    env.advance(750);
    await env.press("Choose an Agent Key");
    const picker = env
      .nodes()
      .find((node) => node.type === "WorkspaceSelectionField")!;
    (picker.props.onChange as (ids: string[]) => void)([
      (picker.props.options as { value: string }[])[0]!.value,
    ]);
    env.harness.flush();
  }
  env.advance(750);
  await env.press(
    action === "deny"
      ? "Reject"
      : kind === "device"
        ? "Approve full account session"
        : "Approve",
  );
}

for (const kind of ["device", "agent-key"] as const) {
  for (const [action, status, expected] of [
    ["approve", "approved", "approved"],
    ["approve", "delivered", "approved"],
    ["deny", "denied", "denied"],
    ["deny", "approved", "completed"],
  ] as const) {
    test(`${kind} background transport failure keeps ${action} uncertain past expiry until resume preview ${status} decides ${expected}`, async () => {
      const env = setup();
      const write = deferred<void>();
      env.setDecision(() => write.promise);
      await startReviewedDecision(env, kind, action);
      await env.app("background");
      env.advance(60_000);
      const expiry = () =>
        env
          .nodes()
          .find(
            (node) =>
              node.props.label ===
              (kind === "device" ? "Expires in" : "Expires"),
          );
      assert.equal(
        expiry()?.props.value,
        "Waiting for NyxID…",
        "an in-flight decision cannot say Expired or 0s",
      );
      write.reject(buildApiError(null, 0));
      await env.harness.settle();
      assert.equal(
        env.stepUpCalls.length,
        0,
        "only MFA failures may enter lifecycle-sensitive step-up handling",
      );
      assert.equal(env.navigationDisabled(), false);
      assert.equal(expiry()?.props.value, "Waiting for NyxID…");
      const refresh = deferred<ReturnType<typeof preview>>();
      env.setDevice(() => refresh.promise);
      env.setAgent(() => refresh.promise);
      await env.app("active");
      assert.ok(
        !env.renders.some((render) => render.props.state === "expired"),
      );
      assert.deepEqual(env.calls, [kind, action, kind]);
      refresh.resolve(preview(status, 0));
      await env.harness.settle();
      assert.equal(env.harness.value().props.state, expected);
      assert.ok(
        !env.renders.some((render) => render.props.state === "expired"),
      );
      env.harness.unmount();
    });
  }

  test(`${kind} an uncertain decision shows neutral waiting copy even before the deadline`, async () => {
    const env = setup();
    env.setDecision(async () => {
      throw buildApiError(null, 503);
    });
    await startReviewedDecision(env, kind, "approve");
    const expiry = env
      .nodes()
      .find(
        (node) =>
          node.props.label === (kind === "device" ? "Expires in" : "Expires"),
      );
    assert.equal(expiry?.props.value, "Waiting for NyxID…");
    env.advance(60_000);
    assert.notEqual(env.harness.value().type, "Terminal");
    env.harness.unmount();
  });

  for (const [status, code] of [
    [400, 987654],
    [403, 987654],
    [404, 987654],
    [409, 987654],
    [422, 987654],
    [400, 11908],
    [404, 11909],
  ] as const) {
    test(`${kind} definitive decision rejection ${status}/${code} keeps local expiry enabled`, async () => {
      const env = setup();
      const failure = buildApiError(
        { error: "rejected", error_code: code, message: "Untrusted detail" },
        status,
      );
      env.setDecision(async () => {
        throw failure;
      });
      await startReviewedDecision(env, kind, "approve");
      const expected =
        kind === "agent-key"
          ? keyModel.agentKeyActionErrorCopy(failure, "confirm")
          : "NyxID couldn't complete this request. Try again.";
      assert.ok(env.nodes().some((node) => node.props.children === expected));
      assert.equal(env.stepUpCalls.length, 0);
      env.advance(60_000);
      assert.equal(env.harness.value().props.state, "expired");
      env.harness.unmount();
    });
  }

  test(`${kind} a held deferred link stays held when an even newer link arrives on the terminal screen`, async () => {
    const env = setup();
    const write = deferred<void>();
    env.setDecision(() => write.promise);
    await startReviewedDecision(env, kind, "approve");
    env.params().user_code = "DEFGHABC";
    env.harness.render();
    write.resolve();
    await env.harness.settle();
    assert.equal(env.harness.value().props.state, "approved");
    env.params().user_code = "EFGHABCD";
    env.harness.render();
    assert.equal(env.harness.value().props.state, "approved");
    (env.harness.value().props.onDone as () => void)();
    env.harness.flush();
    assert.equal(
      env.nodes().find((node) => node.type === "TextInput")?.props.value,
      "EFGH-ABCD",
    );
    assert.deepEqual(env.calls, [kind, "approve"]);
    assert.deepEqual(env.decisionCodes.map(codes.normalizeAuthDeviceUserCode), [
      "ABCDEFGH",
    ]);
    env.harness.unmount();
  });
}

async function uncertainLinkedDecision(
  kind: "device" | "agent-key",
  failure = buildApiError(null, 0),
) {
  const env = setup();
  const write = deferred<void>();
  env.setDecision(() => write.promise);
  await startReviewedDecision(env, kind, "approve");
  env.params().user_code = "EFGHABCD";
  env.harness.render();
  write.reject(failure);
  await env.harness.settle();
  return env;
}
function assertUncertainReview(
  env: ReturnType<typeof setup>,
  kind: "device" | "agent-key",
  message = "Could not reach the login server. Check your connection and try again.",
) {
  assert.notEqual(env.harness.value().type, "Terminal");
  assert.ok(!env.nodes().some((node) => node.props.label === "Continue"));
  assert.ok(env.nodes().some((node) => node.props.children === message));
  assert.equal(
    env
      .nodes()
      .find(
        (node) =>
          node.props.label === (kind === "device" ? "Expires in" : "Expires"),
      )?.props.value,
    "Waiting for NyxID…",
  );
  assert.equal(env.navigationDisabled(), false);
}
for (const kind of ["device", "agent-key"] as const) {
  for (const failure of [0, 503]) {
    for (const exit of ["Done", "Scan or enter another code"]) {
      test(`${kind} deferred link survives uncertain ${failure} with error and waiting copy until ${exit}`, async () => {
        const env = await uncertainLinkedDecision(
          kind,
          buildApiError(null, failure),
        );
        assertUncertainReview(
          env,
          kind,
          failure === 0
            ? undefined
            : "NyxID couldn't complete this request. Try again.",
        );
        assert.deepEqual(env.calls, [kind, "approve"]);
        await env.press(exit);
        assert.equal(
          env.nodes().find((node) => node.type === "TextInput")?.props.value,
          "EFGH-ABCD",
        );
        assert.deepEqual(
          env.calls,
          [kind, "approve"],
          "leaving uncertainty only prefills B",
        );
        env.harness.unmount();
      });
    }
  }
  for (const source of ["auto", "resume"]) {
    for (const status of ["approved", "pending"]) {
      test(`${kind} ${source} preview resolves held uncertainty to ${status} before releasing the deferred link`, async () => {
        const env = await uncertainLinkedDecision(kind);
        assertUncertainReview(env, kind);
        env.setDevice(async () => preview(status));
        env.setAgent(async () => preview(status));
        if (source === "auto") await env.advanceAsync(2000);
        else {
          await env.app("background");
          env.advance(60_000);
          await env.app("active");
        }
        if (status === "approved") {
          assert.equal(env.harness.value().props.state, "approved");
          assert.equal(env.params().user_code, "EFGHABCD");
          assert.ok(!env.nodes().some((node) => node.type === "TextInput"));
          (env.harness.value().props.onRestart as () => void)();
          env.harness.flush();
        }
        assert.equal(
          env.nodes().find((node) => node.type === "TextInput")?.props.value,
          "EFGH-ABCD",
        );
        assert.deepEqual(env.calls, [kind, "approve", kind]);
        await env.advanceAsync(60_000);
        assert.deepEqual(
          env.calls,
          kind === "device" ? [kind, "approve", kind, kind] : [kind, "approve", kind],
          "resolution ends uncertainty polling; a released device link gets one public preview",
        );
        env.harness.unmount();
      });
    }
  }

  for (const boundary of ["undefined", "same-code", "removed-code"] as const) {
    for (const exit of ["onDone", "onRestart"] as const) {
      test(`${kind} ${boundary} params during a decision never consume the first terminal ${exit} tap`, async () => {
        const env = setup();
        const write = deferred<void>();
        env.setDecision(() => write.promise);
        await startReviewedDecision(env, kind, "approve");
        if (boundary === "removed-code") {
          env.params().user_code = "EFGHABCD";
          env.harness.render();
        }
        env.params().user_code =
          boundary === "same-code" ? "abcd-efgh" : undefined;
        env.harness.render();
        write.resolve();
        await env.harness.settle();
        assert.equal(env.harness.value().props.state, "approved");
        (env.harness.value().props[exit] as () => void)();
        env.harness.flush();
        if (exit === "onDone")
          assert.deepEqual(env.calls, [kind, "approve", "back"]);
        else {
          assert.equal(
            env.nodes().find((node) => node.type === "TextInput")?.props.value,
            "",
          );
          assert.equal(env.params().user_code, undefined);
          assert.deepEqual(env.calls, [kind, "approve"]);
        }
        env.harness.unmount();
      });
    }
  }

  test(`${kind} uncertainty polling resolves to approved on its second bounded attempt`, async () => {
    const env = await uncertainLinkedDecision(kind);
    let attempts = 0;
    const read = async () => {
      if (++attempts === 1) throw buildApiError(null, 503);
      return preview("approved");
    };
    env.setDevice(read);
    env.setAgent(read);
    await env.advanceAsync(1999);
    assert.equal(attempts, 0);
    await env.advanceAsync(1);
    assert.equal(attempts, 1);
    assertUncertainReview(env, kind);
    await env.advanceAsync(3999);
    assert.equal(attempts, 1);
    await env.advanceAsync(1);
    assert.equal(attempts, 2);
    assert.equal(env.harness.value().props.state, "approved");
    await env.advanceAsync(120_000);
    assert.equal(attempts, 2);
    env.harness.unmount();
  });

  test(`${kind} uncertainty polling honors Retry-After before spending its next attempt`, async () => {
    const env = await uncertainLinkedDecision(kind);
    let attempts = 0;
    const read = async () => {
      if (++attempts === 1)
        throw buildApiError(null, 429, {
          get: (name) => (name === "Retry-After" ? "10" : null),
        });
      return preview("approved");
    };
    env.setDevice(read);
    env.setAgent(read);
    await env.advanceAsync(2000);
    assert.equal(attempts, 1);
    await env.advanceAsync(9999);
    assert.equal(
      attempts,
      1,
      "the 4-second backoff cannot bypass a 10-second cooldown",
    );
    await env.advanceAsync(1);
    assert.equal(attempts, 2);
    assert.equal(env.harness.value().props.state, "approved");
    env.harness.unmount();
  });

  for (const boundary of ["inactive", "background", "unmount"] as const) {
    test(`${kind} uncertainty polling stops on ${boundary}`, async () => {
      const env = await uncertainLinkedDecision(kind);
      if (boundary === "unmount") env.harness.unmount();
      else await env.app(boundary);
      await env.advanceAsync(120_000);
      assert.deepEqual(env.calls, [kind, "approve"]);
      if (boundary !== "unmount") env.harness.unmount();
    });
  }

  test(`${kind} uncertainty polling exhausts five attempts in 60 seconds and resume resets the budget`, async () => {
    const env = await uncertainLinkedDecision(kind);
    let attempts = 0;
    const read = async () => {
      attempts++;
      throw buildApiError(null, 503);
    };
    env.setDevice(read);
    env.setAgent(read);
    for (const [delay, count] of [
      [2000, 1],
      [4000, 2],
      [8000, 3],
      [16000, 4],
      [30000, 5],
    ] as const) {
      await env.advanceAsync(delay);
      assert.equal(attempts, count);
    }
    assertUncertainReview(env, kind);
    await env.advanceAsync(300_000);
    assert.equal(attempts, 5);
    await env.app("background");
    await env.app("active");
    assert.equal(attempts, 6, "resume still previews after budget exhaustion");
    await env.advanceAsync(2000);
    assert.equal(attempts, 7, "resume starts a fresh schedule");
    env.harness.unmount();
  });

  test(`${kind} a manual retry during uncertainty backoff never duplicates an automatic preview`, async () => {
    const env = await uncertainLinkedDecision(kind);
    await env.advanceAsync(1000);
    const retry = deferred<void>();
    env.setDecision(() => retry.promise);
    await env.press(
      kind === "device" ? "Approve full account session" : "Approve",
    );
    await env.advanceAsync(8000);
    assert.deepEqual(
      env.calls,
      [kind, "approve", "approve"],
      "no preview during the critical retry",
    );
    retry.reject(buildApiError(null, 0));
    await env.harness.settle();
    const read = deferred<ReturnType<typeof preview>>();
    env.setDevice(() => read.promise);
    env.setAgent(() => read.promise);
    await env.advanceAsync(1999);
    assert.deepEqual(env.calls, [kind, "approve", "approve"]);
    await env.advanceAsync(1);
    assert.deepEqual(env.calls, [kind, "approve", "approve", kind]);
    await env.advanceAsync(10_000);
    assert.deepEqual(
      env.calls,
      [kind, "approve", "approve", kind],
      "a slow preview cannot overlap another",
    );
    read.resolve(preview("approved"));
    await env.harness.settle();
    assert.equal(env.harness.value().props.state, "approved");
    env.harness.unmount();
  });

  test(`${kind} an automatic preview from an abandoned generation cannot replace a newly prefilled code`, async () => {
    const env = await uncertainLinkedDecision(kind);
    const read = deferred<ReturnType<typeof preview>>();
    env.setDevice(() => read.promise);
    env.setAgent(() => read.promise);
    await env.advanceAsync(2000);
    await env.press("Scan or enter another code");
    assert.equal(
      env.nodes().find((node) => node.type === "TextInput")?.props.value,
      "EFGH-ABCD",
    );
    read.resolve(preview("approved"));
    await env.harness.settle();
    assert.equal(
      env.nodes().find((node) => node.type === "TextInput")?.props.value,
      "EFGH-ABCD",
    );
    assert.ok(!env.renders.some((render) => render.props.state === "approved"));
    env.harness.unmount();
  });
}

test("Device Login expiry tick between decision finally and the uncertainty state commit cannot render expired", async () => {
  const env = setup();
  const write = deferred<void>();
  env.setDecision(() => write.promise);
  await startReviewedDecision(env, "device", "approve");
  env.elapse(60_000);
  write.reject(buildApiError(null, 0));
  // Let the API and decision finally finish without committing queued React state.
  await new Promise<void>((resolve) => setImmediate(resolve));
  env.tickBeforeCommit();
  env.harness.flush();
  assertUncertainReview(env, "device");
  assert.ok(!env.renders.some((render) => render.props.state === "expired"));
  env.harness.unmount();
});

for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} a pending automatic preview without a deferred link clears uncertainty and stops polling`, async () => {
    const env = setup();
    env.setDecision(async () => {
      throw buildApiError(null, 0);
    });
    await startReviewedDecision(env, kind, "approve");
    await env.advanceAsync(2000);
    assert.ok(
      !env
        .nodes()
        .some(
          (node) =>
            node.props.children ===
            "Could not reach the login server. Check your connection and try again.",
        ),
    );
    assert.notEqual(
      env
        .nodes()
        .find(
          (node) =>
            node.props.label === (kind === "device" ? "Expires in" : "Expires"),
        )?.props.value,
      "Waiting for NyxID…",
    );
    assert.deepEqual(env.calls, [kind, "approve", kind]);
    await env.advanceAsync(16_000);
    assert.deepEqual(env.calls, [kind, "approve", kind]);
    env.harness.unmount();
  });

  test(`${kind} a Retry-After beyond the automatic budget stops polling without losing the uncertainty error`, async () => {
    const env = await uncertainLinkedDecision(kind);
    let attempts = 0;
    const read = async () => {
      attempts++;
      throw buildApiError(null, 429, {
        get: (name) => (name === "Retry-After" ? "120" : null),
      });
    };
    env.setDevice(read);
    env.setAgent(read);
    await env.advanceAsync(300_000);
    assert.equal(attempts, 1);
    assertUncertainReview(env, kind);
    env.harness.unmount();
  });
}

for (const status of ["approved", "pending"]) {
  test(`Device Login expiry tick before a resolving ${status} preview commits preserves the server result`, async () => {
    const env = setup();
    env.setDecision(async () => {
      throw buildApiError(null, 0);
    });
    await startReviewedDecision(env, "device", "approve");
    const read = deferred<ReturnType<typeof preview>>();
    env.setDevice(() => read.promise);
    await env.advanceAsync(2000);
    env.elapse(60_000);
    read.resolve(preview(status));
    await new Promise<void>((resolve) => setImmediate(resolve));
    env.tickBeforeCommit();
    env.harness.flush();
    assert.ok(!env.renders.some((render) => render.props.state === "expired"));
    if (status === "approved")
      assert.equal(env.harness.value().props.state, "approved");
    else {
      assert.notEqual(env.harness.value().type, "Terminal");
      assert.notEqual(
        env.nodes().find((node) => node.props.label === "Expires in")?.props
          .value,
        "Waiting for NyxID…",
      );
    }
    env.harness.unmount();
  });
}

for (const kind of ["device", "agent-key"] as const) {
  test(`${kind} a newer link arriving during uncertainty stays held until Done`, async () => {
    const env = await uncertainLinkedDecision(kind);
    env.params().user_code = "DEFGHABC";
    env.harness.render();
    assertUncertainReview(env, kind);
    await env.press("Done");
    assert.equal(
      env.nodes().find((node) => node.type === "TextInput")?.props.value,
      "DEFG-HABC",
    );
    assert.deepEqual(env.calls, [kind, "approve"]);
    env.harness.unmount();
  });
}

test("Agent Key device-flow Continue preserves the requested profile through the Agent Key preview schema", async () => {
  const env = setup();
  env.setDevice(async () => ({ ...preview(), requested_profile: "work" }));
  env.mount("agent-key", { user_code: "ABCDEFGH", flow: "device" });
  await env.press("Continue");
  assert.equal(
    env.nodes().find((node) => node.props.label === "Requested profile")?.props
      .value,
    "work",
  );
  assert.deepEqual(env.calls, ["device"]);
  assert.equal(env.approvals.length, 0);
  env.harness.unmount();
});

for (const source of ["resume", "automatic"] as const) {
  test(`Agent Key device-flow ${source} refresh preserves the requested profile through the Agent Key preview schema`, async () => {
    const env = setup();
    env.mount("agent-key", { user_code: "ABCDEFGH", flow: "device" });
    await env.press("Continue");
    if (source === "automatic") {
      env.advance(750);
      await env.press("Choose an Agent Key");
      const picker = env
        .nodes()
        .find((node) => node.type === "WorkspaceSelectionField")!;
      (picker.props.onChange as (ids: string[]) => void)([
        (picker.props.options as { value: string }[])[0]!.value,
      ]);
      env.harness.flush();
      env.setDecision(async () => {
        throw buildApiError(null, 0);
      });
      env.advance(750);
      await env.press("Approve");
      assertUncertainReview(env, "agent-key");
    }
    env.setDevice(async () => ({ ...preview(), requested_profile: "work" }));
    if (source === "resume") {
      await env.app("background");
      env.advance(750);
      await env.app("active");
    } else await env.advanceAsync(2000);
    assert.equal(
      env.nodes().find((node) => node.props.label === "Requested profile")
        ?.props.value,
      "work",
    );
    assert.deepEqual(
      env.calls,
      source === "resume"
        ? ["device", "device"]
        : ["device", "approve", "device"],
    );
    env.harness.unmount();
  });
}

test("Agent Key device preview schema selection keeps shared Retry-After cooldown recording", async () => {
  const env = setup();
  env.setDevice(async () => {
    throw buildApiError({ error: "rate_limited", error_code: 11206 }, 429, {
      get: (name) => (name === "Retry-After" ? "12" : null),
    });
  });
  await assert.rejects(
    env.resolver.previewLoginRequest("ABCDEFGH", "device", "agent-key"),
  );
  assert.equal(env.resolver.previewCooldownMs(), 12000);
  await assert.rejects(
    env.resolver.previewLoginRequest("ABCDEFGH", "device", "agent-key"),
    { kind: "rate_limited" },
  );
  assert.deepEqual(env.calls, ["device"]);
  env.advance(12000);
  env.setDevice(async () => ({ ...preview(), requested_profile: "work" }));
  const result = await env.resolver.previewLoginRequest(
    "ABCDEFGH",
    "device",
    "agent-key",
  );
  assert.equal(
    "requested_profile" in result && result.requested_profile,
    "work",
  );
});

for (const status of [0, 503]) {
  test(`Agent Key mint ${status} failure keeps mint retry without uncertainty controls, code input or deferred-link hold`, async () => {
    const env = setup();
    env.setDecision(async () => {
      throw buildApiError(null, status);
    });
    env.mount("agent-key", { mint: true });
    await env.press("Full account session");
    await env.press("Generate account login code");
    const expected =
      status === 0
        ? "Could not reach the login server. Check your connection and try again."
        : "NyxID couldn't complete this request. Try again.";
    assert.ok(env.nodes().some((node) => node.props.children === expected));
    assert.equal(env.navigationDisabled(), false);
    assert.ok(
      !env
        .nodes()
        .some(
          (node) =>
            node.props.label === "Done" ||
            node.props.label === "Scan or enter another code" ||
            node.type === "TextInput",
        ),
    );
    assert.deepEqual(env.calls, ["mint"]);
    env.setDecision(async () => {});
    env.advance(750);
    await env.press("Generate account login code");
    assert.ok(env.nodes().some((node) => node.type === "LoginCodeStatusView"));
    assert.deepEqual(env.calls, ["mint", "mint"]);
    env.harness.unmount();

    const linked = setup();
    const mint = deferred<void>();
    linked.setDecision(() => mint.promise);
    linked.mount("agent-key", { mint: true });
    await linked.press("Full account session");
    await linked.press("Generate account login code");
    linked.params().user_code = "EFGHABCD";
    linked.harness.render();
    mint.reject(buildApiError(null, status));
    await linked.harness.settle();
    assert.ok(
      linked
        .nodes()
        .some((node) => node.props.label === "Full account session"),
      "the deferred route applies immediately after mint failure",
    );
    assert.ok(
      !linked
        .nodes()
        .some(
          (node) =>
            node.props.label === "Done" ||
            node.props.label === "Scan or enter another code" ||
            node.type === "TextInput",
        ),
    );
    assert.deepEqual(
      linked.calls,
      ["mint"],
      "applying the link never previews or decides",
    );
    linked.harness.unmount();
  });

  test(`Agent Key restricted mint ${status} failure keeps credential review and retry without request uncertainty`, async () => {
    const env = setup();
    env.mount("agent-key", { mint: true });
    await env.press("Choose an Agent Key");
    const picker = env
      .nodes()
      .find((node) => node.type === "WorkspaceSelectionField")!;
    (picker.props.onChange as (ids: string[]) => void)([
      (picker.props.options as { value: string }[])[0]!.value,
    ]);
    env.harness.flush();
    env.setDecision(async () => {
      throw buildApiError(null, status);
    });
    env.advance(750);
    await env.press("Generate restricted login code");
    const expected =
      status === 0
        ? "Could not reach the login server. Check your connection and try again."
        : "NyxID couldn't complete this request. Try again.";
    assert.ok(env.nodes().some((node) => node.props.children === expected));
    assert.ok(
      env
        .nodes()
        .some((node) => node.props.label === "Generate restricted login code"),
    );
    assert.ok(
      !env
        .nodes()
        .some(
          (node) =>
            node.props.label === "Done" ||
            node.props.label === "Scan or enter another code" ||
            node.props.accessibilityLabel === "User code",
        ),
    );
    assert.equal(env.navigationDisabled(), false);
    env.setDecision(async () => {});
    env.advance(750);
    await env.press("Generate restricted login code");
    assert.ok(env.nodes().some((node) => node.type === "LoginCodeStatusView"));
    assert.deepEqual(env.calls, ["mint", "mint"]);
    env.harness.unmount();
  });
}

for (const kind of ["device", "agent-key"] as const) {
  for (const uncertain of [false, true]) {
    test(`${kind} ${uncertain ? "uncertain approve retried" : "plain approve"} with an already-delivered error settles ${uncertain ? "approved" : "completed"}`, async () => {
      const env = setup();
      const alreadyUsed = buildApiError(
        {
          error: "already_delivered",
          error_code: kind === "device" ? 11205 : 11905,
        },
        409,
      );
      env.setDecision(async () => {
        throw uncertain ? buildApiError(null, 0) : alreadyUsed;
      });
      await startReviewedDecision(env, kind, "approve");
      if (uncertain) {
        assertUncertainReview(env, kind);
        env.setDecision(async () => {
          throw alreadyUsed;
        });
        env.advance(750);
        await env.press(
          kind === "device" ? "Approve full account session" : "Approve",
        );
      }
      assert.equal(env.harness.value().type, "Terminal");
      assert.equal(
        env.harness.value().props.state,
        uncertain ? "approved" : "completed",
      );
      assert.equal(env.navigationDisabled(), false);
      assert.deepEqual(
        env.calls,
        uncertain ? [kind, "approve", "approve"] : [kind, "approve"],
      );
      env.harness.unmount();
    });
  }
}


test("a linked device request waits for the shared cooldown even when both timers fire before a render", async () => {
  const env = setup();
  env.resolver.recordLoginPreviewFailure({ statusCode: 429, retryAfterMs: 5000 });
  env.mount("device", { user_code: "ABCDEFGH" });
  assert.deepEqual(env.calls, []);
  env.advance(4999);
  assert.deepEqual(env.calls, []);
  // advance() invokes all due callbacks before flushing React state, so the
  // preview callback must consult the live resolver deadline, not old UI state.
  env.advance(1);
  await env.harness.settle();
  assert.deepEqual(env.calls, ["device"]);
  assert.ok(env.nodes().some(node => node.props.value === "ABCD-EFGH"));
  await env.app("active");
  await env.harness.settle();
  assert.deepEqual(env.calls, ["device"]);
  assert.deepEqual(env.decisionCodes, []);
  env.harness.unmount();
});

test("a cooldown extended while a linked preview waits is respected and a replacement gets the only read", async () => {
  const env = setup();
  env.resolver.recordLoginPreviewFailure({ statusCode: 429, retryAfterMs: 5000 });
  env.mount("device", { user_code: "ABCDEFGH" });
  env.advance(4000);
  env.resolver.recordLoginPreviewFailure({ statusCode: 429, retryAfterMs: 5000 });
  env.params().user_code = "EFGHABCD";
  env.harness.render();
  await env.advanceAsync(4999);
  assert.deepEqual(env.calls, []);
  await env.advanceAsync(1);
  assert.deepEqual(env.calls, ["device"]);
  assert.ok(env.nodes().some(node => node.props.value === "EFGH-ABCD"));
  assert.deepEqual(env.decisionCodes, []);
  env.harness.unmount();
});


test("removing a delayed linked code or editing it cancels its automatic preview", async () => {
  for (const action of ["remove", "edit", "unmount"] as const) {
    const env = setup();
    env.resolver.recordLoginPreviewFailure({ statusCode: 429, retryAfterMs: 5000 });
    env.mount("device", { user_code: "ABCDEFGH" });
    assert.deepEqual(env.calls, []);
    if (action === "remove") {
      env.params().user_code = undefined;
      env.harness.render();
    } else if (action === "edit") {
      env.field("EFGHABCD");
    } else env.harness.unmount();
    await env.advanceAsync(6000);
    assert.deepEqual(env.calls, []);
    assert.deepEqual(env.decisionCodes, []);
    if (action !== "unmount") env.harness.unmount();
  }
});

for (const [button, action] of [["Approve full account session", "approve"], ["Reject", "deny"]]) {
  test(`a staging device scan keeps the reviewed request bound through ${action}`, async () => {
    const env = setup();
    env.mount("device", { start_scanner: true });
    const scanner = env.nodes().find((node) => node.type === "DeviceCodeScanner")!;
    (scanner.props.onCode as (code: string, kind: string) => void)("ABCDEFGH", "staging-device");
    await env.harness.settle();
    assert.deepEqual(env.calls, ["staging-preview"]);
    env.advance(750);
    await env.press(button!);
    assert.deepEqual(env.calls, ["staging-preview", "staging-" + action]);
    assert.deepEqual(env.decisionCodes, ["ABCDEFGH"]);
  });
}
