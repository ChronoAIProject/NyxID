import fs from "node:fs";
import path from "node:path";
import ts from "typescript";
import { portionForPath } from "./i18n-portions.mjs";

export const LANGUAGES = ["en", "zh-CN", "zh-TW"];
export const FAMILIES = ["common", "errors", "wizard"];
export const codeUnitCompare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
export const sortObject = (object) => Object.fromEntries(
  Object.keys(object).sort(codeUnitCompare).map((key) => [key, object[key]]),
);
export const canonicalText = (object) => `${JSON.stringify(sortObject(object), null, 2)}\n`;
export const englishOverrides = (english) => Object.fromEntries(
  Object.entries(sortObject(english)).filter(([key, value]) => key !== value),
);
export const stripPluralSuffix = (key) => key.replace(/_(?:zero|one|two|few|many|other)$/, "");
export const isTestSource = (file) => /(?:\.(?:test|spec)\.[cm]?[jt]sx?$|\/(?:__tests__|test|tests)\/)/.test(file);

export function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

export function writeJson(file, object) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const text = canonicalText(object);
  if (!fs.existsSync(file) || fs.readFileSync(file, "utf8") !== text) fs.writeFileSync(file, text);
}

export function walkFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    return entry.isDirectory() ? walkFiles(file) : entry.isFile() ? [file] : [];
  }).sort(codeUnitCompare);
}

export function catalogFiles(locales) {
  return walkFiles(locales).filter((file) => file.endsWith(".json") && !file.endsWith("/en-overrides.json"));
}

export function allCatalogFiles(locales) {
  return walkFiles(locales).filter((file) => file.endsWith(".json"));
}

export function discoverPortions(locales) {
  const dir = path.join(locales, "app");
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort(codeUnitCompare);
}

export const portionFile = (locales, portion, language) => path.join(locales, "app", portion, `${language}.json`);
export const emptyLanguages = () => Object.fromEntries(LANGUAGES.map((language) => [language, {}]));

export function loadPortions(locales) {
  return Object.fromEntries(discoverPortions(locales).map((portion) => [portion,
    Object.fromEntries(LANGUAGES.map((language) => {
      const file = portionFile(locales, portion, language);
      return [language, fs.existsSync(file) ? readJson(file) : {}];
    })),
  ]));
}

export function analyzePortions(portions) {
  const merged = emptyLanguages();
  const locations = new Map();
  const issues = [];
  for (const portion of Object.keys(portions).sort(codeUnitCompare)) {
    const catalogs = portions[portion];
    const keySets = LANGUAGES.map((language) => canonicalText(Object.fromEntries(Object.keys(catalogs[language]).map((key) => [key, true]))));
    if (new Set(keySets).size > 1) issues.push(`app/${portion}: key parity differs across en/zh-CN/zh-TW`);
    for (const language of LANGUAGES) {
      for (const [key, value] of Object.entries(catalogs[language])) {
        if (!locations.has(key)) locations.set(key, new Set());
        locations.get(key).add(portion);
        merged[language][key] = value;
      }
    }
  }
  const duplicates = [];
  for (const [key, owners] of locations) {
    if (owners.size < 2) continue;
    const names = [...owners];
    const identical = LANGUAGES.every((language) => names.every((portion) =>
      Object.hasOwn(portions[portion][language], key) &&
      portions[portion][language][key] === portions[names[0]][language][key],
    ));
    duplicates.push({ key, portions: names, identical });
    if (!identical) issues.push(`app duplicate key has differing values: ${key} (${names.join(", ")})`);
  }
  const pluralOwners = new Map();
  for (const [key, owners] of locations) {
    const base = stripPluralSuffix(key);
    const group = pluralOwners.get(base) ?? { keys: [], owners: new Set() };
    group.keys.push(key);
    for (const owner of owners) group.owners.add(owner);
    pluralOwners.set(base, group);
  }
  for (const [base, group] of pluralOwners) {
    if (group.keys.length > 1 && group.owners.size > 1) issues.push(`app plural siblings are not co-located: ${base} (${[...group.owners].join(", ")})`);
  }
  return { merged, locations, duplicates, issues };
}

export function consolidateDuplicates(portions) {
  const result = structuredClone(portions);
  const { duplicates, issues } = analyzePortions(portions);
  const differing = issues.filter((issue) => !issue.includes("plural siblings"));
  if (differing.length) throw new Error(differing.join("\n"));
  // Move the whole plural family when any sibling is duplicated.
  const bases = new Set(duplicates.map(({ key }) => stripPluralSuffix(key)));
  const { merged } = analyzePortions(portions);
  if (bases.size) result.shared ??= emptyLanguages();
  for (const language of LANGUAGES) {
    for (const [key, value] of Object.entries(merged[language])) {
      if (!bases.has(stripPluralSuffix(key))) continue;
      for (const catalogs of Object.values(result)) delete catalogs[language][key];
      result.shared[language][key] = value;
    }
  }
  return result;
}

export function filesystemSources(root) {
  return walkFiles(path.join(root, "src"))
    .map((file) => [path.relative(root, file).split(path.sep).join("/"), file])
    .filter(([file]) => /\.tsx?$/.test(file) && !isTestSource(file) && !file.includes("/locales/"))
    .map(([file, absolute]) => [file, fs.readFileSync(absolute, "utf8")]);
}

export function scanUsage(sources, keys) {
  const groups = new Map();
  for (const key of keys) {
    const base = stripPluralSuffix(key);
    if (!groups.has(base)) groups.set(base, new Set());
  }
  for (const [file, source] of sources) {
    if (!/\.tsx?$/.test(file) || isTestSource(file) || file.includes("/locales/")) continue;
    const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
    function visit(node) {
      if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
        groups.get(stripPluralSuffix(node.text))?.add(portionForPath(file));
      }
      ts.forEachChild(node, visit);
    }
    visit(ast);
  }
  return new Map([...groups].map(([key, owners]) => [key,
    owners.size === 0 ? "unattributed" : owners.size > 1 ? "shared" : [...owners][0],
  ]));
}

export function applyDelta(base, branch, onto, label = "catalog") {
  const result = { ...onto };
  const conflicts = [];
  const changes = [];
  for (const key of new Set([...Object.keys(base), ...Object.keys(branch)])) {
    const before = Object.hasOwn(base, key);
    const after = Object.hasOwn(branch, key);
    const current = Object.hasOwn(onto, key);
    if (before === after && base[key] === branch[key]) continue;
    changes.push(key);
    if (!before) {
      if (!current || onto[key] === branch[key]) result[key] = branch[key];
      else conflicts.push(`${label}: added key diverged: ${key}`);
    } else if (!after) {
      if (current && onto[key] === base[key]) delete result[key];
      else conflicts.push(`${label}: removed key diverged: ${key}`);
    } else if (current && onto[key] === base[key]) {
      result[key] = branch[key];
    } else {
      conflicts.push(`${label}: changed key diverged: ${key}`);
    }
  }
  return { result, conflicts, changes };
}

export function placeCatalogs(merged, sources, existing = {}) {
  const result = structuredClone(existing);
  const keys = [...new Set(LANGUAGES.flatMap((language) => Object.keys(merged[language])))];
  const placement = scanUsage(sources, keys);
  const existingOwners = analyzePortions(existing).locations;
  for (const language of LANGUAGES) {
    for (const catalogs of Object.values(result)) catalogs[language] = {};
    for (const key of keys) {
      if (!Object.hasOwn(merged[language], key)) continue;
      const siblingOwner = [...existingOwners].find(([sibling]) => stripPluralSuffix(sibling) === stripPluralSuffix(key))?.[1];
      const owner = existingOwners.get(key) ?? siblingOwner;
      const portion = owner?.size === 1 ? [...owner][0] : placement.get(stripPluralSuffix(key));
      result[portion] ??= emptyLanguages();
      result[portion][language][key] = merged[language][key];
    }
  }
  return result;
}

export function writePortions(locales, portions) {
  for (const [portion, catalogs] of Object.entries(portions)) {
    for (const language of LANGUAGES) writeJson(portionFile(locales, portion, language), catalogs[language]);
  }
  for (const portion of discoverPortions(locales)) {
    if (Object.hasOwn(portions, portion)) continue;
    for (const language of LANGUAGES) {
      const file = portionFile(locales, portion, language);
      if (fs.existsSync(file)) fs.rmSync(file);
    }
    const dir = path.join(locales, "app", portion);
    if (!fs.readdirSync(dir).length) fs.rmdirSync(dir);
  }
}

/** Resolve relative, @/ and src/ imports independently of the current machine. */
export function catalogImportIssue(file, specifier) {
  if (isTestSource(file)) return null;
  const resolved = specifier.startsWith("@/") ? `src/${specifier.slice(2)}` :
    specifier.startsWith(".") ? path.posix.normalize(path.posix.join(path.posix.dirname(file), specifier)) : specifier;
  if (!resolved.startsWith("src/i18n/locales/")) return null;
  const isBarrel = /^src\/i18n\/locales\/app\/(?:en-full|zh-CN|zh-TW)\.ts$/.test(file);
  if (/\/en-full(?:\.ts)?$/.test(resolved)) return "full English catalog is test-only";
  if (/^src\/i18n\/locales\/app\/[^/]+\/(?:en|zh-CN|zh-TW)\.json$/.test(resolved) && !isBarrel) return "portion JSON imports are restricted to the catalog barrels and tests";
  if (!["src/i18n/index.ts", "src/i18n/instance.ts", "src/i18n/wizard.ts"].includes(file) && !isBarrel) return "catalog imports are restricted to sanctioned i18n modules and tests";
  return null;
}
