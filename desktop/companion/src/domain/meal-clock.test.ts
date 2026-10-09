import { describe, expect, it } from "vitest";

import { createDefaultCompanionSettings } from "./companion";
import {
  dateKeyAt,
  dueLabel,
  findDueMeal,
  instantForLocalTime,
  nextMealOccurrence,
} from "./meal-clock";

describe("meal clock", () => {
  it("selects the next enabled meal across a local day boundary", () => {
    const settings = createDefaultCompanionSettings("Asia/Shanghai");
    const next = nextMealOccurrence(
      settings.meals,
      new Date("2026-10-09T15:30:00.000Z"),
      settings.timezone,
    );

    expect(
      dateKeyAt(new Date("2026-10-09T15:30:00.000Z"), settings.timezone),
    ).toBe("2026-10-09");
    expect(next).toEqual({
      mealId: "breakfast",
      label: "Breakfast",
      dateKey: "2026-10-10",
      scheduledAt: "2026-10-10T00:00:00.000Z",
    });
  });

  it("treats a meal as due at the exact scheduled minute", () => {
    const settings = createDefaultCompanionSettings("UTC");
    const due = findDueMeal(
      settings.meals,
      new Date("2026-10-09T12:30:00.000Z"),
      settings.timezone,
    );

    expect(due?.mealId).toBe("lunch");
    expect(due?.scheduledAt).toBe("2026-10-09T12:30:00.000Z");
  });

  it("uses the first occurrence during a fall-back overlap", () => {
    expect(
      instantForLocalTime(
        "2026-11-01",
        "01:30",
        "America/New_York",
      ).toISOString(),
    ).toBe("2026-11-01T05:30:00.000Z");
  });

  it("advances a nonexistent spring-forward time to the first real minute", () => {
    expect(
      instantForLocalTime(
        "2026-03-08",
        "02:30",
        "America/New_York",
      ).toISOString(),
    ).toBe("2026-03-08T07:00:00.000Z");
  });

  it("returns structured due labels without embedding display copy", () => {
    expect(
      dueLabel(
        new Date("2026-10-10T00:15:00.000Z"),
        new Date("2026-10-09T23:30:00.000Z"),
        "UTC",
      ),
    ).toEqual({ kind: "tomorrow", time: "00:15" });
    expect(
      dueLabel(
        new Date("2026-10-09T12:29:20.000Z"),
        new Date("2026-10-09T12:30:00.000Z"),
        "UTC",
      ),
    ).toEqual({ kind: "now" });
  });
});
