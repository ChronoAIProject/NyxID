import assert from "node:assert/strict";
import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import { auditCatalogs, catalogWarnings, discoverPortions, legacyImportIssues, root } from "./lib/i18n-catalogs.mjs";
import { areaForFeature } from "./lib/i18n-portions.mjs";

test("mobile catalogs are split by area, sorted, in parity and fully re-exported by the barrels", () => {
  assert.ok(discoverPortions().length > 0, "no mobile catalog areas found");
  assert.deepEqual(auditCatalogs(), []);
  // Identical cross-area duplicates only warn; differing values fail above.
  for (const warning of catalogWarnings()) console.warn(warning);
});

test("every feature directory maps to its own, non-reserved catalog area", () => {
  const features = fs.readdirSync(path.join(root, "src/features"), { withFileTypes: true }).filter((entry) => entry.isDirectory());
  assert.ok(features.length > 0);
  for (const { name } of features) assert.equal(areaForFeature(name), name);
});

test("no mobile source imports a pre-split catalog file", () => {
  assert.deepEqual(legacyImportIssues(), []);
});

test("iOS permission descriptions resolve only for the iOS native project", async () => {
  const require = createRequire(import.meta.url);
  const { getResolvedLocalesAsync } = require("@expo/config-plugins/build/utils/locales");
  const inputs = {
    "zh-Hans": "./locales/zh-Hans.json",
    "zh-Hant": "./locales/zh-Hant.json",
  };
  const { localesMap: ios } = await getResolvedLocalesAsync(root, inputs, "ios");
  const { localesMap: android } = await getResolvedLocalesAsync(root, inputs, "android");

  for (const locale of Object.keys(inputs)) {
    assert.ok(ios[locale].NSCameraUsageDescription);
    assert.ok(ios[locale].NSMicrophoneUsageDescription);
    assert.ok(ios[locale].NSSpeechRecognitionUsageDescription);
    assert.deepEqual(android[locale], {});
  }
});
