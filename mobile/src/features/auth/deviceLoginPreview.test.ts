import assert from "node:assert/strict";
import test from "node:test";

import {
  compareDeviceLoginTimezones,
  formatDeviceLoginOriginWarning,
  formatDeviceLoginRelativeTime,
  resolveDeviceLoginDeadlineMs,
  secondsUntilDeviceLoginDeadline,
} from "./deviceLoginPreview";

test("renders only negative initiating-origin states as security signals", () => {
  assert.equal(
    formatDeviceLoginOriginWarning("matched", "https://nyxid.dev"),
    null,
  );
  assert.equal(formatDeviceLoginOriginWarning("absent", null), null);
  assert.match(
    formatDeviceLoginOriginWarning("mismatched", "https://login-copy.example") ?? "",
    /started from login-copy\.example, not the official NyxID site/,
  );
  assert.match(
    formatDeviceLoginOriginWarning("non_http", "file:\/\/\/tmp\/login.html") ?? "",
    /non-HTTP\(S\) initiating origin/,
  );
  assert.match(
    formatDeviceLoginOriginWarning("malformed", "not a url") ?? "",
    /Origin header was malformed/,
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
  assert.equal(compareDeviceLoginTimezones(null, "Asia/Singapore"), "unavailable");
  assert.equal(compareDeviceLoginTimezones("Europe/Moscow", undefined), "unavailable");
});

test("uses backend remaining seconds as the countdown anchor", () => {
  const now = Date.parse("2026-08-20T10:00:00Z");
  assert.equal(
    resolveDeviceLoginDeadlineMs("2026-08-20T10:20:00Z", 32, now),
    now + 32_000,
  );
  assert.equal(
    secondsUntilDeviceLoginDeadline(now + 1_500, now),
    2,
  );
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
