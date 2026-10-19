import { describe, expect, it } from "vitest";
import { billingRow as row } from "@/test/billing-fixture";
import { creditsLabel, creditsSumLabel, knownTotal } from "./billing-display";

const estimated = "estimated_credits_micros";

describe("knownTotal", () => {
  it("is zero with nothing unknown for no rows", () => {
    expect(knownTotal([], estimated)).toEqual({ sum: 0n, unknown: 0 });
  });
  it("sums priced rows and counts null or missing values as unknown", () => {
    const rows = [
      row({ estimated_credits_micros: 1_500_000 }),
      row({ estimated_credits_micros: null }),
      row({ estimated_credits_micros: 0 }),
      row({ estimated_credits_micros: undefined }),
      row({ estimated_credits_micros: 250 }),
    ];
    expect(knownTotal(rows, estimated)).toEqual({
      sum: 1_500_250_000_000n,
      unknown: 2,
    });
  });
  it("prefers exact decimal strings and treats an exact null as unknown", () => {
    const rows = [
      // The exact value wins over its truncated micro display.
      row({ estimated_credits: "0.0000008", estimated_credits_micros: 0 }),
      row({ estimated_credits: "0.0000008", estimated_credits_micros: 0 }),
      row({ estimated_credits: null, estimated_credits_micros: 2440 }),
    ];
    expect(knownTotal(rows, estimated)).toEqual({
      sum: 1_600_000n,
      unknown: 1,
    });
  });
  it("reads only the requested field", () => {
    const rows = [row({ wallet_credits_micros: null })];
    expect(knownTotal(rows, "wallet_credits_micros")).toEqual({
      sum: 0n,
      unknown: 1,
    });
    expect(knownTotal(rows, "grant_credits_micros")).toEqual({
      sum: 2_440_000_000n,
      unknown: 0,
    });
  });
});

describe("creditsLabel", () => {
  it("shows zero credits for no rows", () => {
    expect(creditsLabel([], estimated)).toEqual({
      text: "0",
      partial: false,
      unknown: 0,
    });
  });
  it("is unavailable when every row is unpriced", () => {
    const rows = [
      row({ estimated_credits_micros: null }),
      row({ estimated_credits_micros: null }),
    ];
    expect(creditsLabel(rows, estimated)).toEqual({
      text: "Unavailable",
      partial: false,
      unknown: 2,
    });
  });
  it("marks a partial sum as a lower bound", () => {
    const rows = [
      row({ estimated_credits_micros: 2440 }),
      row({ estimated_credits_micros: null }),
    ];
    expect(creditsLabel(rows, estimated)).toEqual({
      text: "≥\u00a00.00244",
      partial: true,
      unknown: 1,
    });
  });
  describe("with free rows", () => {
    // The backend always prices non-billable rows at zero.
    const free = () =>
      row({
        billable: false,
        lago_acked: false,
        estimated_credits_micros: 0,
        wallet_credits_micros: 0,
        grant_credits_micros: 0,
        allowance_credits_micros: 0,
      });
    it("stays unavailable when the only charged row is unpriced", () => {
      const rows = [free(), row({ estimated_credits_micros: null })];
      expect(creditsLabel(rows, estimated)).toEqual({
        text: "Unavailable",
        partial: false,
        unknown: 1,
      });
    });
    it("is a lower bound when some charged row is priced", () => {
      const rows = [free(), row(), row({ estimated_credits_micros: null })];
      expect(creditsLabel(rows, estimated)).toEqual({
        text: "≥\u00a00.00244",
        partial: true,
        unknown: 1,
      });
    });
    it("is zero when every row is free", () => {
      expect(creditsLabel([free(), free()], estimated)).toEqual({
        text: "0",
        partial: false,
        unknown: 0,
      });
    });
  });
  it("formats a sub-display-precision lower bound without rounding it to zero", () => {
    const rows = [
      row({ estimated_credits: "0.0000008" }),
      row({ estimated_credits: null }),
    ];
    expect(creditsLabel(rows, estimated)).toEqual({
      text: "≥\u00a0<0.000001",
      partial: true,
      unknown: 1,
    });
  });
  it("formats a complete sum from micros to credits", () => {
    const rows = [
      row({ estimated_credits_micros: 1_234_567_890 }),
      row({ estimated_credits_micros: 10 }),
    ];
    expect(creditsLabel(rows, estimated)).toEqual({
      text: "1,234.5679",
      partial: false,
      unknown: 0,
    });
  });
});

describe("creditsSumLabel", () => {
  const benefits = [
    "grant_credits_micros",
    "allowance_credits_micros",
  ] as const;
  it("adds every field when all values are known", () => {
    const rows = [
      row({ grant_credits_micros: 1000, allowance_credits_micros: 1200 }),
    ];
    expect(creditsSumLabel(rows, benefits)).toEqual({
      text: "0.0022",
      partial: false,
      unknown: 0,
    });
  });
  it("is a lower bound when either field is partly unknown", () => {
    const rows = [
      row({ grant_credits_micros: 1000, allowance_credits_micros: null }),
      row({ grant_credits_micros: 500, allowance_credits_micros: 200 }),
    ];
    expect(creditsSumLabel(rows, benefits)).toEqual({
      text: "≥\u00a00.0017",
      partial: true,
      unknown: 1,
    });
  });
  it("stays a lower bound when only one field is entirely unknown", () => {
    const rows = [
      row({ grant_credits_micros: 1000, allowance_credits_micros: null }),
      row({ grant_credits_micros: 500, allowance_credits_micros: null }),
    ];
    expect(creditsSumLabel(rows, benefits).text).toBe("≥\u00a00.0015");
  });
  it("is unavailable only when both fields are entirely unknown", () => {
    const rows = [
      row({ grant_credits_micros: null, allowance_credits_micros: null }),
    ];
    expect(creditsSumLabel(rows, benefits)).toEqual({
      text: "Unavailable",
      partial: false,
      unknown: 2,
    });
  });
});
