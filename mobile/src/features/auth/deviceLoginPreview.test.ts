import assert from "node:assert/strict";
import test from "node:test";

import {
  formatDeviceLoginRelativeTime,
  resolveDeviceLoginDeadlineMs,
  secondsUntilDeviceLoginDeadline,
} from "./deviceLoginPreview";

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
