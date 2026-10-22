import assert from "node:assert/strict";
import test from "node:test";
import { guestApprovalLabel, guestApprovalMode } from "./guestApproval";

const guest = {
  platform: "telegram",
  chat_title: "Team",
  sender_name: "Visitor",
  agent_name: "researcher",
};
test("guest approval always offers one request even for a grant-mode row", () => {
  assert.equal(guestApprovalMode(guest, "grant"), "per_request");
  assert.equal(guestApprovalMode(guest, "per_request"), "per_request");
  assert.equal(guestApprovalMode(undefined, "grant"), "grant");
  assert.equal(guestApprovalMode(null, "per_request"), "per_request");
});
test("guest approval labels identify the guest and specialist", () => {
  assert.equal(
    guestApprovalLabel(guest),
    "Guest request: Visitor via telegram in Team · researcher",
  );
});
