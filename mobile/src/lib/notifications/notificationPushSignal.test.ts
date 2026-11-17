import assert from "node:assert/strict";
import test from "node:test";
import {
  notificationPushSignalFromData,
  notificationPushSignalFromResponse,
} from "./notificationPushSignal";

test("notification pushes expose kind, id and deeplink; approval pushes do not", () => {
  assert.deepEqual(
    notificationPushSignalFromData({ type: "needs_decision", notification_id: "n1", deeplink: "https://app/assistant?c=x" }),
    { kind: "needs_decision", notificationId: "n1", deeplink: "https://app/assistant?c=x" },
  );
  assert.deepEqual(notificationPushSignalFromData({ notification_id: "n2" }), { kind: null, notificationId: "n2", deeplink: null });
  assert.equal(notificationPushSignalFromData({ type: "approval_request", challenge_id: "c" }), null);
  assert.equal(notificationPushSignalFromData(null), null);
});

test("a tapped response is read from its content, then the raw push payload", () => {
  const response = (content: unknown, trigger?: unknown) =>
    ({ notification: { request: { content: { data: content }, trigger } } }) as any;
  assert.equal(notificationPushSignalFromResponse(null), null);
  assert.equal(notificationPushSignalFromResponse(response({ type: "chat_update", notification_id: "n" }))!.notificationId, "n");
  assert.equal(
    notificationPushSignalFromResponse(response({}, { type: "push", payload: { notification_id: "p" } }))!.notificationId,
    "p",
  );
  assert.equal(
    notificationPushSignalFromResponse(response({}, { type: "push", remoteMessage: { data: { type: "group_mention" } } }))!.kind,
    "group_mention",
  );
  assert.equal(notificationPushSignalFromResponse(response({}, { type: "local" })), null);
});
