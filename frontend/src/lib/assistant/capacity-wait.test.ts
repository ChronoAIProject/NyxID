import { describe, expect, it } from "vitest";
import { formatCapacityWait } from "./capacity-wait";
import { storedError } from "./nyxagent-transport";

describe("capacity wait copy", () => {
  it.each([
    [0, "0 seconds"], [1, "1 second"], [1000, "1 second"], [45000, "45 seconds"],
    [60000, "1 minute"], [65000, "1 minute 5 seconds"], [120000, "2 minutes"],
    [120001, "2 minutes 1 second"],
  ])("formats %i ms consistently", (ms, copy) => {
    expect(formatCapacityWait(ms)).toBe(copy);
    expect(storedError("capacity_exceeded", ms)).toBe(ms > 0
      ? `NyxBot is at capacity right now (waited ${copy}). Try again shortly.`
      : "NyxBot is at capacity right now. Try again shortly.");
  });
  it.each([null, undefined, 0])("omits an empty or unknown wait: %s", (ms) => {
    expect(storedError("capacity_exceeded", ms)).not.toContain("waited");
  });
});
