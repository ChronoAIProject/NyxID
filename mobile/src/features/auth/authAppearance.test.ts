import assert from "node:assert/strict";
import test from "node:test";
import { darkColors, lightColors } from "../../theme/mobileTheme";
import {
  authInputChrome,
  authLightColors,
  authSurfaceColors,
} from "./authAppearance";

const luminance = (hex: string) =>
  [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
    .reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i]!, 0);
const contrast = (a: string, b: string) =>
  (Math.max(luminance(a), luminance(b)) + 0.05) /
  (Math.min(luminance(a), luminance(b)) + 0.05);

test("auth light fills, placeholders, divider and focus/control edges retain accessible contrast", () => {
  const c = authSurfaceColors(lightColors, "light");
  for (const fill of [c.bg, authLightColors.fill, authLightColors.pressed]) {
    for (const text of [c.textPrimary, c.textSecondary, authLightColors.muted])
      assert.ok(
        contrast(text, fill) >= 4.5,
        `${text} on ${fill}: ${contrast(text, fill)}`,
      );
    for (const edge of [authLightColors.border, authLightColors.focus])
      assert.ok(
        contrast(edge, fill) >= 3,
        `${edge} on ${fill}: ${contrast(edge, fill)}`,
      );
  }
  assert.equal(c.card, authLightColors.fill);
  assert.equal(c.navActive, authLightColors.pressed);
  assert.equal(c.borderSoft, authLightColors.border);
  assert.equal(c.textTertiary, authLightColors.muted);
  assert.notEqual(c.card, c.bg);
});

test("auth palette scopes only light auth and returns dark tokens unchanged", () => {
  const before = JSON.stringify(lightColors);
  assert.equal(authSurfaceColors(darkColors, "dark"), darkColors);
  authSurfaceColors(lightColors, "light");
  assert.equal(JSON.stringify(lightColors), before);
  assert.equal(lightColors.card, "#FFFFFF");
  assert.equal(lightColors.cardSoft, "#FFFFFF");
  assert.equal(authInputChrome("dark", true, false), undefined);
});

test("auth focus never changes geometry or overrides an error edge", () => {
  const idle = authInputChrome("light", false, false)!;
  const focused = authInputChrome("light", true, false)!;
  assert.equal(idle.borderWidth, focused.borderWidth);
  assert.equal(idle.borderRadius, focused.borderRadius);
  assert.equal(focused.borderColor, authLightColors.focus);
  assert.equal(authInputChrome("light", true, true)!.borderColor, undefined);
  assert.equal("shadowOpacity" in focused, false);
});
