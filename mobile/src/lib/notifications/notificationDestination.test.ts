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
test("backend assistant, voice, automation and group payloads resolve to exact native threads", () => {
  for (const type of [
    "assistant_reply",
    "assistant_voice_completed",
    "automation_result",
  ]) {
    const result = routes({
      type,
      conversation_id: c,
      request_id: "voice-work-is-not-an-approval",
      agent_id: "agent",
      acknowledgement_id: "card",
    }).at(-1)!;
    assert.equal(result.name, "Assistant");
    assert.equal((result.params as any).conversationId, c);
    assert.equal((result.params as any).agentId, "agent");
    assert.equal((result.params as any).acknowledgementId, "card");
    assert.equal((result.params as any).draft, false);
  }
  for (const data of [
    { type: "group_mention", group_id: g },
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
