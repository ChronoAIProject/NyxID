import assert from "node:assert/strict";
import test from "node:test";
import { LoginCodeMonitor, type LoginCodeMonitorState } from "./loginCodeMonitor";
import type { LoginCodeStatus } from "../../lib/api/loginCodeApi";

const row = (status: LoginCodeStatus["status"]): LoginCodeStatus => ({request_id: "fixture", status,
  auth_kind: "agent_key", expires_at: "2099-01-01T00:00:00Z", redeemed_at: status === "redeemed" ? "2026-01-01T00:00:00Z" : null,
  client_label: "Fixture terminal", client_ip: "192.0.2.1", client_ip_attribution: "verified", can_revoke: status === "redeemed"});
const tick = () => new Promise((resolve) => setTimeout(resolve, 10));

test("status polling has one request in flight and stops on redemption; revoke refreshes once", async () => {
  let calls = 0;
  let complete!: (result: LoginCodeStatus) => void;
  let revoked = false;
  const states: LoginCodeMonitorState[] = [];
  const monitor = new LoginCodeMonitor({
    status: async () => { calls++; return revoked ? row("revoked") : new Promise((resolve) => { complete = resolve; }); },
    cancel: async () => { throw Error("unexpected cancellation"); },
    revoke: async (id) => { assert.equal(id, "fixture"); revoked = true; },
  }, "fixture", (state) => states.push(state), 1);
  monitor.start();
  await tick();
  assert.equal(calls, 1);
  complete(row("redeemed"));
  await tick();
  assert.equal(calls, 1);
  assert.equal(states.at(-1)?.status?.can_revoke, true);
  await monitor.act("revoke");
  await tick();
  assert.equal(calls, 2);
  assert.equal(states.at(-1)?.status?.status, "revoked");
  monitor.stop();
});

test("late pending response cannot overwrite cancellation or revive polling", async () => {
  let first!: (result: LoginCodeStatus) => void;
  let calls = 0;
  let cancelled = false;
  const states: LoginCodeMonitorState[] = [];
  const monitor = new LoginCodeMonitor({
    status: async () => { calls++; return cancelled ? row("cancelled") : new Promise((resolve) => { first = resolve; }); },
    cancel: async () => { cancelled = true; }, revoke: async () => {},
  }, "fixture", (state) => states.push(state), 1);
  monitor.start();
  await monitor.act("cancel");
  first(row("pending"));
  await tick();
  assert.equal(states.at(-1)?.status?.status, "cancelled");
  assert.equal(calls, 2);
  monitor.stop();
});

test("unmounted status responses never update the screen", async () => {
  let complete!: (result: LoginCodeStatus) => void;
  let notifications = 0;
  const monitor = new LoginCodeMonitor({status: () => new Promise((resolve) => { complete = resolve; }),
    cancel: async () => {}, revoke: async () => {}}, "fixture", () => notifications++, 1);
  monitor.start(); monitor.stop(); complete(row("redeemed"));
  await tick();
  assert.equal(notifications, 0);
});

test("failed cancellation reconciles a redemption that occurred around local expiry", async () => {
  let calls = 0;
  const states: LoginCodeMonitorState[] = [];
  const monitor = new LoginCodeMonitor({
    status: async () => { calls++; return {...row(calls === 1 ? "pending" : "redeemed"), expires_at: "2000-01-01T00:00:00Z"}; },
    cancel: async () => { throw Error("already redeemed"); }, revoke: async () => {},
  }, "fixture", (state) => states.push(state), 1);
  monitor.start();
  await Promise.resolve();
  await monitor.act("cancel");
  await tick();
  assert.equal(states.at(-1)?.status?.status, "redeemed");
  assert.equal(states.at(-1)?.status?.can_revoke, true);
  assert.equal(states.at(-1)?.error, false);
  monitor.stop();
});
