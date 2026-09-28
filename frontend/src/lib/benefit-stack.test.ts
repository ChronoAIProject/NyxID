import { describe, expect, it } from "vitest";
import {
  equalShareStack,
  overallCaption,
  proportionalStack,
  stackValueText,
  type StackInput,
} from "./benefit-stack";

const input = (key: string, used: number, limit = 1000): StackInput => ({
  key,
  label: key,
  short: key,
  used,
  limit,
  legend: `${limit - used} left`,
  detail: `${limit - used} of ${limit} left`,
});

describe("equalShareStack", () => {
  const stack = equalShareStack([
    input("input", 412),
    input("output", 244),
    input("cache", 0),
    input("images", 240),
    input("requests", 0),
  ]);

  it("gives each allowance an equal share filled by its own utilization", () => {
    const widths = stack.entries.map((entry) => entry.share);
    [8.24, 4.88, 0, 4.8, 0].forEach((expected, index) =>
      expect(widths[index]).toBeCloseTo(expected, 10),
    );
    expect(stack.overall).toBeCloseTo(17.92, 10);
    expect(overallCaption(stack.overall)).toBe("18% used");
  });

  it("assigns a distinct opacity step per allowance, in order", () => {
    expect(stack.entries.map((entry) => entry.step)).toEqual([0, 1, 2, 3, 4]);
  });

  it("lists the overall and every allowance for screen readers", () => {
    expect(stackValueText(stack)).toBe(
      "18% used; input 41.2% used; output 24.4% used; cache 0% used; images 24% used; requests 0% used",
    );
  });

  it("merges allowances beyond five into one lowest-step tail entry", () => {
    const long = equalShareStack(
      ["a", "b", "c", "d", "e", "f", "g"].map((key, index) =>
        input(key, index === 6 ? 1000 : 100),
      ),
    );
    expect(long.entries).toHaveLength(5);
    const tail = long.entries.at(-1)!;
    expect(tail).toMatchObject({
      key: "more",
      short: "3 more",
      step: 4,
      merged: 3,
    });
    // e and f at 10%, g spent: (0.1 + 0.1 + 1) / 7 of the bar.
    expect(tail.share).toBeCloseTo((1.2 / 7) * 100, 10);
    // A spent allowance in the tail still marks the tail entry.
    expect(tail.status).toBe("exhausted");
    expect(long.items.map((item) => item.step)).toEqual([0, 1, 2, 3, 4, 4, 4]);
    expect(long.overall).toBeCloseTo(
      long.entries.reduce((sum, entry) => sum + entry.share, 0),
      5,
    );
  });

  it("flags one spent allowance even when the average is low", () => {
    const low = equalShareStack([
      input("tokens", 0),
      input("requests", 1000),
      input("images", 0),
      input("bytes", 0),
    ]);
    expect(low.status).toBe("normal");
    expect(low.entries[1]).toMatchObject({ status: "exhausted" });
    expect(overallCaption(low.overall)).toBe("25% used");
  });

  it("turns warning at an 80% average and exhausted when all are spent", () => {
    expect(equalShareStack([input("a", 800), input("b", 800)]).status).toBe(
      "warning",
    );
    const spent = equalShareStack([
      input("a", 1000),
      input("b", 1000),
      input("c", 1000),
    ]);
    expect(spent.overall).toBe(100);
    expect(spent.status).toBe("exhausted");
    expect(overallCaption(spent.overall)).toBe("100% used");
  });

  it("never rounds an unspent allowance up to 100% and reads unused as such", () => {
    expect(overallCaption(99.9)).toBe("99.9% used");
    expect(overallCaption(0.71)).toBe("0.71% used");
    expect(overallCaption(0)).toBe("Unused");
    expect(equalShareStack([input("a", 0)]).entries[0]!.share).toBe(0);
  });
});

describe("proportionalStack", () => {
  it("sizes each grant by its consumed credits over all original credits", () => {
    const stack = proportionalStack([
      input("welcome", 5, 10),
      input("promo", 3, 30),
    ]);
    expect(stack.entries.map((entry) => entry.share)).toEqual([12.5, 7.5]);
    expect(stack.entries.map((entry) => entry.step)).toEqual([0, 1]);
    expect(stack.overall).toBe(20);
  });

  it("is one segment for a single grant", () => {
    const stack = proportionalStack([input("welcome", 35_500, 5_000_000)]);
    expect(stack.entries).toHaveLength(1);
    expect(overallCaption(stack.overall)).toBe("0.71% used");
  });
});
