import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as model from "./agentKeyLoginModel";
import * as schema from "../../lib/api/agentKeyLoginSchema";
import type {
  AgentKeyOptions,
  AgentKeySummary,
  NewAgentKey,
} from "../../lib/api/agentKeyLoginSchema";

type Element = { type: unknown; props: Record<string, any> };
function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child as Element)
          : [],
      ),
  ];
}
const options: AgentKeyOptions = {
  personal_owner_id: "person",
  orgs: [{ id: "org", owner_id: "org", name: "Organization" }],
  services: [
    { id: "manual", owner_id: "person", name: "Personal service" },
    {
      id: "platform",
      owner_id: "person",
      name: "Personal platform",
      auto_connected: true,
    },
    {
      id: "org-platform",
      owner_id: "org",
      name: "Org platform",
      auto_connected: true,
    },
  ],
  nodes: [{ id: "node", owner_id: "person", name: "Personal node" }],
  keys: [],
};
function commonMocks() {
  return {
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": Object.fromEntries(
      [
        "View",
        "Text",
        "TextInput",
        "Switch",
        "Pressable",
        "ScrollView",
        "KeyboardAvoidingView",
      ].map((name) => [name, name]),
    ),
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "./deviceLoginStyles": { createDeviceLoginStyles: () => ({}) },
    "../../components/PrimaryButton": { PrimaryButton: "PrimaryButton" },
    "../../components/workspace/WorkspaceSelectionField": {
      WorkspaceSelectionField: "WorkspaceSelectionField",
    },
    "../../lib/api/agentKeyLoginSchema": schema,
    "./agentKeyLoginModel": model,
  };
}
function setupForm(
  initialValues = model.defaultNewAgentKey(),
  disabled = false,
) {
  const reviewed: NewAgentKey[] = [];
  const harness = hookHarness<Element>(commonMocks());
  const { AgentKeyCreateForm } = harness.load(
    fileURLToPath(new URL("./AgentKeyCreateForm.tsx", import.meta.url)),
  ) as {
    AgentKeyCreateForm: (props: unknown) => Element;
  };
  harness.mount(() =>
    AgentKeyCreateForm({
      options,
      initialValues,
      disabled,
      onReview: (input: NewAgentKey) => reviewed.push(input),
    }),
  );
  return {
    harness,
    reviewed,
    field: (label: string) => {
      const found = elements(harness.value()).find(
        (item) =>
          item.type === "WorkspaceSelectionField" && item.props.label === label,
      );
      assert.ok(found, `Missing ${label} selection`);
      return found.props;
    },
    toggle: (label: string) => {
      const found = elements(harness.value()).find(
        (item) =>
          item.type === "Switch" && item.props.accessibilityLabel === label,
      );
      assert.ok(found, `Missing ${label} toggle`);
      return found.props;
    },
    review: () => {
      const button = elements(harness.value()).find(
        (item) =>
          item.type === "PrimaryButton" &&
          item.props.label === "Review permissions",
      );
      assert.ok(button);
      button.props.onPress();
      harness.flush();
    },
  };
}

test("key form uses compact selection fields and keeps edits local until explicit review", () => {
  const form = setupForm();
  assert.deepEqual([...form.field("Scopes").value], ["read", "proxy"]);
  assert.equal(form.field("Service access").multiple, true);
  form.field("Scopes").onChange(["read", "services:read", "proxy"]);
  form.field("Platform").onChange(["codex"]);
  form.harness.flush();
  assert.equal(form.reviewed.length, 0);
  form.review();
  assert.equal(form.reviewed.length, 1);
  assert.equal(form.reviewed[0].scopes, "read services:read proxy");
  assert.equal(form.reviewed[0].platform, "codex");
  assert.equal(form.reviewed[0].allow_all_services, false);
  assert.equal(form.reviewed[0].allow_all_nodes, false);
});

test("platform access locks only same-owner implied services without replacing exact grants", () => {
  const form = setupForm({
    ...model.defaultNewAgentKey(),
    allow_auto_connected_services: true,
    allowed_service_ids: ["unavailable", "platform", "org-platform"],
  });
  const field = form.field("Service access");
  const platform = field.options.find((item: any) => item.value === "platform");
  const org = field.options.find((item: any) => item.value === "org-platform");
  assert.equal(platform.disabled, true);
  assert.equal(org.disabled, false);
  field.onChange(["unavailable", "platform", "org-platform", "manual"]);
  form.harness.flush();
  form.review();
  assert.deepEqual(form.reviewed[0].allowed_service_ids, [
    "unavailable",
    "platform",
    "org-platform",
    "manual",
  ]);
  assert.equal(form.reviewed[0].allow_auto_connected_services, true);

  const impliedOnly = setupForm({
    ...model.defaultNewAgentKey(),
    allow_auto_connected_services: true,
  });
  assert.deepEqual(
    [...impliedOnly.field("Service access").value],
    ["platform"],
  );
  impliedOnly.field("Service access").onChange(["platform", "manual"]);
  impliedOnly.harness.flush();
  impliedOnly.review();
  assert.deepEqual(impliedOnly.reviewed[0].allowed_service_ids, ["manual"]);
});

test("changing key owner clears previous-owner grants while keeping a same-owner selection intact", () => {
  const form = setupForm({
    ...model.defaultNewAgentKey(),
    allowed_service_ids: ["manual"],
    allowed_node_ids: ["node"],
    allow_auto_connected_services: true,
  });
  form.field("Owner").onChange(["personal"]);
  form.harness.flush();
  assert.ok(form.field("Service access").value.includes("manual"));
  form.field("Owner").onChange(["org"]);
  form.harness.flush();
  assert.deepEqual([...form.field("Service access").value], []);
  assert.deepEqual(
    form.field("Service access").options.map((item: any) => item.value),
    ["org-platform"],
  );
  assert.equal(form.field("Service access").scopeKey, "org");
  assert.deepEqual([...form.field("Node access").options], []);
  form.review();
  assert.equal(form.reviewed[0].target_org_id, "org");
  assert.deepEqual(form.reviewed[0].allowed_node_ids, []);
  assert.equal(form.reviewed[0].allow_auto_connected_services, false);
});

test("allow-all toggles remain explicit, clear restricted grants, and do not enable the other resource", () => {
  const form = setupForm({
    ...model.defaultNewAgentKey(),
    allowed_service_ids: ["manual"],
    allowed_node_ids: ["node"],
  });
  form.toggle("Allow all services").onValueChange(true);
  form.harness.flush();
  assert.equal(
    elements(form.harness.value()).some(
      (item) =>
        item.type === "WorkspaceSelectionField" &&
        item.props.label === "Service access",
    ),
    false,
  );
  assert.deepEqual([...form.field("Node access").value], ["node"]);
  form.review();
  assert.equal(form.reviewed[0].allow_all_services, true);
  assert.equal(form.reviewed[0].allow_all_nodes, false);
  assert.deepEqual(form.reviewed[0].allowed_service_ids, []);
});

test("applying the existing custom expiry preserves its exact timestamp", () => {
  const input = {
    ...model.defaultNewAgentKey(),
    expires_at: "2099-01-02T03:04:05Z",
  };
  const form = setupForm(input);
  assert.deepEqual([...form.field("Key expiry").value], ["custom"]);
  form.field("Key expiry").onChange(["custom"]);
  form.harness.flush();
  form.review();
  assert.equal(form.reviewed[0].expires_at, input.expires_at);
});

test("expiry selectors retain validation and disable all permission pickers during pending work", () => {
  const form = setupForm();
  form.field("Key expiry").onChange(["90"]);
  form.harness.flush();
  form.field("Key expiry").onChange(["custom"]);
  form.harness.flush();
  form.review();
  assert.equal(form.reviewed.length, 0);
  assert.ok(
    elements(form.harness.value()).some(
      (item) => item.props.accessibilityLabel === "Custom key expiry",
    ),
  );
  form.field("Key expiry").onChange(["none"]);
  form.harness.flush();
  form.review();
  assert.equal(form.reviewed[0].expires_at, null);
  const pending = setupForm(model.defaultNewAgentKey(), true);
  for (const item of elements(pending.harness.value()).filter(
    (element) =>
      element.type === "WorkspaceSelectionField" || element.type === "Switch",
  )) {
    assert.equal(item.props.disabled, true);
  }
});

function setupLogin() {
  const apiCalls: string[] = [];
  const key: AgentKeySummary = {
    ...model.newKeySummary(model.defaultNewAgentKey(), options),
    id: "existing-key",
    name: "Known key",
    owner_id: "person",
    key_prefix: "nyx_123",
    created_now: false,
  };
  const harness = hookHarness<Element>({
    ...commonMocks(),
    "react-native": {
      ...commonMocks()["react-native"],
      Platform: { OS: "ios" },
    },
    "lucide-react-native": { ChevronLeft: "ChevronLeft" },
    "../../components/ScreenContainer": { ScreenContainer: "ScreenContainer" },
    "../../lib/api/agentKeyLoginApi": { agentKeyLoginApi: {} },
    "../../lib/api/loginCodeApi": {
      loginCodeApi: {
        options: async () => {
          apiCalls.push("options");
          return { ...options, keys: [key] };
        },
        mint: async () => {
          apiCalls.push("mint");
        },
      },
    },
    "./LoginCodeStatusView": { LoginCodeStatusView: "LoginCodeStatusView" },
    "./AuthSessionContext": {
      useAuthSession: () => ({ isAuthenticated: true }),
    },
    "./DeviceLoginScreen": {
      DetailRow: "DetailRow",
      formatLocation: () => "",
      formatNetwork: () => "",
      formatDevice: () => "",
      requesterValue: () => "",
    },
    "./AgentKeyCreateForm": { AgentKeyCreateForm: "AgentKeyCreateForm" },
    "./DeviceCodeScanner": { DeviceCodeScanner: "DeviceCodeScanner" },
  });
  const { AgentKeyLoginScreen } = harness.load(
    fileURLToPath(new URL("./AgentKeyLoginScreen.tsx", import.meta.url)),
  ) as { AgentKeyLoginScreen: (props: unknown) => Element };
  harness.mount(() =>
    AgentKeyLoginScreen({
      navigation: { canGoBack: () => true },
      route: { params: { mint: true } },
    }),
  );
  return { harness, apiCalls, key };
}

test("login key picker replaces repeated permission blocks and still requires full review before issuance", async () => {
  const { harness, apiCalls, key } = setupLogin();
  const choose = elements(harness.value()).find(
    (item) => item.props.label === "Choose an Agent Key",
  );
  assert.ok(choose);
  choose.props.onPress();
  await harness.settle();
  const picker = elements(harness.value()).find(
    (item) => item.type === "WorkspaceSelectionField",
  );
  assert.ok(picker);
  assert.equal(picker.props.confirmLabel, "Review permissions");
  assert.equal(picker.props.options[0].value, key.id);
  assert.match(picker.props.options[0].description, /nyx_123/);
  assert.equal(
    elements(harness.value()).filter((item) => item.type === "DetailRow")
      .length,
    0,
  );
  picker.props.onChange(["unavailable-key"]);
  harness.flush();
  assert.equal(
    elements(harness.value()).some(
      (item) => item.props.label === "Generate restricted login code",
    ),
    false,
  );
  picker.props.onChange([key.id]);
  harness.flush();
  assert.ok(
    elements(harness.value()).some(
      (item) =>
        item.type === "DetailRow" &&
        item.props.label === "Key" &&
        item.props.value === "Known key",
    ),
  );
  assert.ok(
    elements(harness.value()).some(
      (item) =>
        item.type === "DetailRow" &&
        item.props.label === "Scopes" &&
        item.props.value === "read proxy",
    ),
  );
  assert.ok(
    elements(harness.value()).some(
      (item) => item.props.label === "Generate restricted login code",
    ),
  );
  assert.deepEqual(apiCalls, ["options"]);
});

test("returning to keys allows reviewing the same sole key again without issuance", async () => {
  const { harness, apiCalls, key } = setupLogin();
  const choose = elements(harness.value()).find(
    (item) => item.props.label === "Choose an Agent Key",
  );
  assert.ok(choose);
  choose.props.onPress();
  await harness.settle();
  const firstPicker = elements(harness.value()).find(
    (item) => item.type === "WorkspaceSelectionField",
  );
  assert.ok(firstPicker);
  firstPicker.props.onChange([key.id]);
  harness.flush();
  const back = elements(harness.value()).find(
    (item) => item.props.label === "Back to keys",
  );
  assert.ok(back);
  back.props.onPress();
  harness.flush();
  const nextPicker = elements(harness.value()).find(
    (item) => item.type === "WorkspaceSelectionField",
  );
  assert.ok(nextPicker);
  assert.equal(nextPicker.props.options.length, 1);
  assert.deepEqual([...nextPicker.props.value], []);
  assert.equal(nextPicker.props.summary, "Known key");
  nextPicker.props.onChange([key.id]);
  harness.flush();
  assert.ok(
    elements(harness.value()).some(
      (item) =>
        item.type === "DetailRow" &&
        item.props.label === "Key" &&
        item.props.value === "Known key",
    ),
  );
  assert.ok(
    elements(harness.value()).some(
      (item) => item.props.label === "Generate restricted login code",
    ),
  );
  assert.deepEqual(apiCalls, ["options"]);
});
