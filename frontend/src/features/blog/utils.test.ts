import { describe, it, expect } from "vitest";
import { estimateReadingMinutes } from "./utils";

describe("estimateReadingMinutes", () => {
  it("floors at 1 minute even for empty or tiny bodies", () => {
    expect(estimateReadingMinutes("")).toBe(1);
    expect(estimateReadingMinutes("just a few words here")).toBe(1);
  });

  it("rounds to nearest minute at ~225 wpm", () => {
    // 225 words -> exactly 1 minute
    expect(estimateReadingMinutes(Array(225).fill("word").join(" "))).toBe(1);
    // 450 words -> 2 minutes
    expect(estimateReadingMinutes(Array(450).fill("word").join(" "))).toBe(2);
    // 338 words -> 1.5 -> rounds to 2
    expect(estimateReadingMinutes(Array(338).fill("word").join(" "))).toBe(2);
    // 280 words -> 1.24 -> rounds to 1
    expect(estimateReadingMinutes(Array(280).fill("word").join(" "))).toBe(1);
  });

  it("ignores surrounding whitespace when counting words", () => {
    expect(estimateReadingMinutes("  \n\n  hello   world \t ")).toBe(1);
  });
});
