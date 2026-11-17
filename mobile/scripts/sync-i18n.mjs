#!/usr/bin/env node
import path from "node:path";
import { identicalDuplicateWarnings } from "../../shared/i18n/catalog-core.mjs";
import {
  ADOPT_HINT, analyzePortions, consolidateDuplicates, legacyFiles, loadPortions, root, writeCatalogs,
} from "./lib/i18n-catalogs.mjs";

try {
  const legacy = legacyFiles();
  if (legacy.length) throw new Error(`legacy monolith ${legacy.map((file) => path.relative(root, file)).join(", ")} is present; ${ADOPT_HINT}`);
  // All validation precedes any writes.
  const current = loadPortions();
  const warnings = identicalDuplicateWarnings(analyzePortions(current), "moved to shared");
  const next = consolidateDuplicates(current);
  const analysis = analyzePortions(next);
  if (analysis.issues.length) throw new Error(analysis.issues.join("\n"));
  for (const warning of warnings) console.warn(warning);
  writeCatalogs(next);
  console.log(`i18n: canonicalized mobile catalogs, consolidated duplicates and regenerated barrels; ${Object.keys(analysis.merged["zh-CN"]).length} keys.`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
