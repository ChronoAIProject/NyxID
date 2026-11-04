import { describe, expect, it } from "vitest";
import { computedTextSize, TEXT_STYLES } from "./typography";

describe("semantic text roles", () => {
  it("assigns numeric size and weight to every role", () => {
    for (const classes of Object.values(TEXT_STYLES)) expect(classes).toMatch(/^text-\d+ font-(normal|medium|semibold|bold)$/);
    expect(TEXT_STYLES.fieldLabel).toBe("text-14 font-medium");
    expect(TEXT_STYLES.dialogTitle).toBe(TEXT_STYLES.cardTitle);
    expect(TEXT_STYLES.input).toBe(TEXT_STYLES.menuItem);
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
