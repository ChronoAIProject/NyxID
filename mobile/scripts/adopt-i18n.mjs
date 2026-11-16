#!/usr/bin/env node
import fs from "node:fs";
import { gitRepository } from "../../shared/i18n/catalog-core.mjs";
import {
  LANGUAGES, analyzePortions, collectPortions, consolidateDuplicates, isPlacementSource,
  legacyFiles, mergeDeltas, placeCatalogs, root, writeCatalogs,
} from "./lib/i18n-catalogs.mjs";

function snapshot(repo, ref, { sources: withSources }) {
  const files = repo.snapshotFiles(ref, "src", (file) =>
    /^src\/i18n\/locales\/.*\.json$/.test(file) || withSources && isPlacementSource(file));
  const portions = collectPortions(files, /^src\/i18n\/locales\/([^/]+)\/([^/]+)\.json$/);
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`${ref}: ${analysis.issues.join("\n")}`);
  const legacy = (language) => files.has(`src/i18n/locales/${language}.json`) ? JSON.parse(files.get(`src/i18n/locales/${language}.json`)) : {};
  const mobile = Object.keys(portions).length ? analysis.merged : Object.fromEntries(LANGUAGES.map((language) => [language, legacy(language)]));
  const sources = [...files].filter(([file]) => isPlacementSource(file));
  return { catalogs: { mobile }, portions, sources };
}

try {
  const repo = gitRepository(root);
  const refs = repo.resolveAdoptRefs(process.argv.slice(2), "usage: pnpm adopt:i18n --branch <ref> --base <ref|empty> --onto <ref>");
  const emptyBase = refs.base === "empty";
  // Placement reads branch sources, plus onto sources unless adopting from an empty base.
  const base = snapshot(repo, refs.base, { sources: false });
  const branch = snapshot(repo, refs.branch, { sources: true });
  const onto = snapshot(repo, refs.onto, { sources: !emptyBase });
  const { result, conflicts } = mergeDeltas(base.catalogs, branch.catalogs, onto.catalogs, ["mobile"], emptyBase);
  if (conflicts.length) throw new Error(`No files written. Resolve these catalog divergences explicitly:\n${conflicts.join("\n")}`);
  const sources = emptyBase ? branch.sources : [...onto.sources, ...branch.sources];
  const portions = consolidateDuplicates(placeCatalogs(result.mobile, sources, emptyBase ? {} : onto.portions));
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`No files written. ${analysis.issues.join("\n")}`);
  writeCatalogs(portions);
  for (const file of legacyFiles()) fs.rmSync(file);
  console.log(`i18n: adopted ${refs.branch} (base ${refs.base}) onto ${refs.onto}; ${Object.keys(result.mobile["zh-CN"]).length} mobile keys. Stage the catalog files with git add -A src/i18n/locales.`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
