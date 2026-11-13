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
function setup(legacy = false) {
  const writes: unknown[] = [];
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
