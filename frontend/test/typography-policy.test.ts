import { beforeAll, describe, expect, it } from "vitest";
// @ts-expect-error The lint guard is deliberately a standalone Node script.
import { checkTypography, typographyViolations } from "../scripts/check-typography.mjs";
import { readFileSync, readdirSync } from "node:fs";
import postcss from "postcss";
import { resolve } from "node:path";
import { TEXT_STYLES } from "../src/lib/typography";

describe("typography policy", () => {
  let sourceViolations: string[];

  // This one-time static analysis reads and parses the repository. Coverage and
  // shared CI CPU can exceed a test's 5s budget; only the scan setup gets 30s.
  beforeAll(() => {
    sourceViolations = checkTypography(resolve("src"));
  }, 30_000);

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
    expect(sourceViolations).toEqual([]);
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
    expect(found.get(TEXT_STYLES.countBadge)).toEqual({ "font-size": "var(--text-10)", "font-weight": "600", "line-height": "var(--line-count)" });
    expect(readFileSync("src/typography.css", "utf8")).toContain("--line-count: 1;");
    expect(found.get(TEXT_STYLES.fieldLabel)).toEqual({ "font-size": "var(--text-14)", "font-weight": "500", "line-height": "var(--line-body)" });
  });
  it.each([
    '<label htmlFor="name">Name</label>',
    '<Label style={{fontWeight:400}}>Name</Label>',
    '<Label style={{fontSize:"var(--text-13)"}}>Name</Label>',
    'const OVERRIDE={fontWeight:400}; <Label style={OVERRIDE}>Name</Label>',
    '<Label {...{style:{fontWeight:400}}}>Name</Label>',
    '<Label {...{className:"type-secondary"}}>Name</Label>',
    'const props = {className:"type-secondary"}; <Label {...props}>Name</Label>',
    '<Label><span {...{className:"type-secondary"}}>Name</span><Input /></Label>',
    'const captionProps = {className:"type-secondary"}; <Label><span {...captionProps}>Name</span><Input /></Label>',
    'const caption = {className:"type-secondary"}; const props = {...caption}; <Label><span {...{...props}}>Name</span><Input /></Label>',
    'const className = "type-secondary"; const props = {className}; <Label><span {...props}>Name</span><Input /></Label>',
    'const weight = {fontWeight:600}; const style = {...weight}; const props = {style}; <Label><span {...{...props}}>Name</span></Label>',
    '<span className="text-14" style={{fontWeight:600}}>Value</span>',
    '<span className="text-[length:var(--text-14)]" style={{fontWeight:600}}>Value</span>',
    'const props = {className:"text-14", style:{fontWeight:600}}; <span {...props}>Value</span>',
    'const caption = {className:"text-14"}; const weight = {fontWeight:600}; const props = {...caption, style:{...weight}}; <span {...props}>Value</span>',
    'const props = {className:"type-body", style:{fontWeight:600}}; <span {...props}>Value</span>',
    'const style = active ? {fontWeight:600} : {}; const props = {style}; <span className="text-14" {...props}>Value</span>',
    'const fontWeight=400; <Label style={{fontWeight}}>Name</Label>',
    '<Label className="font-light">Name</Label>',
    '<Label className="font-[450]">Name</Label>',
    '<Label className="text-[length:var(--text-13)]">Name</Label>',
    '<span className="type-body font-light">Body</span>',
    '<span className="type-body font-[450]">Body</span>',
    '<span className="type-body text-[length:var(--text-13)]">Body</span>',
    '<Label><span className="type-secondary">Name</span><Input /></Label>',
    '<Label asChild><span style={{fontWeight:400}}>Name</span></Label>',
    'import {Label as Caption} from "@/components/ui/label"; <Caption className="type-secondary">Name</Caption>',
    'import {Label as Caption} from "@/components/ui/label"; <Caption style={{fontSize:"var(--text-13)"}}>Name</Caption>',
    'import * as Labels from "@/components/ui/label"; <Labels.Label className="text-13">Name</Labels.Label>',
    '<span className="type-body text-13">Body</span>',
    '<span className={cn(TEXT_STYLES.body, "text-13")}>Body</span>',
    '<span className="type-body" style={{fontSize:"var(--text-13)"}}>Body</span>',
    '<span className="type-body" style={{fontWeight:500}}>Body</span>',
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
    expect(typographyViolations("components/custom-form.tsx", '<Label>Name<FieldHelp>Shared explanatory text</FieldHelp><NativeSelect /></Label>')).toEqual([]);
    expect(typographyViolations("components/custom-form.tsx", '<span className={title ? "type-card-title" : "text-14"} />')).toEqual([]);
    expect(typographyViolations("components/custom-form.tsx", '<span className="type-body text-[#fff] [&_code]:text-14" />')).toEqual([]);
    expect(typographyViolations("components/custom-form.tsx", 'const layout = {className:"flex-1 text-foreground", style:{display:"flex", color:"var(--foreground)"}}; const props = {...layout}; <Label {...props}><span {...{...props}}>Name</span><Input /></Label>')).toEqual([]);
    expect(typographyViolations("components/custom-form.tsx", 'const props = {className:"type-secondary"}; <Label {...props} className="flex-1 text-foreground">Name</Label>')).toEqual([]);
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
    expect(typographyViolations("components/probe.css", '.caption {@apply type-field-label;font-size:var(--text-13)}')).not.toEqual([]);
    expect(typographyViolations("components/probe.css", '.caption {@apply type-body text-13}')).not.toEqual([]);
    expect(typographyViolations("components/probe.css", '.caption {@apply type-body;@apply text-13}')).not.toEqual([]);
    expect(typographyViolations("other.css", ":root { --text-12: 1rem; }")).toHaveLength(2);
    expect(typographyViolations("other.css", ".label {font: 500 11px sans-serif; font-size: 0.75rem;}")).toHaveLength(3);
    expect(typographyViolations("app.css", "html { --text-12: 0.75rem; }")).toHaveLength(1);
    expect(typographyViolations("other.css", ".label {font-size: var(--text-11);}")).toHaveLength(1);
    expect(typographyViolations("other.css", ".label {@apply text-sm;}")).toHaveLength(1);
  });
  it("limits the compact-count size exception to its shared role", () => {
    expect(typographyViolations("typography.css", "@utility type-count-badge {font-size:var(--text-10);font-weight:600;line-height:var(--line-count)}")).toEqual([]);
    expect(typographyViolations("other.css", ".counter {font-size:var(--text-10)}")).not.toEqual([]);
    expect(typographyViolations("typography.css", "@utility type-body {font-size:var(--text-10)}")).not.toEqual([]);
    expect(typographyViolations("typography.css", "@utility type-count-badge {font-size:var(--text-9)}")).not.toEqual([]);
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
