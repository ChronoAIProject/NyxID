import { describe, expect, it, vi } from "vitest";

vi.mock("@/i18n", () => {
  throw new Error(
    "Standalone formatting must not import application i18n or its catalogs",
  );
});

import { formatDate, formatRelativeTime, formatTimeDistance } from "./utils";

describe("standalone formatting", () => {
  it("keeps English defaults without initializing application localization", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date("2024-06-15T12:00:00Z"));
      expect(formatDate("2024-06-15T12:00:00Z")).toMatch(/Jun\s+15,\s+2024/);
      expect(formatRelativeTime("2024-06-15T11:45:00Z")).toBe("15m ago");
      expect(formatTimeDistance("2024-06-15T15:00:00Z")).toBe("in 3h");
    } finally {
      vi.useRealTimers();
    }
  });
});
