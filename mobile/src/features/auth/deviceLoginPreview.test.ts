import assert from "node:assert/strict";
import test from "node:test";

import {
  compareDeviceLoginTimezones,
  formatDeviceLoginOriginValue,
  formatDeviceLoginRelativeTime,
  resolveDeviceLoginDeadlineMs,
  resolveDeviceLoginValueTones,
  secondsUntilDeviceLoginDeadline,
} from "./deviceLoginPreview";

test("formats only negative initiating-origin states as terse row values", () => {
  assert.equal(
    formatDeviceLoginOriginValue("matched", "https://nyxid.dev"),
    null,
  );
  assert.equal(formatDeviceLoginOriginValue("absent", null), null);
  assert.equal(
    formatDeviceLoginOriginValue("mismatched", "https://login-copy.example"),
    "login-copy.example",
  );
  assert.equal(
    formatDeviceLoginOriginValue("non_http", "file:///tmp/login.html"),
    "Non-HTTP origin",
  );
  assert.equal(
    formatDeviceLoginOriginValue("malformed", "not a url"),
    "Malformed origin",
  );
});

test("compares a requester timezone with the approving phone without inventing absence", () => {
  assert.equal(
    compareDeviceLoginTimezones("Europe/Moscow", "Asia/Singapore"),
    "different",
  );
  assert.equal(
    compareDeviceLoginTimezones("Asia/Singapore", "Asia/Singapore"),
    "same",
  );
  assert.equal(
    compareDeviceLoginTimezones(null, "Asia/Singapore"),
    "unavailable",
  );
  assert.equal(
    compareDeviceLoginTimezones("Europe/Moscow", undefined),
    "unavailable",
  );
});

test("prioritizes one anomalous row value beside the fixed caution accent", () => {
  assert.deepEqual(resolveDeviceLoginValueTones(false, false, 600), {
    origin: "default",
    timezone: "default",
    expiry: "default",
  });
  assert.deepEqual(resolveDeviceLoginValueTones(true, true, 30), {
    origin: "danger",
    timezone: "default",
    expiry: "default",
  });
  assert.deepEqual(resolveDeviceLoginValueTones(false, true, 30), {
    origin: "default",
    timezone: "warning",
    expiry: "default",
  });
  assert.deepEqual(resolveDeviceLoginValueTones(false, false, 30), {
    origin: "default",
    timezone: "default",
    expiry: "warning",
  });
  assert.deepEqual(resolveDeviceLoginValueTones(false, false, 0), {
    origin: "default",
    timezone: "default",
    expiry: "danger",
  });
});

test("uses backend remaining seconds as the countdown anchor", () => {
  const now = Date.parse("2026-08-20T10:00:00Z");
  assert.equal(
    resolveDeviceLoginDeadlineMs("2026-08-20T10:20:00Z", 32, now),
    now + 32_000,
  );
  assert.equal(secondsUntilDeviceLoginDeadline(now + 1_500, now), 2);
  assert.equal(secondsUntilDeviceLoginDeadline(now - 1, now), 0);
});

test("falls back to expires_at for stale backends and formats exact recent age", () => {
  const now = Date.parse("2026-08-20T10:00:00Z");
  assert.equal(
    resolveDeviceLoginDeadlineMs("2026-08-20T10:01:00Z", null, now),
    now + 60_000,
  );
  assert.equal(
    formatDeviceLoginRelativeTime("2026-08-20T09:59:28Z", now),
    "32 seconds ago",
  );
});
