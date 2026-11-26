import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import cn from "../../i18n/locales/zh-CN";
import tw from "../../i18n/locales/zh-TW";
import { FLOATING_TRAILING_INSET } from "./workspaceLayout";
import {
  estimateBodyTextWidth,
  fittingPlaceholder,
  SEARCH_FIELD,
  searchFieldTextWidth,
} from "./textFit";

/** Search and Recents: page padding 0, toolbar padding 16 on each side. */
const TOOLBAR_PADDING = 16;
/**
 * The narrowest windows the field appears in: 320pt (first-generation iPhone
 * SE on iOS 15, iPad Slide Over and narrow Split View) and the folded Duo in
 * rail mode (466pt minus the rail inset, the same in full screen).
 */
const NARROWEST_FIELDS = {
  "320pt window": 320 - 2 * TOOLBAR_PADDING,
  "folded Duo rail": 466 - FLOATING_TRAILING_INSET - 2 * TOOLBAR_PADDING,
};
/**
 * React Native's font scale at the largest Dynamic Type (accessibility
 * extra-extra-extra-large), from RCTAccessibilityManager's multiplier table.
 */
const LARGEST_FONT_SCALE = 3.571;

const catalogs = {
  en: (key: string) => key,
  "zh-CN": (key: string) => (cn as Record<string, string>)[key],
  "zh-TW": (key: string) => (tw as Record<string, string>)[key],
};

/** Every placeholder set the browse screens pass, as English catalog keys. */
function placeholderSets() {
  const source = readFileSync(
    fileURLToPath(new URL("./WorkspaceBrowseScreen.tsx", import.meta.url)),
    "utf8",
  );
  return [...source.matchAll(/placeholders=\{\[([\s\S]*?)\]\}/g)].map(([, list]) =>
    [...list!.matchAll(/translateApp\("([^"]+)"\)/g)].map(([, key]) => key!),
  );
}

test("search placeholders fit the narrowest field in every language: the full phrase at default size, the short one at the largest Dynamic Type", () => {
  const sets = placeholderSets();
  assert.ok(sets.length >= 2, "Search and Recents fields found");
  for (const keys of sets) {
    assert.ok(keys.length >= 2, `${keys}: a descriptive phrase and a short fallback`);
    for (const [language, translate] of Object.entries(catalogs)) {
      const [full, ...rest] = keys.map((key) => translate(key));
      const shortest = rest[rest.length - 1]!;
      assert.ok(full && shortest, `${language} translations for ${keys}`);
      for (const [where, field] of Object.entries(NARROWEST_FIELDS)) {
        const room = searchFieldTextWidth(field);
        const fullWidth = estimateBodyTextWidth(full!, SEARCH_FIELD.fontSize);
        assert.ok(
          fullWidth <= room,
          `${language} "${full}" ${fullWidth.toFixed(0)}pt fits ${room}pt (${where})`,
        );
        const largest = SEARCH_FIELD.fontSize * LARGEST_FONT_SCALE;
        const shortWidth = estimateBodyTextWidth(shortest, largest);
        assert.ok(
          shortWidth <= room,
          `${language} "${shortest}" ${shortWidth.toFixed(0)}pt fits ${room}pt at the largest size (${where})`,
        );
        // The field shows exactly these.
        assert.equal(
          fittingPlaceholder([full!, shortest], room, SEARCH_FIELD.fontSize),
          full,
        );
        // At the largest size the field keeps whichever phrase still fits.
        const chosen = fittingPlaceholder([full!, shortest], room, largest);
        assert.ok(estimateBodyTextWidth(chosen, largest) <= room, `${chosen} (${where})`);
      }
    }
  }
});

test("the placeholder falls back to the short phrase until the field is measured or when nothing fits", () => {
  const candidates = ["Search chats, agents, settings", "Search"];
  assert.equal(fittingPlaceholder(candidates, undefined, 15), "Search");
  assert.equal(fittingPlaceholder(candidates, 0, 15), "Search");
  assert.equal(fittingPlaceholder(candidates, 10, 15), "Search", "even if it clips, the shortest");
  assert.equal(fittingPlaceholder(candidates, 400, 15), candidates[0]);
});

test("width estimates use the bundled font's advances and count CJK as full width", () => {
  // Manrope: space 0.2em, "a" 0.555em, then the 5% safety margin.
  assert.equal(estimateBodyTextWidth(" ", 20), 0.2 * 20 * 1.05);
  assert.equal(estimateBodyTextWidth("a", 20), 0.555 * 20 * 1.05);
  assert.equal(estimateBodyTextWidth("搜索、", 10), 3 * 10 * 1.05);
  // The old phrase was what clipped on the device.
  assert.ok(
    estimateBodyTextWidth("Search chats, agents, crews, pages and settings", 15) >
      searchFieldTextWidth(NARROWEST_FIELDS["folded Duo rail"]),
  );
});
