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
    ['"font-display text-28 font-medium"', "underweighted prominent metric"],
    ['<span className={cn("font-display font-medium", compact ? "text-22" : "text-28")} />', "split metric assignment"],
    ['"font-display text-28"', "metric without a weight"],
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
  it("enforces the custom onboarding button role's size and weight", () => {
    expect(typographyViolations("onboarding.css", ".nyxbot-onboarding .nb-provider {font-size: var(--text-14);font-weight: 400;}")).toHaveLength(1);
    expect(typographyViolations("onboarding.css", ".nb-device-login > button {font-size: var(--text-13);font-weight: 500;}")).toHaveLength(1);
    expect(typographyViolations("onboarding.css", ".nb-primary,.nb-secondary {font-size: var(--text-14);font-weight: 500;}")).toEqual([]);
  });
});
