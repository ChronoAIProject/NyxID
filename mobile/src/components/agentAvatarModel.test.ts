import assert from "node:assert/strict";
import test from "node:test";
import {
  AGENT_AVATAR_TINTS,
  agentInitials,
  agentTintIndex,
} from "./agentAvatarModel";
import {
  darkColors,
  lightColors,
  darkSheetColors,
  lightSheetColors,
} from "../theme/mobileTheme";

// Fixed outputs executed from origin/main's nyxbot-avatar.ts, including UTF-16 input.
test("agent initials and stable tint match web vectors", () => {
  for (const [name, initials] of [
    ["", "?"],
    ["release-notes", "RN"],
    ["Knowledge editor", "KE"],
    ["R", "R"],
    ["Équipe 中文", "QU"],
    ["中文", "中文"],
    ["---", "--"],
    ["42 tester", "4T"],
    ["🤖", "🤖"],
  ])
    assert.equal(agentInitials(name!), initials);
  for (const [id, tint] of [
    ["", 1],
    ["a", 4],
    ["nyxbot", 3],
    ["11111111-1111-4111-8111-111111111111", 1],
    ["22222222-2222-4222-8222-222222222222", 5],
    ["Knowledge editor", 2],
    ["中文", 5],
    ["🤖", 3],
  ] as const)
    assert.equal(agentTintIndex(id), tint);
});

type Rgb = number[];
function rgb(hex: string): Rgb {
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
}
function over(color: string, base: Rgb): Rgb {
  if (color === "transparent") return base;
  if (color.startsWith("#")) return rgb(color);
  const parts = color.match(/[\d.]+/g)!.map(Number);
  return base.map(
    (v, i) => v * (1 - parts[3]!) + (parts[i]! / 255) * parts[3]!,
  );
}
function luminance(rgb: Rgb) {
  return rgb.reduce(
    (sum, v, i) =>
      sum +
      (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4) *
        [0.2126, 0.7152, 0.0722][i]!,
    0,
  );
}
function contrast(a: Rgb, b: Rgb) {
  const values = [luminance(a), luminance(b)].sort((a, b) => a - b);
  return (values[1]! + 0.05) / (values[0]! + 0.05);
}

test("web avatar contrast exceptions stay explicit across chat and drawer cards", (t) => {
  const textRisks: string[] = [];
  const minima = new Map<string, { text: number; edge: number }>();
  const edgeRisks: string[] = [];
  for (const [mode, palettes] of [
    ["dark", [darkColors, darkSheetColors]],
    ["light", [lightColors, lightSheetColors]],
  ] as const) {
    AGENT_AVATAR_TINTS.forEach((tint, index) => {
      for (const colors of palettes)
        for (const surface of [colors.card, colors.cardSoft]) {
          const fill = over(tint[mode].fill, rgb(surface));
          const text = contrast(rgb(tint[mode].text), fill);
          const edge = Math.max(
            contrast(fill, rgb(surface)),
            contrast(over(tint[mode].border, fill), rgb(surface)),
          );
          const key = `${mode}:${index}`;
          const previous = minima.get(key);
          minima.set(key, {
            text: Math.min(previous?.text ?? Infinity, text),
            edge: Math.min(previous?.edge ?? Infinity, edge),
          });
          if (text < 4.5) textRisks.push(`${mode}:${index}`);
          if (edge < 3) edgeRisks.push(`${mode}:${index}`);
        }
    });
  }
  t.diagnostic(
    JSON.stringify(
      Object.fromEntries(
        [...minima].map(([k, v]) => [
          k,
          { text: +v.text.toFixed(2), edge: +v.edge.toFixed(2) },
        ]),
      ),
    ),
  );
  // Exact web colors are decorative identity; full names/status remain AA text.
  assert.deepEqual(
    [...new Set(textRisks)],
    ["dark:0", "dark:1", "light:0", "light:2"],
  );
  assert.deepEqual(
    [...new Set(edgeRisks)],
    ["dark:0", "dark:1", "dark:2", "dark:3", "dark:4", "dark:5"],
  );
});
