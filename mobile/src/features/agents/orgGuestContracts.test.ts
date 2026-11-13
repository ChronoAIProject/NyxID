import assert from "node:assert/strict";
import test from "node:test";
import {
  orgGuestSettingsSchema,
  orgGuestReviewSchema,
  orgGuestSwitchPayload,
} from "./orgGuestContracts";
test("old settings stay legacy and cannot enable a new Grants route", () => {
  const value = orgGuestSettingsSchema.parse({
    channel_id: "bot",
    org_id: "org",
    can_manage: true,
    guest_enabled: false,
    spent_today_credits: "0.000000000001",
    reserved_today_credits: "0",
  });
  assert.equal(value.guest_policy_mode, "legacy");
  assert.equal(value.grants_ready, false);
  assert.deepEqual(value.effective_guest_services, []);
  assert.equal(value.spent_today_credits, "0.000000000001");
});
test("mobile switch sends only enabled and revision, preserving saved legacy fields", () => {
  assert.deepEqual(orgGuestSwitchPayload(true, "rev"), {
    enabled: true,
    expected_revision: "rev",
  });
});
test("review preserves bot names, service names and guest operation narrowing", () => {
  const value = orgGuestReviewSchema.parse({
    ready: true,
    pending: true,
    review: "bound",
    bots: [{ id: "bot", name: "Office bot", status: "failed" }],
    services: [
      { id: "s", label: "Mail", access: "none", limited_operations: true },
    ],
  });
  assert.equal(value.bots[0]!.name, "Office bot");
  assert.equal(value.services[0]!.access, "none");
  assert.equal(value.services[0]!.limited_operations, true);
});
