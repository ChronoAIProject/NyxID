import { describe, expect, it } from "vitest";
import {
  nyxbotDailyData,
  nyxbotDateRange,
  nyxbotWindow,
} from "./assistant-analytics";
import type { AssistantAnalytics } from "@/schemas/assistant-analytics";

describe("NyxBot analytics dates", () => {
  it("uses UTC calendar days including today across month boundaries", () => {
    expect(nyxbotDateRange(7, new Date("2026-03-01T23:59:00-08:00"))).toEqual({
      from: "2026-02-24",
      to: "2026-03-02",
    });
  });
  it("uses an exclusive next-day upper bound and validates the inclusive day count", () => {
    expect(nyxbotWindow("2026-01-01", "2026-01-31")).toEqual({
      from: "2026-01-01T00:00:00.000Z",
      to: "2026-02-01T00:00:00.000Z",
    });
    expect(nyxbotWindow("2026-01-01", "2026-02-01")).toBeNull();
    expect(nyxbotWindow("2026-01-02", "2026-01-01")).toBeNull();
    expect(nyxbotWindow("", "2026-01-01")).toBeNull();
  });
  it("fills missing dates and keeps activity and feedback populations separate", () => {
    const analytics = {
      daily_sessions: [{ day: "2026-01-02", sessions: 4 }],
      daily_turns: [{ day: "2026-01-01", turns: 3 }],
    } as AssistantAnalytics;
    const data = nyxbotDailyData(
      "2026-01-01T00:00:00Z",
      "2026-01-04T00:00:00Z",
      analytics,
      { counts: [{ day: "2026-01-02", up: 2, down: 1 }], recent: [] },
    );
    expect(data).toEqual([
      { day: "2026-01-01", sessions: 0, turns: 3, up: 0, down: 0 },
      { day: "2026-01-02", sessions: 4, turns: 0, up: 2, down: 1 },
      { day: "2026-01-03", sessions: 0, turns: 0, up: 0, down: 0 },
    ]);
  });
});
