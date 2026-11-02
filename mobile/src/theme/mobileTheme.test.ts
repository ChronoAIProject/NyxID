import assert from "node:assert/strict";
import test from "node:test";
import { chatBubbleColors, lightBubblePalettes, normalizeLightBubblePalette } from "./chatAppearance";
import {
  darkColors,
  darkSheetColors,
  lightColors,
  lightSheetColors,
} from "./mobileTheme";
type Rgb = [number, number, number];
function rgb(hex: string): Rgb {
  return [1, 3, 5].map(
    (start) => parseInt(hex.slice(start, start + 2), 16) / 255,
  ) as Rgb;
}
function luminance(values: Rgb) {
  return values
    .map((value) =>
      value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
    )
    .reduce(
      (sum, value, index) => sum + value * [0.2126, 0.7152, 0.0722][index]!,
      0,
    );
}
function ratio(a: Rgb, b: Rgb) {
  const values = [luminance(a), luminance(b)].sort((x, y) => x - y);
  return (values[1]! + 0.05) / (values[0]! + 0.05);
}
function composite(rgba: string, base: string): Rgb {
  const parts = rgba.match(/[\d.]+/g)!.map(Number);
  return rgb(base).map(
    (value, index) =>
      value * (1 - parts[3]!) + (parts[index]! / 255) * parts[3]!,
  ) as Rgb;
}
test("every chat palette keeps AA text contrast and an invalid saved choice falls back to gray", () => {
  for (const palette of [...Object.values(lightBubblePalettes), chatBubbleColors("dark", "graphite")])
    assert.ok(ratio(rgb(palette.foreground), rgb(palette.background)) >= 4.5);
  assert.equal(normalizeLightBubblePalette("removed-palette"), "gray");
  assert.equal(normalizeLightBubblePalette(null), "gray");
  assert.equal(chatBubbleColors("light", "graphite"), lightBubblePalettes.graphite);
  assert.deepEqual(chatBubbleColors("dark", "gray"), chatBubbleColors("dark", "graphite"));
});
test("native body and secondary text meet AA on every shared surface in both themes", () => {
  for (const colors of [darkColors, lightColors])
    for (const background of [
      colors.bg,
      colors.card,
      colors.cardSoft,
      colors.sidebarBase,
    ])
      for (const foreground of [
        colors.textPrimary,
        colors.textSecondary,
        colors.textMuted,
        colors.textTertiary,
        colors.primaryOnTint,
      ]) {
        assert.ok(
          ratio(rgb(foreground), rgb(background)) >= 4.5,
          `${foreground} on ${background}`,
        );
      }
});
test("native status labels meet AA on their tinted surfaces", () => {
  for (const colors of [darkColors, lightColors])
    for (const base of [
      colors.bg,
      colors.card,
      colors.cardSoft,
      colors.sidebarBase,
    ])
      for (const tone of [
        colors.successTone,
        colors.warningTone,
        colors.infoTone,
        colors.dangerTone,
        colors.primaryTone,
        colors.riskHigh,
        colors.riskMedium,
        colors.riskLow,
      ]) {
        assert.ok(
          ratio(rgb(tone.text), composite(tone.bg, base)) >= 4.5,
          `${tone.text} on ${tone.bg} over ${base}`,
        );
      }
});
test("drawer surfaces keep text at AA and stand apart from the dimmed chat", () => {
  for (const [base, sheet] of [
    [darkColors, darkSheetColors],
    [lightColors, lightSheetColors],
  ] as const) {
    for (const background of [sheet.bg, sheet.card, sheet.cardSoft])
      for (const foreground of [
        sheet.textPrimary,
        sheet.textSecondary,
        sheet.textMuted,
        sheet.textTertiary,
        sheet.primaryOnTint,
      ])
        assert.ok(
          ratio(rgb(foreground), rgb(background)) >= 4.5,
          `${foreground} on drawer ${background}`,
        );
    const dimmedChat = composite(base.sheetScrim, base.bg);
    // The handle is the drawer's visible affordance against the dimmed chat and its own surface.
    assert.ok(ratio(rgb(sheet.textSecondary), dimmedChat) >= 3);
    assert.ok(ratio(rgb(sheet.textSecondary), rgb(sheet.bg)) >= 3);
    assert.ok(luminance(rgb(sheet.bg)) !== luminance(dimmedChat));
  }
  // Dark mode cannot use the chat background: it measured 1.02:1 against the dimmed chat.
  const dimmedDark = composite(darkColors.sheetScrim, darkColors.bg);
  assert.ok(ratio(rgb(darkSheetColors.bg), dimmedDark) >= 1.1);
  assert.ok(
    ratio(composite(darkColors.sheetEdge, darkSheetColors.bg), dimmedDark) >=
      1.75,
  );
});
test("composer text and placeholder meet AA on the filled composer", () => {
  for (const colors of [darkColors, lightColors])
    for (const foreground of [colors.textPrimary, colors.textMuted])
      assert.ok(
        ratio(rgb(foreground), rgb(colors.composerBg)) >= 4.5,
        `${foreground} on ${colors.composerBg}`,
      );
});

test("selected, interactive, error and disabled labels keep AA on zinc surfaces", () => {
  for (const colors of [
    darkColors,
    lightColors,
    darkSheetColors,
    lightSheetColors,
  ]) {
    for (const base of [
      colors.bg,
      colors.card,
      colors.cardSoft,
      colors.sidebarBase,
    ]) {
      for (const surface of [
        composite(colors.navActive, base),
        composite(colors.ghostBg, base),
      ]) {
        for (const text of [
          colors.textPrimary,
          colors.textSecondary,
          colors.ghostText,
        ])
          assert.ok(
            ratio(rgb(text), surface) >= 4.5,
            `${text} on selected/interactive ${base}`,
          );
      }
      assert.ok(
        ratio(rgb(colors.primary), rgb(base)) >= 4.5,
        `accent text on ${base}`,
      );
      assert.ok(
        ratio(rgb(colors.danger), composite(colors.dangerSoftBg, base)) >= 4.5,
      );
      // Inactive controls are exempt in WCAG; native labels still remain readable.
      assert.ok(ratio(rgb(colors.textTertiary), rgb(colors.cardSoft)) >= 4.5);
      assert.ok(
        ratio(rgb(colors.controlBorder), rgb(base)) >= 3,
        `control edge on ${base}`,
      );
    }
    assert.ok(
      ratio(rgb(colors.bg), rgb(colors.textPrimary)) >= 3,
      "submit glyph",
    );
    assert.ok(
      ratio(rgb(colors.textPrimary), rgb(colors.composerBg)) >= 3,
      "submit circle",
    );
  }
});

test("white CTA labels meet AA throughout normal and pressed native gradients", () => {
  for (const colors of [darkColors, lightColors]) {
    for (const [start, end] of [
      [colors.gradientStart, colors.gradientEnd],
      [colors.gradientStartPressed, colors.gradientEndPressed],
    ]) {
      for (let step = 0; step <= 20; step++) {
        const fill = rgb(start!).map(
          (value, index) => value + ((rgb(end!)[index]! - value) * step) / 20,
        ) as Rgb;
        assert.ok(ratio(rgb(colors.onPrimary), fill) >= 4.5);
      }
    }
  }
});

test("sidebar is the darkest dark layer and gray beneath a white light content slab", () => {
  assert.ok(
    luminance(rgb(darkColors.sidebarBase)) < luminance(rgb(darkColors.bg)),
  );
  assert.equal(lightColors.bg, "#FFFFFF");
  assert.equal(lightColors.sidebarBase, "#F4F4F5");
  for (const colors of [darkColors, lightColors]) {
    for (const foreground of [
      colors.textPrimary,
      colors.textSecondary,
      colors.textMuted,
      colors.primary,
    ])
      assert.ok(ratio(rgb(foreground), rgb(colors.sidebarBase)) >= 4.5);
    assert.ok(
      ratio(
        rgb(colors.textPrimary),
        composite(colors.navActive, colors.sidebarBase),
      ) >= 4.5,
    );
    assert.ok(ratio(rgb(colors.controlBorder), rgb(colors.sidebarBase)) >= 3);
  }
});
