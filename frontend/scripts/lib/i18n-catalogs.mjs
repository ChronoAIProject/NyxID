import { createRequire } from "node:module";
import path from "node:path";
import * as core from "../../../shared/i18n/catalog-core.mjs";
import { portionForPath } from "./i18n-portions.mjs";

const require = createRequire(import.meta.url);
let typescript;
/** TypeScript, loaded on first use: adopting or syncing catalogs with no sources to parse never needs it. */
const ts = new Proxy({}, { get: (_, name) => (typescript ??= require("typescript"))[name] });

export {
  applyMixedDelta, chooseCatalogViews, layoutReference, refuseCatalogIssues,
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
  !/^src\/features\/(?:landing|nyxbot-onboarding)\//.test(file) &&
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

/**
 * Resolve a relative or aliased module specifier to a package-relative POSIX
 * path, independently of the current machine. Bare package specifiers are
 * returned unchanged.
 */
export function resolveSpecifier(file, specifier, aliases) {
  const alias = aliases.find(([from]) => specifier.startsWith(from));
  if (alias) return path.posix.normalize(`${alias[1]}${specifier.slice(alias[0].length)}`);
  if (specifier.startsWith(".")) return path.posix.normalize(path.posix.join(path.posix.dirname(file), specifier));
  return specifier;
}

/** Pre-split catalog files and the module each import must use instead. */
export const LEGACY_CATALOG_IMPORTS = {
  "src/i18n/locales/zh-CN.json": "src/i18n/locales/app/zh-CN",
  "src/i18n/locales/zh-TW.json": "src/i18n/locales/app/zh-TW",
  "src/i18n/locales/en.json": "src/i18n/locales/app/en-full",
  "src/i18n/locales/en-overrides.json": "src/i18n/locales/app/en-overrides.json",
};

/**
 * The replacement for a specifier that names a legacy catalog exactly, keeping
 * its prefix style (alias or relative); null for anything else.
 */
export function legacyImportReplacement(file, specifier, legacy, aliases) {
  const resolved = resolveSpecifier(file, specifier, aliases);
  if (!Object.hasOwn(legacy, resolved)) return null;
  const tail = path.posix.basename(resolved);
  if (!specifier.endsWith(`/${tail}`)) return null;
  return `${specifier.slice(0, -tail.length)}${path.posix.relative(path.posix.dirname(resolved), legacy[resolved])}`;
}

/**
 * Rewrite legacy catalog specifiers in `sources` ([file, text] pairs).
 * `specifiers(file, text)` lists module-specifier string literals as
 * `{ start, end, text, line, attributes? }` (start/end include the quotes;
 * `attributes` is the range of its import attributes clause). Returns the
 * changed texts and one `{ file, line, from, to, removedAttributes }` entry per
 * rewrite; attributes are removed when the replacement is no longer JSON.
 */
export function rewriteLegacyImports(sources, specifiers, legacy, aliases) {
  const changed = [];
  const rewrites = [];
  for (const [file, text] of sources) {
    let next = text;
    const found = specifiers(file, text)
      .map((literal) => ({ ...literal, to: legacyImportReplacement(file, literal.text, legacy, aliases) }))
      .filter((literal) => literal.to);
    const edits = found.flatMap((literal) => [
      { start: literal.start + 1, end: literal.end - 1, text: literal.to },
      // JSON import attributes (`with`/`assert { type: "json" }`) are invalid once the target is a module.
      ...(literal.attributes && !literal.to.endsWith(".json") ? [{ ...literal.attributes, text: "" }] : []),
    ]);
    for (const edit of edits.sort((a, b) => b.start - a.start)) next = `${next.slice(0, edit.start)}${edit.text}${next.slice(edit.end)}`;
    for (const literal of found) {
      rewrites.push({ file, line: literal.line, from: literal.text, to: literal.to, removedAttributes: Boolean(literal.attributes && !literal.to.endsWith(".json")) });
    }
    if (found.length) changed.push([file, next]);
  }
  return { changed, rewrites };
}

const MODULE_CALLS = /^(?:require|require\.resolve|import\.meta\.glob|(?:vi|jest)\.(?:mock|doMock|unmock|importActual|importMock|requireActual|requireMock))$/;
/** The module-specifier literal of an import/export, import type, import(), require() or vi/jest mock call. */
export function moduleSpecifierOf(node, ast) {
  const specifier = (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) ? node.moduleSpecifier :
    ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword || MODULE_CALLS.test(node.expression.getText(ast))) ? node.arguments[0] :
    ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) ? node.argument.literal : undefined;
  return specifier && ts.isStringLiteralLike(specifier) ? specifier : undefined;
}

/** Every module-specifier literal in a TypeScript source, for rewriteLegacyImports. */
export function moduleSpecifiers(file, source) {
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const found = [];
  const visit = (node) => {
    const literal = moduleSpecifierOf(node, ast);
    if (literal) {
      // The import attributes clause after the specifier, if any: ` with { ... }` / ` assert { ... }`, or import()'s options argument.
      const attributes = (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.attributes ? { start: literal.getEnd(), end: node.attributes.getEnd() } :
        ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword && node.arguments[1] ? { start: literal.getEnd(), end: node.arguments[1].getEnd() } : undefined;
      found.push({ start: literal.getStart(ast), end: literal.getEnd(), text: literal.text, line: ast.getLineAndCharacterOfPosition(literal.getStart(ast)).line + 1, attributes });
    }
    ts.forEachChild(node, visit);
  };
  visit(ast);
  return found;
}

/** Cheap text filter: could this source import a legacy catalog? */
export const mayImportLegacyCatalog = (source, legacy) =>
  Object.keys(legacy).some((file) => source.includes(path.posix.basename(file)));

/** Resolve relative, @/ and src/ imports independently of the current machine. */
export function catalogImportIssue(file, specifier) {
  if (isTestSource(file)) return null;
  const resolved = resolveSpecifier(file, specifier.split(/[?#]/)[0], SPECIFIER_ALIASES);
  if (!resolved.startsWith("src/i18n/locales/")) return null;
  const isBarrel = /^src\/i18n\/locales\/app\/(?:en-full|zh-CN|zh-TW)\.ts$/.test(file);
  if (/\/en-full(?:\.[cm]?[jt]s)?$/.test(resolved)) return "full English catalog is test-only";
  if (/^src\/i18n\/locales\/app\/[^/]+\/(?:en|zh-CN|zh-TW)\.json$/.test(resolved) && !isBarrel) return "portion JSON imports are restricted to the catalog barrels and tests";
  if (!["src/i18n/index.ts", "src/i18n/instance.ts", "src/i18n/wizard.ts"].includes(file) && !isBarrel) return "catalog imports are restricted to sanctioned i18n modules and tests";
  return null;
}
