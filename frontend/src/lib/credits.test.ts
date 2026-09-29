import { describe, expect, it } from "vitest";
import {
  decimalCredits,
  exactCredits,
  formatExactCredits,
  legacyCredits,
  parseCredits,
} from "./credits";
import { total } from "./billing-display";
import { billingUsageRowSchema } from "@/schemas/billing";

const row = (amount: string) =>
  billingUsageRowSchema.parse({
    metric: "cache_read_tokens",
    lago_metric_code: "cache",
    layer: "platform",
    quantity: 1,
    requests: 0,
    bytes: 0,
    events: 1,
    lago_acked: true,
    estimated_credits_micros: 0,
    estimated_credits: amount,
  });

describe("exact credits", () => {
  it("preserves fractions and large values without floating point", () => {
    const amount = "9999999999999999999999.999999999999";
    expect(decimalCredits(parseCredits(amount))).toBe(amount);
    expect(decimalCredits(parseCredits("0.0000008") * 44_608n)).toBe(
      "0.0356864",
    );
    expect(legacyCredits(35_686)).toBe("0.035686");
    expect(() => legacyCredits(Number.MAX_SAFE_INTEGER + 1)).toThrow();
  });
  it("never displays a sub-display-precision nonzero amount as zero", () => {
    expect(formatExactCredits("0.0000008")).toBe("<0.000001");
    expect(formatExactCredits("-0.0000008")).toBe(">-0.000001");
    expect(formatExactCredits("0.0356864")).toBe("0.035686");
    expect(formatExactCredits("0.0356865")).toBe("0.035687");
  });
  it("sums exact API values before formatting and preserves unknown values", () => {
    expect(
      total([row("0.0000008"), row("0.0000008")], "estimated_credits_micros"),
    ).toBe("0.0000016");
    expect(exactCredits(null, 123)).toBeNull();
    expect(exactCredits(undefined, 123)).toBe("0.000123");
    expect(row("0.0000008").estimated_credits).toBe("0.0000008");
    expect(() => row("0.0000000000001")).toThrow();
  });
});

it("compact headlines preserve exact displayed digits", async () => {
  const { formatCompactCredits } = await import("./credits");
  expect(formatCompactCredits("1234.56789")).toBe("1.2K");
  expect(formatCompactCredits("9999999999999999.123456789012")).toBe("10,000T");
  expect(formatCompactCredits("-1250000.000000000001")).toBe("-1.3M");
  expect(formatCompactCredits("0.000000000001")).toBe("<0.000001");
});
