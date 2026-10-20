import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_COLORS,
  EDITABLE_COLORS,
  buildCustomColorCss,
  contrastRatio,
  sanitizeCustomColors,
} from "./theme-colors";

describe("sanitizeCustomColors", () => {
  it("keeps known keys with #RRGGBB values, uppercased", () => {
    expect(sanitizeCustomColors({ light: { border: "#c4c4ca" }, dark: {} })).toEqual({
      light: { border: "#C4C4CA" },
      dark: {},
    });
  });

  it("drops anything that could break out of the stylesheet", () => {
    const tampered = {
      light: {
        border: "#fff; } body { display: none",
        input: "red",
        foreground: "#12345",
        "not-a-token": "#123456",
      },
      dark: "nope",
    };
    expect(sanitizeCustomColors(tampered)).toEqual({ light: {}, dark: {} });
    expect(sanitizeCustomColors(null)).toEqual({ light: {}, dark: {} });
  });
});

describe("buildCustomColorCss", () => {
  it("emits nothing without overrides", () => {
    expect(buildCustomColorCss({ light: {}, dark: {} })).toBe("");
  });

  it("scopes each theme and fans a color out to all of its tokens", () => {
    const css = buildCustomColorCss({ light: { card: "#FAFAFA" }, dark: {} });
    expect(css).toContain("html.theme-light.theme-light {");
    expect(css).not.toContain("theme-dark");
    for (const v of ["card", "popover", "surface"]) {
      expect(css).toContain(`--color-${v}: #FAFAFA;`);
    }
  });
});

describe("contrastRatio", () => {
  it("matches the WCAG reference values", () => {
    expect(contrastRatio("#000000", "#FFFFFF")).toBeCloseTo(21, 5);
    expect(contrastRatio("#FFFFFF", "#FFFFFF")).toBeCloseTo(1, 5);
  });
});

/** Pull `--color-x: value;` declarations out of one top-level CSS block. */
function block(css: string, selector: string): Map<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} block`).toBeGreaterThanOrEqual(0);
  const body = css.slice(start, css.indexOf("\n}", start));
  return new Map([...body.matchAll(/--color-([\w-]+):\s*([^;]+);/g)].map((m) => [m[1]!, m[2]!.trim()]));
}

describe("DEFAULT_COLORS", () => {
  it("match what app.css ships for each theme", () => {
    const css = readFileSync(resolve(__dirname, "../app.css"), "utf8");
    const base = block(css, "@theme");
    for (const mode of ["dark", "light"] as const) {
      const themed = block(css, `html.theme-${mode}`);
      for (const { key, vars } of EDITABLE_COLORS) {
        for (const v of vars) {
          const shipped = themed.get(v) ?? base.get(v);
          expect(shipped?.toUpperCase(), `${mode} --color-${v}`).toBe(DEFAULT_COLORS[mode][key]);
        }
      }
    }
  });
});
