import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { notificationDestination } from "./notificationDestination";
import { notificationPushSignalFromData } from "./notificationPushSignal";
const service = readFileSync(
  new URL(
    "../../../../backend/src/services/notification_service.rs",
    import.meta.url,
  ),
  "utf8",
);
const assistant = readFileSync(
  new URL(
    "../../../../backend/src/services/assistant_notification_service.rs",
    import.meta.url,
  ),
  "utf8",
);
const config = { scheme: "nyxid", trustedUrls: ["https://web.example"] };
// Fixtures mirror the emitted HashMap entries. Source assertions deliberately
// fail when a backend key changes, so the parser contract cannot silently drift.
test("backend approval push keys retain challenge navigation and silent decision refresh identity", () => {
  for (const key of ["type", "request_id", "challenge_id", "deeplink", "url"])
    assert.ok(
      service.includes(`"${key}"`),
      `notification_service.rs no longer emits ${key}; update the native contract`,
    );
  assert.match(service, /format!\("nyxid:\/\/challenge\/\{\}", request.id\)/);
  for (const type of ["approval_request", "approval_decision"]) {
    assert.ok(service.includes(`"${type}".to_string()`));
    const payload =
      type === "approval_request"
        ? {
            type,
            request_id: "request",
            challenge_id: "request",
            deeplink: "nyxid://challenge/request",
          }
        : { type, request_id: "request", decision: "approved" };
    assert.equal(
      notificationDestination(payload, config),
      "nyxid://activity?challengeId=request",
    );
  }
  assert.equal(
    notificationDestination(
      {
        type: "machine_control",
        url: "https://web.example/assistant/machines?machine=node",
      },
      config,
    ),
    "nyxid://assistant/machines?machine=node",
  );
});
test("NyxBot backend notification id and canonical URL remain readable through APNs and FCM wrappers", () => {
  for (const key of [
    "type",
    "kind",
    "notification_id",
    "deeplink",
    "conversation_id",
  ])
    assert.ok(
      assistant.includes(`"${key}".to_owned()`),
      `assistant notification payload lost ${key}`,
    );
  const payload = {
    type: "chat_update",
    kind: "chat_update",
    notification_id: "notice",
    deeplink: "https://web.example/assistant?c=nyxa-thread",
    conversation_id: "nyxa-thread",
  };
  for (const wrapped of [
    payload,
    { data: payload },
    { data: JSON.stringify(payload) },
  ]) {
    assert.equal(
      notificationDestination(wrapped, config),
      "nyxid://chat?conversationId=nyxa-thread&draft=false&openLatest=false",
    );
  }
  assert.equal(
    notificationPushSignalFromData(payload)?.notificationId,
    "notice",
  );
});
