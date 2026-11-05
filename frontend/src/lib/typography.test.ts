import { readFileSync } from "node:fs";
import postcss from "postcss";
import { describe, expect, it } from "vitest";
import { computedTextSize, TEXT_STYLES } from "./typography";

describe("semantic text roles", () => {
  it("assigns numeric size and weight to every role", () => {
    const roles = new Map<string, Record<string, string>>();
    postcss.parse(readFileSync("src/typography.css", "utf8")).walkAtRules("utility", rule => {
      roles.set(rule.params, Object.fromEntries((rule.nodes ?? []).filter(node => node.type === "decl").map(node => [node.prop, node.value])));
    });
    for (const role of Object.values(TEXT_STYLES)) expect(roles.get(role)).toMatchObject({
      "font-size": expect.stringMatching(/^var\(--text-\d+\)$/),
      "font-weight": expect.stringMatching(/^[4567]00$/),
      "line-height": expect.stringMatching(/^var\(--line-[\w-]+\)$/),
    });
    expect(roles.get(TEXT_STYLES.fieldLabel)).toEqual({ "font-size": "var(--text-14)", "font-weight": "500", "line-height": "var(--line-body)" });
    expect(roles.get(TEXT_STYLES.dialogTitle)).toEqual(roles.get(TEXT_STYLES.cardTitle));
    expect(roles.get(TEXT_STYLES.input)).toEqual(roles.get(TEXT_STYLES.menuItem));
  });
  it("resolves canvas text from the token and the live root scale", () => {
    const root = document.documentElement;
    root.style.setProperty("--text-14", "0.875rem");
    root.style.fontSize = "13.6px";
    expect(computedTextSize(14)).toBeCloseTo(11.9);
    root.style.fontSize = "27.2px";
    expect(computedTextSize(14)).toBeCloseTo(23.8);
    root.removeAttribute("style");
  });
});
