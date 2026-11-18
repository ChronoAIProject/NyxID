import { createRequire } from "node:module";
import path from "node:path";
import * as core from "../../../shared/i18n/catalog-core.mjs";
import { portionForPath } from "./i18n-portions.mjs";

const require = createRequire(import.meta.url);
let typescript;
/** TypeScript, loaded on first use: adopting or syncing catalogs with no sources to parse never needs it. */
const ts = new Proxy({}, { get: (_, name) => (typescript ??= require("typescript"))[name] });

export {
  applyMixedDelta, chooseCatalogViews, layoutReference, legacyImportReplacement, mayImportLegacyCatalog, refuseCatalogIssues, resolveSpecifier, rewriteLegacyImports,
  applyDelta, canonicalText, codeUnitCompare, readJson, sortObject, stripPluralSuffix,
  walkFiles, writeJson,
} from "../../../shared/i18n/catalog-core.mjs";

export const LANGUAGES = ["en", "zh-CN", "zh-TW"];
export const FAMILIES = ["common", "errors", "wizard"];
const tools = core.catalogTools(LANGUAGES, { label: "app" });
export const { emptyLanguages, analyzePortions, consolidateDuplicates, collectPortions, mergeDeltas } = tools;
export const englishOverrides = (english) => Object.fromEntries(
  Object.entries(core.sortObject(english)).filter(([key, value]) => key !== value),
);
export const isTestSource = (file) => /(?:\.(?:test|spec)\.[cm]?[jt]sx?$|\/(?:__tests__|test|tests)\/)/.test(file);
/** Fixture and mock sources; shared with check-i18n's copy audit. */
export const FIXTURE_SOURCE = /fixtures?|mock/;
/**
 * The one predicate for sources whose string literals decide catalog placement.
 * Excludes tests, test harnesses, dev previews, fixtures/mocks, features with their
 * own catalogs, and the catalogs themselves.
 */
export const isPlacementSource = (file) => /\.tsx?$/.test(file) &&
  !isTestSource(file) &&
  file !== "src/test-setup.ts" &&
  !/^src\/(?:test-utils|dev)\//.test(file) &&
  !FIXTURE_SOURCE.test(file) &&
  !/^src\/features\/(?:landing|nyxbot-onboarding|website-a)\//.test(file) &&
  !file.includes("/locales/");

export function catalogFiles(locales) {
  return core.walkFiles(locales).filter((file) => file.endsWith(".json") && !file.endsWith("/en-overrides.json"));
}

export function allCatalogFiles(locales) {
  return core.walkFiles(locales).filter((file) => file.endsWith(".json"));
}

const appDir = (locales) => path.join(locales, "app");
export const discoverPortions = (locales) => core.discoverPortions(appDir(locales));
export const portionFile = (locales, portion, language) => core.portionFile(appDir(locales), portion, language);
export const loadPortions = (locales) => tools.loadPortions(appDir(locales));
export const writePortions = (locales, portions) => tools.writePortions(appDir(locales), portions);

/** Every string and no-substitution template literal in a TypeScript source. */
function stringLiterals(file, source) {
  const literals = [];
  const visit = (node) => {
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) literals.push(node.text);
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true));
  return literals;
}
const scan = { isSource: isPlacementSource, areaForPath: portionForPath, literals: stringLiterals };
export const scanUsage = (sources, keys) => core.scanUsage(sources, keys, scan);
export const placeCatalogs = (merged, sources, existing = {}, preferred = new Map()) => tools.placeCatalogs(merged, sources, existing, scan, preferred);

export const SPECIFIER_ALIASES = [["@/", "src/"], ["/src/", "src/"]];

/** Pre-split catalog files and the module each import must use instead. */
export const LEGACY_CATALOG_IMPORTS = {
  "src/i18n/locales/zh-CN.json": "src/i18n/locales/app/zh-CN",
  "src/i18n/locales/zh-TW.json": "src/i18n/locales/app/zh-TW",
  "src/i18n/locales/en.json": "src/i18n/locales/app/en-full",
  "src/i18n/locales/en-overrides.json": "src/i18n/locales/app/en-overrides.json",
};
export const moduleSpecifierOf = (node, ast) => core.moduleSpecifierOf(ts, node, ast);
export const moduleSpecifiers = core.moduleSpecifierLister(ts);
/** Rewrite legacy catalog imports in frontend/src, printing each rewrite. */
export const rewriteLegacyCatalogImports = (root) => core.rewriteLegacyImportsInTree({
  root, dir: "src", specifiers: moduleSpecifiers, legacy: LEGACY_CATALOG_IMPORTS, aliases: SPECIFIER_ALIASES,
  typecheck: "npx tsc -b",
  typeNote: "rewritten imports now load Record<string, string> catalogs, so indexing one with a literal key can be string | undefined; fix any such npx tsc -b error by hand.",
});

/** Resolve relative, @/ and src/ imports independently of the current machine. */
export function catalogImportIssue(file, specifier) {
  if (isTestSource(file)) return null;
  const resolved = core.resolveSpecifier(file, specifier.split(/[?#]/)[0], SPECIFIER_ALIASES);
  if (!resolved.startsWith("src/i18n/locales/")) return null;
  const isBarrel = /^src\/i18n\/locales\/app\/(?:en-full|zh-CN|zh-TW)\.ts$/.test(file);
  if (/\/en-full(?:\.[cm]?[jt]s)?$/.test(resolved)) return "full English catalog is test-only";
  if (/^src\/i18n\/locales\/app\/[^/]+\/(?:en|zh-CN|zh-TW)\.json$/.test(resolved) && !isBarrel) return "portion JSON imports are restricted to the catalog barrels and tests";
  if (!["src/i18n/index.ts", "src/i18n/instance.ts", "src/i18n/wizard.ts"].includes(file) && !isBarrel) return "catalog imports are restricted to sanctioned i18n modules and tests";
  return null;
}
