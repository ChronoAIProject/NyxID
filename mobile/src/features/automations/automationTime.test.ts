import assert from "node:assert/strict";
import test from "node:test";
import { formatAutomationTime } from "./automationTime";

test("automation display uses the saved timezone independently of the device and handles missing times", () => {
  assert.equal(formatAutomationTime(null, "UTC"), "—");
  assert.equal(formatAutomationTime(undefined, "UTC"), "—");
  const instant = "2026-10-01T00:30:00Z";
  for (const zone of ["Asia/Singapore", "America/New_York", "Asia/Kathmandu"]) {
    assert.equal(
      formatAutomationTime(instant, zone),
      new Intl.DateTimeFormat(undefined, {
        timeZone: zone,
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(instant)),
    );
  }
});
