import { describe, it, expect } from "vitest";
import { formatBucketDate, formatBucketLabel } from "./usage-bucket";

describe("formatBucketDate", () => {
  it("formats an ISO date as month + day", () => {
    expect(formatBucketDate("2026-04-15")).toBe("Apr 15");
  });

  it("returns the raw string when the date cannot be parsed", () => {
    expect(formatBucketDate("not-a-date")).toBe("not-a-date");
    expect(formatBucketDate("2026-04")).toBe("2026-04");
    expect(formatBucketDate("2026-XX-15")).toBe("2026-XX-15");
  });
});

describe("formatBucketLabel", () => {
  it("uses singular request label for a count of 1", () => {
    expect(
      formatBucketLabel({ date: "2026-04-15", request_count: 1, error_count: 0 }),
    ).toBe("Apr 15: 1 request");
  });

  it("uses plural request label for any other count", () => {
    expect(
      formatBucketLabel({ date: "2026-04-14", request_count: 25, error_count: 0 }),
    ).toBe("Apr 14: 25 requests");
    expect(
      formatBucketLabel({ date: "2026-04-13", request_count: 0, error_count: 0 }),
    ).toBe("Apr 13: 0 requests");
  });

  it("appends error count when greater than zero, with singular/plural", () => {
    expect(
      formatBucketLabel({ date: "2026-04-14", request_count: 25, error_count: 1 }),
    ).toBe("Apr 14: 25 requests, 1 error");
    expect(
      formatBucketLabel({ date: "2026-04-14", request_count: 25, error_count: 3 }),
    ).toBe("Apr 14: 25 requests, 3 errors");
  });

  it("omits errors when error count is zero", () => {
    expect(
      formatBucketLabel({ date: "2026-04-15", request_count: 1, error_count: 0 }),
    ).toBe("Apr 15: 1 request");
  });
});
