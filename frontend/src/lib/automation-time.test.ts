import { describe, expect, it } from "vitest";
import {
  automationInstant,
  formatAutomationTime,
  localTime,
} from "./automation-time";
import { parseAutomationSearch } from "./automation-search";

describe("automation calendar input", () => {
  it("uses the selected zone independently of the browser and round trips", () => {
    expect(automationInstant("2026-10-01T08:30", "Asia/Singapore")).toBe(
      "2026-10-01T00:30:00.000Z",
    );
    expect(localTime("2026-10-01T00:30:00Z", "Asia/Singapore")).toBe(
      "2026-10-01T08:30",
    );
    expect(formatAutomationTime("2026-10-01T00:30:00Z", "Asia/Singapore")).toBe(
      new Intl.DateTimeFormat(undefined, {
        timeZone: "Asia/Singapore",
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date("2026-10-01T00:30:00Z")),
    );
  });
  it("collapses spring gaps and chooses the earlier fall-fold instant", () => {
    expect(automationInstant("2026-03-08T02:30", "America/New_York")).toBe(
      "2026-03-08T07:00:00.000Z",
    );
    expect(automationInstant("2026-11-01T01:30", "America/New_York")).toBe(
      "2026-11-01T05:30:00.000Z",
    );
    expect(() => automationInstant("2026-02-30T09:00", "UTC")).toThrow();
  });
  it("accepts only the opaque setup reference and agent from router search", () => {
    const setup = "12345678-1234-4123-8123-123456789abc";
    expect(
      parseAutomationSearch({
        setup,
        agent: "nyxbot",
        instruction: "private",
        label: "private",
      }),
    ).toEqual({ setup, agent: "nyxbot" });
    expect(parseAutomationSearch({ setup: "invalid", agent: 123 })).toEqual({});
  });
});
