import { translate } from "../../i18n/translate";
import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { agentSchema } from "./agentContracts";
import { GUEST_ACCESS_LABEL } from "./orgGuestContracts";
type Element = { type: any; props: Record<string, any> };
const elements = (r: any): Element[] =>
  r && typeof r === "object" && r.props
    ? [r, ...[r.props.children].flat(Infinity).flatMap(elements)]
    : [];
const agent = agentSchema.parse({
  id: "agent",
  kind: "specialist",
  name: "Office",
  description: "Work",
  status: "idle",
  owner_kind: "org",
  org_role: "admin",
  can_maintain: true,
});
function setup(legacy = false, language: "en" | "zh-CN" | "zh-TW" = "en") {
  const writes: unknown[] = [];
  const translations: string[] = [];
  let alertButtons: any[] = [];
  const review = {
    ready: true,
    pending: true,
    review: "bound",
    bots: [{ id: "bot", name: "Office helper", status: "active" }],
    services: [
      {
        id: "calendar-id",
        label: "Calendar",
        access: "read",
        limited_operations: true,
      },
    ],
  };
  const query: any = {
    isError: false,
    dataUpdatedAt: 100,
    data: {
      channel_id: "bot",
      org_id: "org",
      can_manage: true,
      guest_enabled: false,
      guest_revision: "revision",
      grants_ready: true,
      guest_policy_mode: legacy ? "legacy" : "grants",
      effective_guest_services: review.services,
      spent_today_credits: "8",
      reserved_today_credits: "0",
    },
  };
  const session: any = {
    key: ["test"],
    active: true,
    readable: true,
    activationAt: 50,
    busy: false,
    valid: () => true,
    guard: () => {},
    run: async (action: () => Promise<unknown>) => action(),
  };
  const harness = hookHarness<Element>({
    "../../i18n/LanguageProvider": { useAppLanguage: () => ({ t: (phrase: string) => { translations.push(phrase); return translate(language, phrase); } }) },
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      Alert: {
        alert: (_title: string, _text: string, buttons: any[]) => {
          alertButtons = buttons;
        },
      },
      Linking: { openURL: async () => {} },
      Switch: "Switch",
    },
    "@tanstack/react-query": { useQuery: () => query },
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsRow: "SettingsRow",
    },
    "../../components/workspace/WorkspaceUI": { Notice: "Notice" },
    "../../lib/domains": {
      activeDomains: () => ({ frontendUrl: "https://web.example" }),
    },
    "../../lib/api/orgGuestApi": {
      orgGuestApi: {
        preview: async () => review,
        toggle: async (...args: any[]) => {
          args[3]();
          writes.push(args.slice(0, 3));
        },
        adopt: async (...args: any[]) => {
          args[2]();
          writes.push(args.slice(0, 2));
        },
      },
    },
    "../groups/useCollaborationSession": {
      useCollaborationSession: () => session,
    },
    "./orgGuestContracts": { GUEST_ACCESS_LABEL },
  });
  const module = harness.load(
    fileURLToPath(new URL("./OrgGuestControls.tsx", import.meta.url)),
  ) as any;
  return {
    harness,
    module,
    writes,
    translations,
    query,
    session,
    review,
    buttons: () => alertButtons,
  };
}
test("native org bot offers named Grants and switch-only saving, with no budget/checklist", async () => {
  const e = setup();
  e.harness.mount(() =>
    e.module.OrgGuestChannel({
      agent,
      channel: {
        id: "bot",
        bot_label: "Office helper",
        platform: "lark",
        status: "active",
      },
    }),
  );
  elements(e.harness.flush())
    .find((r) => r.props.label === "Office helper")!
    .props.onPress();
  const rows = elements(e.harness.flush());
  assert.ok(
    rows.some(
      (r) =>
        r.props.label === "Guests can use" && r.props.detail === "Calendar",
    ),
  );
  assert.ok(
    !rows.some((r) => /budget|Delegated services/.test(r.props.label ?? "")),
  );
  const toggle = rows.find((r) => r.props.label === "Allow guests")!.props
    .accessory;
  assert.equal(toggle.props.disabled, false);
  await toggle.props.onValueChange(true);
  assert.deepEqual(e.writes, [["bot", true, "revision"]]);
});
test("native legacy page keeps the preservation notice and admin-only switch", () => {
  const e = setup(true);
  e.query.data.can_manage = false;
  e.harness.mount(() =>
    e.module.OrgGuestChannel({
      agent,
      channel: {
        id: "bot",
        bot_label: "Office helper",
        platform: "lark",
        status: "active",
      },
    }),
  );
  elements(e.harness.flush())
    .find((r) => r.props.label === "Office helper")!
    .props.onPress();
  const rows = elements(e.harness.flush());
  assert.ok(
    rows.some(
      (r) =>
        r.props.label ===
        "Existing guest limits are kept. Review them in Grants.",
    ),
  );
  assert.equal(
    rows.find((r) => r.props.label === "Allow guests")!.props.accessory.props
      .disabled,
    true,
  );
});
test("native admin sees model-only access and where to review the frozen list", () => {
  const e = setup(true);
  Object.assign(e.query.data, {
    guest_enabled: true,
    binding_limits_guest_services: true,
    effective_guest_services: [],
  });
  e.harness.mount(() => e.module.OrgGuestChannel({
    agent,
    channel: { id: "bot", bot_label: "Office helper", platform: "discord", status: "active" },
  }));
  elements(e.harness.flush()).find((r) => r.props.label === "Office helper")!.props.onPress();
  const rows = elements(e.harness.flush());
  assert.ok(rows.some((r) => r.props.detail === "Model-only chat"));
  assert.ok(rows.some((r) => r.props.label?.startsWith("This bot's saved limits exclude services listed for guests")));
  assert.equal(e.writes.length, 0);
});
test("native handoff needs explicit confirmation and rejects a stale account callback", async () => {
  const e = setup();
  e.query.data = e.review;
  e.harness.mount(() =>
    e.module.OrgGuestGrantsAdoption({ agent, disabled: false }),
  );
  await elements(e.harness.flush())
    .find(
      (r) =>
        r.props.label ===
        "Use these guest settings for all linked organization bots",
    )!
    .props.onPress();
  await e.harness.settle();
  assert.deepEqual(e.writes, []);
  e.session.valid = () => false;
  e.buttons()
    .find((b) => b.text === "Apply reviewed guest settings")
    .onPress();
  assert.deepEqual(e.writes, []);
  e.session.valid = () => true;
  e.buttons()
    .find((b) => b.text === "Apply reviewed guest settings")
    .onPress();
  await Promise.resolve();
  assert.deepEqual(e.writes, [["agent", "bound"]]);
});

test("native member settings tolerate omitted money, explain read-only access and never write", async () => {
  const e = setup();
  e.query.data.can_manage = false;
  delete e.query.data.spent_today_credits;
  delete e.query.data.reserved_today_credits;
  e.harness.mount(() => e.module.OrgGuestChannel({ agent, channel: { id:"bot",bot_label:"Office helper",platform:"lark",status:"active" } }));
  elements(e.harness.flush()).find((r) => r.props.label === "Office helper")!.props.onPress();
  const rows = elements(e.harness.flush());
  assert.ok(rows.some((r) => r.props.message === "An organization admin can change these settings."));
  assert.ok(!rows.some((r) => r.props.label === "Organization spend today"));
  const control = rows.find((r) => r.props.label === "Allow guests")!.props.accessory;
  assert.equal(control.props.disabled,true);
  control.props.onValueChange(true);
  await e.harness.settle();
  assert.deepEqual(e.writes,[]);
});

test("org channel row renders localized draining status without an activation retry", () => {
  for (const language of ["en", "zh-CN", "zh-TW"] as const) {
    const e = setup(false, language);
    e.harness.mount(() => e.module.OrgGuestChannel({ agent, channel: {
      id: "discord", bot_label: "Office helper", platform: "discord", status: "active",
      gateway_native_activation: "gateway_native_draining",
    } }));
    const rows = elements(e.harness.flush());
    const row = rows.find((r) => r.props.label === "Office helper");
    assert.ok(row);
    const phrase = "Discord intake is paused while the retained gateway release and safety drain are checked. NyxID keeps checking; retrying activation cannot bypass this safety check.";
    assert.equal(row.props.detail, translate(language, phrase));
    assert.ok(!rows.some((r) => /Retry native/.test(r.props.label ?? "")));
  }
});


test("organization channel slugs bypass translation", () => {
  for (const platform of ["x", "lark", "discord"]) {
    const e = setup(false, "zh-CN");
    e.harness.mount(() => e.module.OrgGuestChannel({ agent, channel: {
      id: platform, bot_label: "Plain slug", platform, status: "active",
    } }));
    const rows = elements(e.harness.flush());
    assert.equal(rows.find((r) => r.props.label === "Plain slug")?.props.detail, platform);
    assert.ok(!e.translations.includes(platform));
  }
});

test("an admin outside the maintainer list can read guest settings but cannot toggle them", () => {
  const e = setup();
  e.harness.mount(() => e.module.OrgGuestChannel({
    agent: { ...agent, can_use: true, can_maintain: false },
    channel: { id: "bot", bot_label: "Finance", platform: "lark", status: "active" },
  }));
  elements(e.harness.flush()).find((r) => r.props.label === "Finance")!.props.onPress();
  const toggle = elements(e.harness.flush()).find((r) => r.props.label === "Allow guests")!.props.accessory;
  assert.equal(toggle.props.disabled, true);
  toggle.props.onValueChange(true);
  assert.deepEqual(e.writes, []);
});
