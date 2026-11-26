#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  analyzePortions, catalogFiles, consolidateDuplicates, englishOverrides,
  loadPortions, readJson, writeJson, writePortions,
} from "./lib/i18n-catalogs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const locales = path.join(root, "src/i18n/locales");
const overridesOnly = process.argv.includes("--overrides-only");
try {
  const portions = loadPortions(locales);
  const next = overridesOnly ? portions : consolidateDuplicates(portions);
  const analysis = analyzePortions(next);
  if (analysis.issues.length) throw new Error(analysis.issues.join("\n"));
  if (!overridesOnly) {
    // All validation precedes any writes.
    const catalogs = catalogFiles(locales).map((file) => [file, readJson(file)]);
    for (const [file, catalog] of catalogs) writeJson(file, catalog);
    writePortions(locales, next);
  }
  writeJson(path.join(locales, "app/en-overrides.json"), englishOverrides(analysis.merged.en));
  console.log(`i18n: ${overridesOnly ? "regenerated overrides" : "canonicalized catalogs and consolidated duplicates"}; ${Object.keys(analysis.merged.en).length} app keys.`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
