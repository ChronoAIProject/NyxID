import assert from "node:assert/strict";
import { test } from "node:test";
import { approvalDeliveryWarning } from "./approvalDeliveryPolicy";

test("web-only guarantee requires an explicit server capability and unsuspended status", () => {
  assert.match(approvalDeliveryWarning({ approval_web_available: true, approval_suspended: false }), /Approval remains required/);
  assert.match(approvalDeliveryWarning({ approval_web_available: true, approval_suspended: true }), /server reports approval protection is suspended/);
  assert.match(approvalDeliveryWarning({ approval_suspended: true }), /server reports approval protection is suspended/);
  assert.match(approvalDeliveryWarning({ approval_suspended: false }), /unconfirmed/);
  assert.match(approvalDeliveryWarning(undefined), /unconfirmed/);
});
