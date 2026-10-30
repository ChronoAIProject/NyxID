import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { assistantLinkLabel, assistantLinkTarget } from "./assistantLinkTarget";
import { safeAssistantLinkUrl } from "./assistantMarkdown";
import {
  connectFormValues,
  connectDeviceChallenge,
  connectPollDelay,
} from "./connectLinkState";
import { connectLinkPreviewSchema } from "./connectLinkContracts";
import { ApiError } from "../../lib/api/ApiError";
import { deferred, hookHarness } from "./assistantHookTestHarness";
const origin = "https://app.example.test";
const token = "nyx_clk_synthetic_review";
const id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const preview = () =>
  connectLinkPreviewSchema.parse({
    service_name: "GitHub",
    service_slug: "github",
    label: "Issue review",
    requested_by: "Researcher",
    created_at: "2026-10-28T00:00:00Z",
    expires_at: "2099-10-28T00:00:00Z",
    status: "pending",
    connect_method: "none",
    auth_key_name: "API key",
    credential_mode: null,
    has_platform_oauth_credentials: true,
    requires_gateway_url: false,
    api_key_url: null,
    api_key_instructions: null,
  });
const completed = { id, status: "completed", service_slug: "github" };

test("assistant link classification preserves the exact web handoff and admits native UI only for the configured origin", () => {
  const href = `${origin}/channel-bots/connect/telegram?label=Release%20bot&target_org_id=org&return_to=%2Fassistant%3Fc%3Dchat&extra=kept`;
  const channel = assistantLinkTarget(href, origin)!;
  assert.equal(channel.kind, "channel");
  assert.equal(channel.href, href);
  assert.equal(
    assistantLinkLabel(channel, "Set up Telegram"),
    "Continue on the web · Set up Telegram",
  );
  const connect = assistantLinkTarget(`/connect/${token}`, origin)!;
  assert.equal(connect.kind, "connect");
  assert.equal(assistantLinkLabel(connect, "GitHub"), "Connect GitHub");
  assert.equal(assistantLinkLabel(connect, "Connect GitHub"), "Connect GitHub");
  assert.equal(
    assistantLinkLabel(connect, `${origin}/connect/${token}`),
    "Connect service",
  );
  assert.equal(
    safeAssistantLinkUrl(`/connect/${token}`, origin),
    `${origin}/connect/${token}`,
  );
  assert.equal(
    safeAssistantLinkUrl(
      `http://127.0.0.1:4631/connect/${token}`,
      "http://127.0.0.1:4631",
    ),
    `http://127.0.0.1:4631/connect/${token}`,
  );
  assert.equal(
    safeAssistantLinkUrl("http://untrusted.test/path", origin),
    null,
  );
  for (const bad of [
    `https://other.test/connect/${token}`,
    `https://user:secret@app.example.test/connect/${token}`,
    `${origin}/connect/${token}?redirect=elsewhere`,
    `${origin}/connect/${token}#hash`,
    `${origin}/channel-bots/connect/%2Fbad`,
    `${origin}/assistant?panel=settings&redirect=elsewhere`,
    `${origin}/assistant?panel=settings&panel=billing`,
    "file:///connect/nyx_clk_no",
  ])
    assert.equal(assistantLinkTarget(bad, origin), null, bad);
  assert.deepEqual(
    assistantLinkTarget(
      `${origin}/assistant?panel=billing&panelPeriod=7d&panelService=github`,
      origin,
    ),
    {
      kind: "account",
      panel: "billing",
      href: `${origin}/assistant?panel=billing&panelPeriod=7d&panelService=github`,
      period: "7d",
      tab: undefined,
    },
  );
});

test("hosted connection form validation, challenge merging and polling policy match the web", () => {
  const p = {
    ...preview(),
    connect_method: "api_key" as const,
    requires_gateway_url: true,
  };
  const values = {
    credential: "key",
    endpoint_url: "https://gateway.test",
    oauth_client_id: "",
    oauth_client_secret: "",
  };
  assert.equal(connectFormValues(p, false, values).use_platform_key, false);
  assert.deepEqual(connectFormValues(p, true, { ...values, credential: "" }), {
    use_platform_key: true,
  });
  assert.throws(
    () => connectFormValues(p, false, { ...values, endpoint_url: "" }),
    /Endpoint URL is required/,
  );
  assert.throws(
    () => connectFormValues(p, false, { ...values, oauth_client_id: "client" }),
    /supplied together/,
  );
  assert.throws(() =>
    connectFormValues(p, false, { ...values, credential: "x".repeat(16385) }),
  );
  assert.throws(
    () =>
      connectFormValues(
        { ...p, connect_method: "oauth", credential_mode: "user" },
        false,
        values,
      ),
    /client ID and secret are required/,
  );
  const device = connectDeviceChallenge({
    ...completed,
    status: "device_code_required",
    device_state: "state",
    device_user_code: "ABCD",
    device_verification_uri: "https://provider.test/code",
    device_interval: 5,
  } as any);
  assert.deepEqual(
    connectDeviceChallenge(
      {
        ...completed,
        status: "device_code_required",
        device_interval: 10,
        device_status: "slow_down",
      } as any,
      device,
    ),
    { ...device, interval: 10, status: "slow_down" },
  );
  assert.throws(
    () =>
      connectDeviceChallenge({
        ...completed,
        status: "device_code_required",
      } as any),
    /complete authorization code/,
  );
  const error = (statusCode: number) =>
    new ApiError({
      statusCode,
      errorKey: "bad_request",
      errorCode: 1000,
      message: "No",
    });
  assert.equal(connectPollDelay(error(429), 1, 5), 10);
  assert.equal(connectPollDelay(error(403), 1, 5), null);
  assert.equal(connectPollDelay(new Error("Offline"), 3, 5), null);
  assert.equal(connectPollDelay(new Error("Offline"), 1, 5), 5);
});

function flowHarness() {
  let epoch = 1;
  const availability = { active: true, available: () => availability.active };
  const reads: string[] = [];
  const writes: any[] = [];
  const browsers: string[] = [];
  let value = preview();
  let result: (input: any) => Promise<any> = async () => completed;
  let browser = async (_url: string) => {};
  let terminalStatus = "pending";
  let finished = 0;
  let enabled = true;
  const h = hookHarness<any>({
    "../groups/useGroupRuntime": { useGroupAvailability: () => availability },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../lib/api/servicesApi": {
      servicesApi: {
        catalogEntry: async () => ({ platform_key: { available: true } }),
      },
    },
    "../../lib/api/connectLinksApi": {
      connectLinksApi: {
        preview: async () => {
          reads.push("preview");
          return value;
        },
        complete: async (_token: string, input: any) => {
          writes.push(input);
          return result(input);
        },
        cancel: async () => {
          writes.push("cancel");
          return { ...completed, status: "cancelled" };
        },
        status: async () => {
          reads.push("status");
          return { ...completed, status: terminalStatus };
        },
      },
    },
    "expo-web-browser": {
      openBrowserAsync: async (url: string) => {
        browsers.push(url);
        await browser(url);
      },
    },
  });
  const hook = h.load(
    fileURLToPath(new URL("./useHostedConnect.ts", import.meta.url)),
  ).useHostedConnect as any;
  h.mount(() => hook(token, enabled, () => finished++));
  return {
    h,
    reads,
    writes,
    browsers,
    availability,
    setPreview: (next: typeof value) => {
      value = next;
    },
    setResult: (next: typeof result) => {
      result = next;
    },
    setBrowser: (next: typeof browser) => {
      browser = next;
    },
    setStatus: (next: string) => {
      terminalStatus = next;
    },
    setEnabled: (next: boolean) => {
      enabled = next;
      h.render();
    },
    changeAccount: () => {
      epoch++;
      h.render();
    },
    finished: () => finished,
  };
}
test("real hosted flow fences double approval, unavailable/expired actions, account changes and late completion", async () => {
  const x = flowHarness();
  await x.h.settle();
  assert.equal(x.h.value().usePlatformKey, true);
  const response = deferred<any>();
  x.setResult(async () => response.promise);
  const first = x.h
    .value()
    .approve({ credential: "never send while platform is selected" });
  await x.h.value().approve({});
  assert.deepEqual(JSON.parse(JSON.stringify(x.writes)), [
    { use_platform_key: true },
  ]);
  x.changeAccount();
  response.resolve(completed);
  await first;
  await x.h.settle();
  assert.equal(x.finished(), 0);
  assert.equal(x.h.value().preview.status, "pending");
  x.h.unmount();
  const offline = flowHarness();
  await offline.h.settle();
  offline.availability.active = false;
  offline.h.render();
  await offline.h.value().decline();
  assert.equal(offline.writes.length, 0);
  offline.availability.active = true;
  offline.setPreview({ ...preview(), expires_at: "2000-01-01T00:00:00Z" });
  offline.h.render();
  await offline.h.settle();
  await offline.h.value().approve({});
  assert.equal(offline.writes.length, 0);
  offline.h.unmount();
});
test("OAuth browser dismissal is not success; explicit server recovery settles once and never follows callback URLs", async () => {
  const x = flowHarness();
  x.setPreview({ ...preview(), connect_method: "oauth" });
  await x.h.settle();
  x.setResult(async () => ({
    ...completed,
    status: "oauth_required",
    authorization_url: "https://provider.test/auth",
    callback_url: "https://untrusted.test/redirect",
  }));
  await x.h.value().approve({});
  await x.h.settle();
  assert.deepEqual(x.browsers, ["https://provider.test/auth"]);
  assert.equal(x.h.value().preview.status, "pending");
  assert.equal(x.finished(), 0);
  assert.equal(
    x.writes.length,
    2,
    "web return-page recovery checks the server once, without reopening the provider",
  );
  x.h.unmount();
  const yes = flowHarness();
  await yes.h.settle();
  await yes.h.value().approve({});
  await yes.h.settle();
  assert.equal(yes.h.value().preview.status, "completed");
  assert.equal(yes.finished(), 1);
  await yes.h.value().approve({});
  assert.equal(yes.writes.length, 1);
  yes.h.unmount();
});
test("decline is explicit; closing or disabling a chat never submits or cancels a pending link", async () => {
  const x = flowHarness();
  await x.h.settle();
  x.setEnabled(false);
  await x.h.value().decline();
  x.h.unmount();
  assert.deepEqual(x.writes, []);
  const no = flowHarness();
  await no.h.settle();
  await no.h.value().decline();
  await no.h.settle();
  assert.equal(no.h.value().preview.status, "cancelled");
  assert.deepEqual(no.writes, ["cancel"]);
  no.h.unmount();
});

test("connect API uses the existing authenticated contracts, compacts empty fields and rejects another status identity", async () => {
  const calls: any[] = [];
  let mismatch = false;
  const h = hookHarness({
    "./http": {
      requestJson: async (path: string, options: any) => {
        calls.push({ path, options });
        if (path.endsWith("preview")) return preview();
        if (path.endsWith("complete")) return completed;
        return {
          ...completed,
          id: mismatch ? "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb" : id,
          status: "pending",
          service_name: "GitHub",
          expires_at: "2099-10-28T00:00:00Z",
        };
      },
    },
  });
  const api = h.load(
    fileURLToPath(new URL("../../lib/api/connectLinksApi.ts", import.meta.url)),
  ).connectLinksApi as any;
  await api.preview(token);
  await api.complete(token, {
    use_platform_key: false,
    credential: "key",
    endpoint_url: "",
  });
  assert.deepEqual(JSON.parse(JSON.stringify(calls[1].options.body)), {
    token,
    use_platform_key: false,
    credential: "key",
  });
  await api.cancel(token);
  assert.equal(calls[2].path, "/connect-links/cancel");
  await api.status(id);
  mismatch = true;
  await assert.rejects(api.status(id), /different connection request/);
});

test("device authorization polls only its issued state, preserves code on slow-down, and settles from a server response", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const x = flowHarness();
  await x.h.settle();
  x.setResult(async (input) =>
    input.device_state
      ? {
          ...completed,
          status: "device_code_required",
          device_state: "device",
          device_interval: 2,
          device_status: "slow_down",
        }
      : {
          ...completed,
          status: "device_code_required",
          device_state: "device",
          device_user_code: "ABCD",
          device_verification_uri: "https://provider.test/code",
          device_interval: 1,
        },
  );
  await x.h.value().approve({});
  await x.h.settle();
  t.mock.timers.tick(1000);
  await x.h.settle();
  assert.equal(x.writes.length, 2);
  assert.equal(x.writes[1].device_state, "device");
  assert.equal(x.h.value().device.code, "ABCD");
  assert.equal(x.h.value().device.status, "slow_down");
  assert.equal(x.finished(), 0);
  x.setResult(async () => completed);
  t.mock.timers.tick(2000);
  await x.h.settle();
  assert.equal(x.h.value().preview.status, "completed");
  assert.equal(x.finished(), 1);
  t.mock.timers.tick(30000);
  await x.h.settle();
  assert.equal(
    x.writes.length,
    3,
    "terminal state cancels the next polling timer",
  );
  x.h.unmount();
});

test("real link host routes account panels, preserves channel URL, retains the transcript, and fences a switched chat", async () => {
  type E = { type: any; props: Record<string, any> };
  const nodes = (n: any): E[] =>
    n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
  const actions: any[] = [];
  const browsers: string[] = [];
  const focus: number[] = [];
  const availability = {
    active: true,
    foreground: true,
    focused: true,
    available: () => availability.active,
  };
  let scope = "chat-a";
  let epoch = 1;
  const transcript = {
    type: "Transcript",
    props: { draft: "Keep this draft" },
  };
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": {
      Modal: "Modal",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      View: "View",
      AccessibilityInfo: {
        setAccessibilityFocus: (handle: number) => focus.push(handle),
      },
    },
    "@react-navigation/native": {
      useNavigation: () => ({
        dispatch: (action: any) => actions.push(action),
      }),
    },
    "../../lib/env": { NYXID_FRONTEND_URL: origin },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "Button",
      Card: "Card",
      Label: "Label",
    },
    "../groups/useGroupRuntime": { useGroupAvailability: () => availability },
    "./AssistantHostedConnect": { AssistantHostedConnect: "Connect" },
    "./AssistantLinkContext": {
      AssistantLinkContext: { Provider: "Provider" },
    },
    "lucide-react-native": { X: "X" },
    "expo-web-browser": {
      openBrowserAsync: async (href: string) => browsers.push(href),
    },
  });
  const Host = h.load(
    fileURLToPath(new URL("./AssistantLinkHost.tsx", import.meta.url)),
  ).ScopedAssistantLinkHost as any;
  h.mount(() => Host({ scope, children: transcript }));
  const open = (href: string) => {
    const handled = h.value().props.value(href, 42);
    h.flush();
    return handled;
  };
  assert.equal(open("https://external.test/path"), false);
  open(`${origin}/assistant?panel=billing&panelPeriod=7d`);
  assert.equal(actions.at(-1).payload.params.screen, "UsageCredits");
  assert.equal(actions.at(-1).payload.params.params.period, "7d");
  open(`${origin}/assistant?panel=settings&panelTab=security`);
  assert.equal(actions.at(-1).payload.params.screen, "AccountSecurity");
  open(`${origin}/assistant?panel=settings&panelTab=display`);
  assert.equal(actions.at(-1).payload.params.params.section, "display");
  const channel = `${origin}/channel-bots/connect/telegram?label=Release%20crew&target_org_id=org&x=kept`;
  open(channel);
  nodes(h.value())
    .find((n) => n.props.label === "Continue on the web")!
    .props.onPress();
  assert.deepEqual(browsers, [channel]);
  assert.equal(
    h.value().props.children[0],
    transcript,
    "link flow never replaces the chat or moves its draft",
  );
  open(`${origin}/connect/${token}`);
  assert.equal(
    nodes(h.value()).find((n) => n.type === "Connect")!.props.token,
    token,
  );
  nodes(h.value())
    .find((n) => n.type === "Connect")!
    .props.onBusy(true);
  h.flush();
  nodes(h.value())
    .find((n) => n.type === "Modal")!
    .props.onRequestClose();
  h.flush();
  assert.equal(
    nodes(h.value()).find((n) => n.type === "Modal")!.props.visible,
    true,
    "critical write prevents Close and Android Back",
  );
  scope = "chat-b";
  h.render();
  assert.equal(
    nodes(h.value()).find((n) => n.type === "Connect"),
    undefined,
  );
  assert.equal(h.value().props.children[0], transcript);
  open(`${origin}/connect/${token}`);
  nodes(h.value())
    .find((n) => n.type === "Modal")!
    .props.onRequestClose();
  h.flush();
  nodes(h.value())
    .find((n) => n.type === "Modal")!
    .props.onDismiss();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.deepEqual(focus, [42]);
  epoch++;
  h.render();
  availability.active = false;
  open(`${origin}/assistant?panel=billing`);
  assert.equal(actions.length, 3, "inactive account/thread does not navigate");
  h.unmount();
});

test("real native connection card exposes web credential/device/terminal controls without navigating away", async () => {
  type E = { type: any; props: Record<string, any> };
  const nodes = (n: any): E[] =>
    n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
  const submitted: any[] = [];
  const p = {
    ...preview(),
    connect_method: "api_key" as const,
    requires_gateway_url: true,
  };
  const flow = {
    preview: p,
    loading: false,
    busy: false,
    disabled: false,
    platformAvailable: true,
    usePlatformKey: false,
    device: undefined as any,
    oauth: undefined as any,
    error: undefined as string | undefined,
    setError: (text: string) => {
      flow.error = text;
    },
    approve: async (values: any) => {
      submitted.push(values);
    },
    decline: async () => submitted.push("decline"),
  };
  const h = hookHarness<E>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
      Fragment: "Fragment",
    },
    "../../components/ServiceIcon": { ServiceIcon: "ServiceIcon" },
    "react-native": { View: "View" },
    "../../components/SegmentControl": { SegmentControl: "Segment" },
    "../../components/workspace/WorkspaceUI": Object.fromEntries(
      ["ActionButton", "Card", "Field", "Label", "Notice"].map((name) => [
        name,
        name,
      ]),
    ),
    "./useHostedConnect": { useHostedConnect: () => flow },
  });
  const Card = h.load(
    fileURLToPath(new URL("./AssistantHostedConnect.tsx", import.meta.url)),
  ).AssistantHostedConnect as any;
  h.mount(() => Card({ token, active: true, onBusy: () => {} }));
  const field = (label: string) =>
    nodes(h.value()).find(
      (n) => n.type === "Field" && n.props.label === label,
    )!;
  const button = (label: string) =>
    nodes(h.value()).find((n) => n.props.label === label)!;
  assert.equal(field("API key").props.secureTextEntry, true);
  field("API key").props.onChangeText("review-key");
  field("Service URL").props.onChangeText("https://gateway.test");
  h.flush();
  await button("Approve & connect").props.onPress();
  await h.settle();
  assert.equal(submitted[0].credential, "review-key");
  assert.equal(submitted[0].endpoint_url, "https://gateway.test");
  assert.equal(
    field("API key").props.value,
    "",
    "secret leaves local input state after submission",
  );
  flow.device = {
    code: "ABCD",
    url: "https://provider.test/code",
    interval: 10,
    status: "slow_down",
  };
  flow.error = "Provider stopped checking";
  h.render();
  assert.ok(
    nodes(h.value()).some(
      (n) => n.props.accessibilityLabel === "Authorization code: ABCD",
    ),
  );
  assert.equal(field("API key"), undefined);
  button("Get a new code").props.onPress();
  await h.settle();
  assert.deepEqual(
    Object.keys(submitted.at(-1)),
    [],
    "new device code reuses the existing authorized setup rather than requiring cleared client secrets",
  );
  flow.preview = { ...p, status: "completed" };
  h.render();
  assert.equal(button("Decline"), undefined);
  assert.ok(
    nodes(h.value()).some(
      (n) =>
        n.props.children === "GitHub connected" ||
        (Array.isArray(n.props.children) &&
          n.props.children.join("") === "GitHub connected"),
    ),
  );
  h.unmount();
});
