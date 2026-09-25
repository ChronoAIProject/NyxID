import { describe, expect, it } from "vitest";
import {
  billingMetricLabel,
  formatAllowancePreview,
  resolveServiceBillingMetric,
} from "./billing-units";

describe("billing unit presentation", () => {
  it("uses the backend-resolved service metric without a frontend heuristic", () => {
    expect(
      resolveServiceBillingMetric({ effective_platform_metric: "bytes" }),
    ).toBe("bytes");
  });

  it("formats a live compact monthly preview", () => {
    expect(
      formatAllowancePreview(1_000_000, "tokens", "monthly", "en-US"),
    ).toBe("1,000,000 tokens (1M) free each month");
  });

  it("reflects one-time recurrence and singular units", () => {
    expect(formatAllowancePreview(1, "requests", "one_time", "en-US")).toBe(
      "1 request free once",
    );
    expect(billingMetricLabel("bytes", 1)).toBe("byte");
  });

  it("omits a preview for invalid quantities", () => {
    expect(formatAllowancePreview(Number.NaN, "tokens", "daily")).toBeNull();
    expect(formatAllowancePreview(0, "tokens", "daily")).toBeNull();
  });
});

it("labels component units and preserves future units", () => {
  expect(billingMetricLabel("input_tokens")).toBe("input tokens");
  expect(billingMetricLabel("cache_write_tokens", 1)).toBe("cache-write token");
  expect(billingMetricLabel("images", 1)).toBe("image");
  expect(billingMetricLabel("future_unit", 1)).toBe("future_unit");
});
