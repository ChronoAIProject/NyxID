#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { gitRepository } from "../../shared/i18n/catalog-core.mjs";
import {
  LANGUAGES, FAMILIES, analyzePortions, chooseCatalogViews, collectPortions, consolidateDuplicates, emptyLanguages, englishOverrides,
  isPlacementSource, layoutReference, mergeDeltas, placeCatalogs, refuseCatalogIssues, rewriteLegacyCatalogImports, writeJson, writePortions,
} from "./lib/i18n-catalogs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const locales = path.join(root, "src/i18n/locales");
const NAMES = ["app", ...FAMILIES];
const AREA_FILE = /^src\/i18n\/locales\/app\/([^/]+)\/([^/]+)\.json$/;

function snapshot(repo, ref, { sources: withSources }) {
  const files = repo.snapshotFiles(ref, "src", (file) =>
    /^src\/i18n\/locales\/.*\.json$/.test(file) || withSources && isPlacementSource(file));
  const read = (file) => files.has(file) ? JSON.parse(files.get(file)) : {};
  const portions = collectPortions(files, AREA_FILE);
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`${ref}: ${analysis.issues.join("\n")}`);
  // Both layouts are reported; chooseCatalogViews decides which one each role uses for `app`.
  const areas = Object.keys(portions).length ? analysis.merged : null;
  const monolithLanguages = LANGUAGES.filter((language) => files.has(`src/i18n/locales/${language}.json`));
  const monolith = monolithLanguages.length ? Object.fromEntries(LANGUAGES.map((language) => [language, read(`src/i18n/locales/${language}.json`)])) : null;
  const catalogs = Object.fromEntries(FAMILIES.map((family) => [family, Object.fromEntries(LANGUAGES.map((language) => [language, read(`src/i18n/locales/${family}-${language}.json`)]))]));
  const familyPresent = Object.fromEntries(FAMILIES.map((family) => [family, LANGUAGES.every((language) => files.has(`src/i18n/locales/${family}-${language}.json`))]));
  const sources = [...files].filter(([file]) => isPlacementSource(file));
  return { areas, monolith, monolithLanguages, catalogs, families: catalogs, familyPresent, portions, sources };
}

try {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "--imports-only") {
    rewriteLegacyCatalogImports(root);
    process.exit();
  }
  const repo = gitRepository(root);
  const refs = repo.resolveAdoptRefs(args, "usage: npm run adopt:i18n -- --branch <ref> --base <ref|empty> --onto <ref> | --imports-only",
    { monoliths: LANGUAGES.map((language) => `src/i18n/locales/${language}.json`) });
  const emptyBase = refs.base === "empty";
  // Placement reads branch sources, plus onto sources unless adopting from an empty base.
  const base = snapshot(repo, refs.base, { sources: false });
  const branch = snapshot(repo, refs.branch, { sources: true });
  const onto = snapshot(repo, refs.onto, { sources: !emptyBase });
  // Each branch layout is diffed against that layout at the newer of --base and merge-base(branch, onto).
  const { commitLocal, mergeBaseRef, order } = repo.adoptionReferences(refs, { branchHasAreas: Boolean(branch.areas) });
  const mergeBase = mergeBaseRef ? snapshot(repo, mergeBaseRef, { sources: false }) : null;
  const app = chooseCatalogViews({ base, branch, onto }, refs, {
    languages: LANGUAGES, adopt: "npm run adopt:i18n --", mergeBase, order, commitLocal,
    legacyFork: () => repo.legacyForks(refs.branch, refs.onto, LANGUAGES.map((language) => `src/i18n/locales/${language}.json`)),
    baseLessAreaMerges: () => repo.baseLessAreaMerges(refs.branch, refs.onto, AREA_FILE),
  });
  for (const role of ["base", "branch", "onto"]) ({ base, branch, onto })[role].catalogs.app = app[role];
  const baseCatalogs = { app: app.base, ...Object.fromEntries(FAMILIES.map((family) => {
    if (emptyBase) return [family, emptyLanguages()];
    const reference = layoutReference(family, { base, mergeBase, order });
    if (!reference && branch.familyPresent[family]) {
      throw new Error(`No files written. Neither ${refs.base} (--base) nor the merge-base of ${refs.branch} and ${refs.onto} has the ${family} catalogs, so the branch's edits to them cannot be told apart from main's; pass a --base or --onto that shares that history.`);
    }
    return [family, reference ?? emptyLanguages()];
  })) };
  // A mixed branch's own edits to its inherited split areas take precedence per key (applyMixedDelta).
  const followUps = app.areaDelta ? { app: { label: "app split areas", ...app.areaDelta } } : {};
  const { result, conflicts } = mergeDeltas(baseCatalogs, branch.catalogs, onto.catalogs, NAMES, emptyBase, followUps);
  if (conflicts.length) throw new Error(`No files written. Resolve these catalog divergences explicitly:\n${conflicts.join("\n")}`);
  const merged = result.app;
  const sources = emptyBase ? branch.sources : [...onto.sources, ...branch.sources];
  // Keys a mixed branch added or changed only in its split areas keep the branch's area when onto has no owner.
  const branchOwners = analyzePortions(branch.portions).locations;
  const preferred = new Map(app.areaDelta ? LANGUAGES.flatMap((language) => Object.keys({ ...app.areaDelta.base[language], ...app.areaDelta.branch[language] })
    .filter((key) => app.areaDelta.base[language][key] !== app.areaDelta.branch[language][key] && branchOwners.get(key)?.size === 1)
    .map((key) => [key, [...branchOwners.get(key)][0]])) : []);
  const refuse = (issues) => refuseCatalogIssues(issues, { languages: LANGUAGES, label: "app" });
  let portions;
  try {
    portions = consolidateDuplicates(placeCatalogs(merged, sources, emptyBase ? {} : onto.portions, preferred));
  } catch (error) {
    refuse([error.message]);
  }
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) refuse(analysis.issues);
  writePortions(locales, portions);
  for (const family of FAMILIES) for (const language of LANGUAGES) writeJson(path.join(locales, `${family}-${language}.json`), result[family][language]);
  writeJson(path.join(locales, "app/en-overrides.json"), englishOverrides(merged.en));
  for (const file of [...LANGUAGES.map((language) => `${language}.json`), "en-overrides.json"]) {
    const full = path.join(locales, file);
    if (fs.existsSync(full)) fs.rmSync(full);
  }
  console.log(`i18n: adopted ${refs.branch} (base ${refs.base}) onto ${refs.onto}; ${Object.keys(merged.en).length} app keys. Stage the catalog files with git add -A src/i18n/locales.`);
  rewriteLegacyCatalogImports(root);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
