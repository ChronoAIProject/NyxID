#!/usr/bin/env node
import fs from "node:fs";
import { chooseCatalogViews, gitRepository, refuseCatalogIssues } from "../../shared/i18n/catalog-core.mjs";
import {
  LANGUAGES, analyzePortions, collectPortions, consolidateDuplicates, isPlacementSource,
  legacyFiles, mergeDeltas, placeCatalogs, rewriteLegacyCatalogImports, root, writeCatalogs,
} from "./lib/i18n-catalogs.mjs";

const AREA_FILE = /^src\/i18n\/locales\/([^/]+)\/([^/]+)\.json$/;

function snapshot(repo, ref, { sources: withSources }) {
  const files = repo.snapshotFiles(ref, "src", (file) =>
    /^src\/i18n\/locales\/.*\.json$/.test(file) || withSources && isPlacementSource(file));
  const portions = collectPortions(files, AREA_FILE);
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`${ref}: ${analysis.issues.join("\n")}`);
  // Both layouts are reported; chooseCatalogViews decides which one each role uses.
  const legacy = (language) => files.has(`src/i18n/locales/${language}.json`) ? JSON.parse(files.get(`src/i18n/locales/${language}.json`)) : {};
  const areas = Object.keys(portions).length ? analysis.merged : null;
  const monolithLanguages = LANGUAGES.filter((language) => files.has(`src/i18n/locales/${language}.json`));
  const monolith = monolithLanguages.length ? Object.fromEntries(LANGUAGES.map((language) => [language, legacy(language)])) : null;
  const sources = [...files].filter(([file]) => isPlacementSource(file));
  return { areas, monolith, monolithLanguages, catalogs: {}, portions, sources };
}

try {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "--imports-only") {
    rewriteLegacyCatalogImports();
    process.exit();
  }
  const repo = gitRepository(root);
  const refs = repo.resolveAdoptRefs(args, "usage: pnpm adopt:i18n --branch <ref> --base <ref|empty> --onto <ref> | --imports-only",
    { monoliths: LANGUAGES.map((language) => `src/i18n/locales/${language}.json`) });
  const emptyBase = refs.base === "empty";
  // Placement reads branch sources, plus onto sources unless adopting from an empty base.
  const base = snapshot(repo, refs.base, { sources: false });
  const branch = snapshot(repo, refs.branch, { sources: true });
  const onto = snapshot(repo, refs.onto, { sources: !emptyBase });
  // Each branch layout is diffed against that layout at the newer of --base and merge-base(branch, onto).
  const { commitLocal, mergeBaseRef, order } = repo.adoptionReferences(refs, { branchHasAreas: Boolean(branch.areas) });
  const mergeBase = mergeBaseRef ? snapshot(repo, mergeBaseRef, { sources: false }) : null;
  const chosen = chooseCatalogViews({ base, branch, onto }, refs, {
    languages: LANGUAGES, adopt: "pnpm adopt:i18n", mergeBase, order, commitLocal,
    legacyFork: () => repo.legacyForks(refs.branch, refs.onto, LANGUAGES.map((language) => `src/i18n/locales/${language}.json`)),
    baseLessAreaMerges: () => repo.baseLessAreaMerges(refs.branch, refs.onto, AREA_FILE),
  });
  for (const role of ["base", "branch", "onto"]) ({ base, branch, onto })[role].catalogs.mobile = chosen[role];
  // Then a mixed branch's own edits to the split areas it inherited (mirrored edits are no-ops).
  const followUps = chosen.areaDelta ? { mobile: { label: "mobile split areas", ...chosen.areaDelta } } : {};
  const { result, conflicts } = mergeDeltas(base.catalogs, branch.catalogs, onto.catalogs, ["mobile"], emptyBase, followUps);
  if (conflicts.length) throw new Error(`No files written. Resolve these catalog divergences explicitly:\n${conflicts.join("\n")}`);
  const sources = emptyBase ? branch.sources : [...onto.sources, ...branch.sources];
  // Keys a mixed branch added or changed only in its split areas keep the branch's area when onto has no owner.
  const branchOwners = analyzePortions(branch.portions).locations;
  const preferred = new Map(chosen.areaDelta ? LANGUAGES.flatMap((language) => Object.keys({ ...chosen.areaDelta.base[language], ...chosen.areaDelta.branch[language] })
    .filter((key) => chosen.areaDelta.base[language][key] !== chosen.areaDelta.branch[language][key] && branchOwners.get(key)?.size === 1)
    .map((key) => [key, [...branchOwners.get(key)][0]])) : []);
  const refuse = (issues) => refuseCatalogIssues(issues, { languages: LANGUAGES, label: "mobile" });
  let portions;
  try {
    portions = consolidateDuplicates(placeCatalogs(result.mobile, sources, emptyBase ? {} : onto.portions, preferred));
  } catch (error) {
    refuse([error.message]);
  }
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) refuse(analysis.issues);
  writeCatalogs(portions);
  for (const file of legacyFiles()) fs.rmSync(file);
  console.log(`i18n: adopted ${refs.branch} (base ${refs.base}) onto ${refs.onto}; ${Object.keys(result.mobile["zh-CN"]).length} mobile keys. Stage the catalog files with git add -A src/i18n/locales.`);
  rewriteLegacyCatalogImports();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
