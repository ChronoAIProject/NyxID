import { describe, expect, it } from "vitest";
// @ts-expect-error The lint guard is deliberately a standalone Node script.
import { checkTypography, typographyViolations } from "../scripts/check-typography.mjs";
import { readFileSync, readdirSync } from "node:fs";
import postcss from "postcss";
import { resolve } from "node:path";
import { TEXT_STYLES } from "../src/lib/typography";

describe("typography policy", () => {
  it("never scopes shared treatment to the assistant shell", () => {
    for (const file of readdirSync("src", { recursive: true }).filter(file => String(file).endsWith(".css"))) {
      postcss.parse(readFileSync(resolve("src", String(file)), "utf8")).walkRules(rule => {
        if (rule.selector.includes("assistant-display")) {
          expect(String(file)).toBe("app.css");
          expect(rule.selector).toBe("html.assistant-display [data-chat-index-target]");
        }
      });
    }
    expect(readFileSync(resolve("src/hooks/use-theme.ts"), "utf8")).not.toContain("--assistant-density");
  });
  it("keeps source tokens canonical and role consumers scalable", () => {
    expect(checkTypography(resolve("src"))).toEqual([]);
  });
  it("every role owns size, weight and its line box in one shared utility", () => {
    const roles = postcss.parse(readFileSync("src/typography.css", "utf8"));
    const found = new Map<string, Record<string, string>>();
    roles.walkAtRules("utility", rule => {
      found.set(rule.params, Object.fromEntries((rule.nodes ?? []).filter(node => node.type === "decl").map(node => [node.prop, node.value])));
    });
    for (const role of Object.values(TEXT_STYLES)) {
      expect(found.get(role)).toMatchObject({
        "font-size": expect.stringMatching(/^var\(--text-\d+\)$/),
        "font-weight": expect.stringMatching(/^[4567]00$/),
        "line-height": expect.stringMatching(/^var\(--line-[\w-]+\)$/),
      });
    }
    expect(found.get(TEXT_STYLES.fieldLabel)).toEqual({ "font-size": "var(--text-14)", "font-weight": "500", "line-height": "var(--line-body)" });
  });
  it.each([
    '<label htmlFor="name">Name</label>',
    '<Label className="text-13">Name</Label>',
    'const LABEL = "type-secondary-emphasis"; <FormLabel className={LABEL}>Name</FormLabel>',
    'const SIZE = "text-13"; <span className={cn(SIZE, "font-medium")}>Caption</span>',
    '<FormLabel className={TEXT_STYLES.body}>Name</FormLabel>',
    '<GroupedLabel className={cn("type-body", disabled && "opacity-50")}>Name</GroupedLabel>',
    '<span className="text-14 font-normal">Caption</span>',
    '<span className="!text-14 !font-normal">Caption</span>',
    '<span className="text-14! font-normal!">Caption</span>',
    '<span className="hover:!type-body hover:!font-medium" />',
    '<span className={cn("text-14", active && "font-medium")} />',
    '<span className="type-body leading-relaxed" />',
    '<span className="type-body font-medium" />',
    '<span className={cn(TEXT_STYLES.body, "font-medium")} />',
    '<span className={`${TEXT_STYLES.body} font-normal`} />',
    'const label = cva(`${TEXT_STYLES.fieldLabel} font-normal`)',
    '<span className="leading-6" />',
    '<span className="!leading-6" />',
    'const text = {lineHeight: 1.2}',
  ])("rejects independent label or line-box assignments: %s", source => {
    expect(typographyViolations("components/custom-form.tsx", source)).not.toEqual([]);
  });
  it("allows role consumers and documented prose / unthemed exceptions", () => {
    expect(typographyViolations("components/custom-form.tsx", '<Label className="flex-1 text-foreground">Name</Label><p className="type-prose" />')).toEqual([]);
    expect(typographyViolations("components/ui/input.tsx", '"type-input unthemed:text-xs unthemed:leading-none"')).toEqual([]);
    expect(typographyViolations("components/label.css", '.label {line-height:1.2}')).not.toEqual([]);
    expect(typographyViolations("components/label.css", '.label {@apply type-field-label;line-height:var(--line-body)}')).toEqual([]);
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
    ['"gap-[calc(0.75em*var(--assistant-density,1))]"', "obsolete shell-specific density"],
  ])("rejects %s (%s)", (source) => {
    expect(typographyViolations("components/example.tsx", source)).not.toEqual([]);
  });
  it("checks CSS consumers, font shorthands, and contextual overrides", () => {
    expect(typographyViolations("other.css", ":root { --text-12: 1rem; }")).toHaveLength(2);
    expect(typographyViolations("other.css", ".label {font: 500 11px sans-serif; font-size: 0.75rem;}")).toHaveLength(3);
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
    expect(typographyViolations("onboarding.css", ".nyxbot-onboarding .nb-provider {font-size: var(--text-14);font-weight: 400;}")).not.toEqual([]);
    expect(typographyViolations("onboarding.css", ".nb-device-login > button {font-size: var(--text-13);font-weight: 500;}")).not.toEqual([]);
    expect(typographyViolations("onboarding.css", ".nb-primary,.nb-secondary {@apply type-button;}")).toEqual([]);
  });
});
