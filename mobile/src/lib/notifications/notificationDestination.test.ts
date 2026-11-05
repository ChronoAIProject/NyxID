import assert from "node:assert/strict";
import test from "node:test";
import { getStateFromPath } from "@react-navigation/core";
import {
  notificationDestination,
  nativeNotificationUrl,
} from "./notificationDestination";
import { navigationLinkingConfig } from "../../app/navigationLinking";
import { workspaceStateFromPath } from "../../app/workspaceCanonicalLinks";
const config = {
  scheme: "nyxid",
  trustedUrls: ["https://nyx.example/", "https://api.nyx.example"],
};
const c = `nyxa-${"a".repeat(32)}`;
const g = `nyxg-${"b".repeat(32)}`;
function routes(data: unknown) {
  const href = notificationDestination(data, config);
  assert.ok(href);
  const url = new URL(href);
  const state = workspaceStateFromPath(
    `${url.host}${url.pathname}${url.search}`,
    (path) => getStateFromPath(path, navigationLinkingConfig),
  );
  assert.ok(state, `Native route missing for ${href}`);
  return state.routes.flatMap((r) => r.state?.routes ?? [r]);
}
// Payloads built by notification_service.rs (including the callers in trigger_service,
// mfa_service, channel_bot_health, machine_update_service and handlers/devices),
// plus approval_service.rs:1357. push_service transports these maps unchanged.
const backendPayloads: [
  string,
  Record<string, string>,
  string,
  Record<string, unknown> | undefined,
][] = [
  [
    "automation result",
    { type: "automation_result", conversation_id: c },
    "Assistant",
    { conversationId: c, draft: false, openLatest: false },
  ],
  [
    "group mention",
    {
      type: "group_mention",
      group_id: g,
      deeplink: `https://nyx.example/assistant?c=${g}`,
    },
    "GroupChat",
    { groupId: g },
  ],
  [
    "approval",
    {
      type: "approval_request",
      request_id: "request",
      challenge_id: "request",
      deeplink: "nyxid://challenge/request",
      http_method: "POST",
      resource: "/resource",
      verb: "write",
    },
    "Activity",
    { challengeId: "request" },
  ],
  [
    "execution proposal",
    {
      type: "approval_request",
      request_id: "request",
      challenge_id: "request",
      deeplink: "nyxid://challenge/request",
      decision_kind: "execution_permission",
      can_decide: "false",
      review_path: "/assistant/approvals",
    },
    "Activity",
    { challengeId: "request" },
  ],
  [
    "organization approval",
    {
      type: "approval_request",
      request_id: "request",
      challenge_id: "request",
      deeplink: "nyxid://challenge/request",
      from_org_policy: "true",
      org_id: "org",
      org_name: "Team",
    },
    "Activity",
    { challengeId: "request" },
  ],
  [
    "decision",
    { type: "approval_decision", request_id: "request", decision: "approved" },
    "Activity",
    { challengeId: "request" },
  ],
  [
    "expiry",
    { type: "approval_expired", request_id: "request" },
    "Activity",
    { challengeId: "request" },
  ],
  ["MFA recovery", { type: "mfa_recovery_used" }, "AccountSecurity", undefined],
  ["MFA disabled", { type: "mfa_disabled" }, "AccountSecurity", undefined],
  [
    "machine lifecycle",
    {
      type: "machine_lifecycle",
      machine_id: "machine",
      operation_id: "operation",
      url: "https://nyx.example/machines?machine=machine",
    },
    "Machines",
    { machine: "machine" },
  ],
  [
    "machine control",
    {
      type: "machine_control",
      url: `https://nyx.example/assistant/machines/node/desktop?conversation_id=${c}&context_id=context&display=display`,
    },
    "MachineDesktop",
    {
      nodeId: "node",
      conversation_id: c,
      context_id: "context",
      display: "display",
    },
  ],
  [
    "trigger",
    {
      type: "trigger_event",
      trigger_label: "Webhook",
      event_id: "event",
      preview: "Preview",
    },
    "Automations",
    undefined,
  ],
  [
    "device binding",
    {
      type: "device_bind_success",
      template: "device_bind_success",
      hw_id: "hardware",
      device_label: "Laptop",
      node_id: "credential-node",
    },
    "Activity",
    undefined,
  ],
  [
    "device failures",
    {
      type: "device_repeated_fail",
      template: "device_repeated_fail",
      hw_id: "hardware",
      device_label: "Laptop",
      node_id: "credential-node",
      failed_poll_count: "5",
    },
    "Activity",
    undefined,
  ],
  [
    "device lock",
    {
      type: "device_lock_alert",
      template: "device_lock_alert",
      hw_id: "hardware",
      device_label: "Laptop",
      locked_until: "2026-11-06T10:00:00Z",
      failed_poll_count: "10",
    },
    "Activity",
    undefined,
  ],
  [
    "expired connection",
    {
      type: "connection_expired",
      service_slug: "service",
      user_service_id: "binding",
      api_key_id: "key",
      reconnect_url: "https://nyx.example/keys",
    },
    "Plugins",
    { tab: "connectors" },
  ],
  [
    "failed channel bot",
    {
      type: "channel_bot_failed",
      bot_id: "bot",
      recommended_action: "reconnect",
      url: "https://nyx.example/channel-bots/bot",
    },
    "Plugins",
    { tab: "connectors" },
  ],
];
for (const [label, payload, name, params] of backendPayloads) {
  test(`backend push map: ${label} reaches its native destination`, () => {
    const row = routes(payload).at(-1)!;
    assert.equal(row.name, name);
    assert.deepEqual(row.params, params);
  });
}
test("group conversation IDs and canonical browser links retain exact threads", () => {
  for (const data of [
    { conversation_id: g },
    { deeplink: `https://nyx.example/assistant?c=${g}` },
  ]) {
    assert.equal(routes(data).at(-1)!.name, "GroupChat");
    assert.deepEqual(routes(data).at(-1)!.params, { groupId: g });
  }
});
test("approval payloads and browser review links retain the exact review target", () => {
  for (const type of [
    "approval_request",
    "approval_decision",
    "approval_expired",
  ]) {
    const row = routes({ type, challenge_id: "challenge" }).at(-1)!;
    assert.equal(row.name, "Activity");
    assert.deepEqual(row.params, { challengeId: "challenge" });
  }
  assert.deepEqual(
    routes({ type: "approval_decision", request_id: "id" }).at(-1)!.params,
    { challengeId: "id" },
  );
  assert.equal(
    routes({ review_path: "/assistant/approvals" }).at(-1)!.name,
    "Activity",
  );
  const card = routes({
    deeplink: `https://nyx.example/assistant?c=${c}&agent=expert&card=card-1`,
  }).at(-1)!;
  assert.equal(card.name, "Assistant");
  assert.equal((card.params as any).acknowledgementId, "card-1");
});
test("FCM/APNs nested and string-encoded data use the same mapper", () => {
  for (const wrapper of [
    (v: unknown) => ({ data: v }),
    (v: unknown) => ({ payload: { custom: v } }),
    (v: unknown) => ({ remoteMessage: { data: JSON.stringify(v) } }),
  ]) {
    assert.deepEqual(routes(wrapper({ group_id: g })).at(-1)!.params, {
      groupId: g,
    });
  }
  const cycle: any = {};
  cycle.self = cycle;
  assert.equal(notificationDestination(cycle, config), null);
  assert.equal(notificationDestination({ data: "{" }, config), null);
  assert.equal(
    notificationDestination({ data: "[".repeat(66000) }, config),
    null,
  );
});
test("trusted browser links stay native and external, credential-bearing or malformed links are refused", () => {
  assert.equal(
    nativeNotificationUrl("https://nyx.example/challenge/a%26b", config),
    "nyxid://activity?challengeId=a%26b",
  );
  assert.equal(
    routes({ url: "https://nyx.example/machines?machine=node" }).at(-1)!.name,
    "Machines",
  );
  assert.equal(routes({ type: "connection_expired" }).at(-1)!.name, "Plugins");
  assert.equal(
    routes({ type: "mfa_recovery_used" }).at(-1)!.name,
    "AccountSecurity",
  );
  assert.equal(routes({ type: "trigger_event" }).at(-1)!.name, "Automations");
  for (const url of [
    "https://evil.example/assistant",
    "https://nyx.example.evil/assistant",
    "https://user:pass@nyx.example/assistant",
    "javascript:alert(1)",
    "bad",
    "nyxid://challenge/%",
  ]) {
    assert.equal(nativeNotificationUrl(url, config), null, url);
  }
});

test("reconnect_url maps /keys independently of the type fallback", () => {
  assert.deepEqual(
    routes({ reconnect_url: "https://nyx.example/keys" }).at(-1)!.params,
    { tab: "connectors" },
  );
});
