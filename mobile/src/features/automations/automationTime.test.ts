import assert from "node:assert/strict";
import test from "node:test";
import {
  automationInstant,
  formatAutomationTime,
  localTime,
} from "./automationTime";

test("calendar inputs resolve in the chosen timezone independently of device timezone", () => {
  assert.equal(
    automationInstant("2026-10-01T08:30", "Asia/Singapore"),
    "2026-10-01T00:30:00.000Z",
  );
  assert.equal(
    localTime("2026-10-01T00:30:00Z", "Asia/Singapore"),
    "2026-10-01T08:30",
  );
  assert.equal(
    automationInstant("2026-10-01T08:30", "Asia/Kathmandu"),
    "2026-10-01T02:45:00.000Z",
  );
  assert.equal(
    automationInstant("2026-10-01T08:30:23+05:45", "UTC"),
    "2026-10-01T02:45:23.000Z",
  );
  assert.equal(formatAutomationTime(null, "UTC"), "—");
  assert.equal(
    formatAutomationTime("2026-10-01T00:30:00Z", "Asia/Singapore"),
    new Intl.DateTimeFormat(undefined, {
      timeZone: "Asia/Singapore",
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date("2026-10-01T00:30:00Z")),
  );
});

test("DST folds choose the earlier instant and gaps collapse to the first valid minute", () => {
  assert.equal(
    automationInstant("2026-03-08T02:30", "America/New_York"),
    "2026-03-08T07:00:00.000Z",
  );
  assert.equal(
    automationInstant("2026-11-01T01:30", "America/New_York"),
    "2026-11-01T05:30:00.000Z",
  );
  assert.equal(
    automationInstant("2026-10-04T02:15", "Australia/Lord_Howe"),
    "2026-10-03T15:30:00.000Z",
  );
  assert.equal(
    automationInstant("2011-12-30T12:00", "Pacific/Apia"),
    "2011-12-30T10:00:00.000Z",
  );
});

test("invalid dates and timezones are rejected rather than normalized silently", () => {
  for (const value of [
    "2026-02-30T09:00",
    "2026-10-01",
    "2026-10-01T25:00",
    "",
  ]) {
    assert.throws(() => automationInstant(value, "UTC"));
  }
  assert.throws(() => automationInstant("2026-10-01T09:00", "not/a/timezone"));
});
