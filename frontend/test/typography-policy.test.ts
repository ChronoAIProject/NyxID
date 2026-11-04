import { describe, expect, it } from "vitest";
// @ts-expect-error The lint guard is deliberately a standalone Node script.
import { checkTypography, typographyViolations } from "../scripts/check-typography.mjs";
import { resolve } from "node:path";

describe("typography policy", () => {
  it("keeps source tokens canonical and role consumers scalable", () => {
    expect(checkTypography(resolve("src"))).toEqual([]);
  });
  it.each([
    ['"sm:group-hover:text-10"', "small variant"],
    ['"dark:text-xs"', "Tailwind alias"],
    ['"text-[12px]"', "arbitrary pixel class"],
    ['"text-[0.85em]"', "arbitrary relative code size"],
    ['const tick = { fontSize: 10 };', "fixed chart number"],
    ['<text fontSize={12}>Label</text>', "fixed SVG number"],
  ])("rejects %s (%s)", (source) => {
    expect(typographyViolations("components/example.tsx", source)).not.toEqual([]);
  });
  it("checks CSS consumers, font shorthands, and contextual overrides", () => {
    expect(typographyViolations("other.css", ":root { --text-12: 1rem; }")).toHaveLength(2);
    expect(typographyViolations("other.css", ".label {font: 500 11px sans-serif; font-size: 0.75rem;}")).toHaveLength(2);
    expect(typographyViolations("app.css", "html { --text-12: 0.75rem; }")).toHaveLength(1);
    expect(typographyViolations("other.css", ".label {font-size: var(--text-11);}")).toHaveLength(1);
    expect(typographyViolations("other.css", ".label {@apply text-sm;}")).toHaveLength(1);
  });
  it("permits canonical CSS and explicitly unthemed public assignments", () => {
    expect(typographyViolations("app.css", "@theme {--text-12: 0.75rem;} .body {font-size: var(--text-14);}")).toEqual([]);
    expect(typographyViolations("components/ui/badge.tsx", '"text-12 unthemed:text-10"')).toEqual([]);
    expect(typographyViolations("features/blog/article.tsx", '"text-11"')).toEqual([]);
    expect(typographyViolations("chart.ts", 'const tick = {fontSize: "var(--text-13)"};')).toEqual([]);
  });
});
