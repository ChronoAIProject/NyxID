import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { portionForPath } from "./i18n-portions.mjs";

const require = createRequire(import.meta.url);
let typescript;
/** TypeScript, loaded on first use: adopting or syncing catalogs with no sources to parse never needs it. */
const ts = new Proxy({}, { get: (_, name) => (typescript ??= require("typescript"))[name] });

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
    const ownerSet = [...owners].sort(codeUnitCompare).join(", ");
    const previous = pluralOwners.get(base);
    if (previous !== undefined && previous !== ownerSet) issues.push(`app plural siblings are not co-located: ${base} (${previous}; ${ownerSet})`);
    pluralOwners.set(base, ownerSet);
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

export function scanUsage(sources, keys) {
  const groups = new Map();
  for (const key of keys) {
    const base = stripPluralSuffix(key);
    if (!groups.has(base)) groups.set(base, new Set());
  }
  for (const [file, source] of sources) {
    if (!isPlacementSource(file)) continue;
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
      if (!current || onto[key] === base[key]) delete result[key];
      else conflicts.push(`${label}: removed key diverged: ${key}`);
    } else if (current && onto[key] === base[key]) {
      result[key] = branch[key];
    } else if (current && onto[key] === branch[key]) {
      // A rerun after the same delta has already been adopted is a no-op.
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
  const siblingOwners = new Map([...existingOwners].map(([key, owners]) => [stripPluralSuffix(key), owners]));
  for (const language of LANGUAGES) {
    for (const catalogs of Object.values(result)) catalogs[language] = {};
    for (const key of keys) {
      if (!Object.hasOwn(merged[language], key)) continue;
      const owner = existingOwners.get(key) ?? siblingOwners.get(stripPluralSuffix(key));
      const portion = owner?.size === 1 ? [...owner][0] : placement.get(stripPluralSuffix(key));
      result[portion] ??= emptyLanguages();
      result[portion][language][key] = merged[language][key];
    }
  }
  return result;
}

/** Areas with at least one key in some language; an emptied area is pruned rather than kept as `{}` files. */
export const nonEmptyPortions = (portions) => Object.fromEntries(Object.entries(portions)
  .filter(([, catalogs]) => LANGUAGES.some((language) => Object.keys(catalogs[language] ?? {}).length)));

export function writePortions(locales, allPortions) {
  const portions = nonEmptyPortions(allPortions);
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

/**
 * Apply a mixed branch's edits to `onto`. Its monoliths changed from `monolithBase` (the legacy
 * fork) and its inherited split areas changed from `areaBase` (the last main it merged). Where the
 * areas changed a key, that edit was made against the newer main and takes precedence: the
 * monolith delta skips the key, and a monolith change to a different result is a contradiction.
 * Everything else goes through the ordinary three-way rules.
 */
export function applyMixedDelta({ monolithBase, monolith, areaBase, areas, onto, labels }) {
  const areaKeys = new Set([...Object.keys(areaBase), ...Object.keys(areas)].filter((key) => areaBase[key] !== areas[key]));
  const contradictions = [...areaKeys]
    .filter((key) => monolithBase[key] !== monolith[key] && monolith[key] !== areas[key])
    .map((key) => `${labels.areas}: monoliths and split areas changed differently: ${key}`);
  const withoutAreaKeys = (catalog) => Object.fromEntries(Object.entries(catalog).filter(([key]) => !areaKeys.has(key)));
  const first = applyDelta(withoutAreaKeys(monolithBase), withoutAreaKeys(monolith), onto, labels.monolith);
  const second = applyDelta(areaBase, areas, first.result, labels.areas);
  return { result: second.result, conflicts: [...first.conflicts, ...contradictions, ...second.conflicts] };
}

/**
 * The reference a branch layout is diffed against: that layout at the newer of `--base` and
 * merge-base(branch, onto) that has it (`order`: "base" when the base is the same or newer or there
 * is no merge-base, "mergeBase" when the merge-base is newer, "diverged" when neither contains the
 * other, in which case only the base counts). Each layout is reconciled with main at different
 * points, so the newest one that has it is where the branch's own edits to it begin: a rebased or
 * cherry-picked commit is diffed against its parent, while the legacy-fork route reads the
 * monoliths from the fork and the split areas and unsplit families from the merge-base. `layout` is
 * "monolith", "areas" or a family name. Returns null when no reference has the layout.
 */
export function layoutReference(layout, { base, mergeBase, order }) {
  const view = (snapshot) => !snapshot ? null : layout === "monolith" ? snapshot.monolith : layout === "areas" ? snapshot.areas
    : snapshot.familyPresent?.[layout] ? snapshot.families[layout] : null;
  const candidates = order === "mergeBase" ? [mergeBase, base] : order === "diverged" ? [base] : [base, mergeBase];
  for (const snapshot of candidates) {
    const catalog = view(snapshot);
    if (catalog) return catalog;
  }
  return null;
}

/**
 * Pick the app catalog of each adoption snapshot without losing data. A snapshot may hold the
 * split areas (`areas`, merged), the legacy monoliths (`monolith`, with `monolithLanguages`
 * present), or both, e.g. when a merge that kept the monoliths was committed. The branch uses its
 * monoliths (where legacy edits live) and onto uses its areas; a branch read from its monoliths
 * needs a base with monoliths, since a commit's two layouts come from different points in history.
 * Each branch layout is diffed against its `layoutReference` (`options.mergeBase`, `options.order`).
 * A mixed branch (monoliths beside split areas) also returns `areaDelta` (reference -> branch
 * areas) for applyMixedDelta. A missing reference is refused, never treated as empty.
 * `options.legacyFork()` returns the fork candidates (git-derived) for the refusal guidance.
 */
export function chooseCatalogViews(snapshots, refs, { languages, adopt, mergeBase = null, order = "base", legacyFork = () => [] }) {
  const fail = (lines) => { throw new Error(`No files written. ${[lines].flat().join("\n")}`); };
  /** The one route that adopts everything a monolith branch changed: its monoliths against its legacy fork. */
  const forkGuidance = () => {
    const forks = legacyFork();
    if (forks.length === 1) {
      return [
        `Adopt everything the branch changed by comparing its monoliths with its legacy fork ${forks[0]} (the newest pre-split commit it is based on):`,
        `  ${adopt} --branch ${refs.branch} --base ${forks[0]} --onto ${refs.onto}`,
      ];
    }
    return [
      forks.length
        ? `The branch is based on several unrelated pre-split commits (${forks.join(", ")}); pass the one its monoliths descend from as --base:`
        : "Pass the branch's legacy fork (the newest pre-split commit it is based on) as --base:",
      `  ${adopt} --branch ${refs.branch} --base <legacy-fork> --onto ${refs.onto}`,
    ];
  };
  for (const role of ["base", "branch", "onto"]) {
    const present = snapshots[role].monolithLanguages;
    if (present.length && present.length < languages.length) {
      fail(`${refs[role]} (--${role}) has only some legacy monoliths (${present.map((language) => `${language}.json`).join(", ")}); restore or delete the others, or pass a commit with one complete layout as --${role}.`);
    }
  }
  const { base, branch, onto } = snapshots;
  const mixed = Boolean(branch.monolith && branch.areas);
  const branchView = branch.monolith ? "monolith" : branch.areas ? "areas" : null;
  const baseHasCatalogs = Boolean(base.monolith || base.areas);
  const reference = (layout) => layoutReference(layout, { base, mergeBase, order });
  const inheritedNote = `${refs.branch} (--branch) keeps the legacy monoliths beside split areas it inherited from main, so its own edits live in the monoliths (and any later edits to those areas).`;
  if (!branchView && baseHasCatalogs) {
    fail(`${refs.branch} (--branch) has no app catalogs, so every key of ${refs.base} (--base) would be removed; pass the commit that holds the branch's catalogs as --branch.`);
  }
  if (mixed && !baseHasCatalogs) {
    fail([`${inheritedNote} It cannot be adopted from an empty base.`, ...forkGuidance()]);
  }
  if (mixed && order === "diverged") {
    fail(`${refs.base} (--base) and the merge-base of ${refs.branch} and ${refs.onto} do not contain one another, so the reference for the branch's split areas is ambiguous; pass a --base on the branch's own history (its legacy fork, or the parent of the commit being adopted).`);
  }
  if (mixed && !reference("areas")) {
    fail([
      `${refs.branch} (--branch) keeps the legacy monoliths beside split areas, but neither ${refs.base} (--base) nor the merge-base of ${refs.branch} and ${refs.onto} has split areas (the merge-base is missing or predates the split), so the branch's edits to those areas cannot be told apart from main's.`,
      `Pass an --onto that contains the main the branch merged. If the branch created those areas itself instead of merging them from main, remove them from the branch (git rm -r the area directories), commit, and rerun.`,
    ]);
  }
  if (branchView === "monolith" && baseHasCatalogs && !base.monolith) {
    fail([
      mixed
        ? `${inheritedNote} ${refs.base} (--base) has only the split areas; comparing the two layouts would treat every key main added or changed since the fork as removed or reverted by the branch.`
        : `${refs.branch} (--branch) has the legacy monoliths but ${refs.base} (--base) has only the split areas; comparing the two layouts would treat every key main added or changed since the fork as removed or reverted by the branch.`,
      ...forkGuidance(),
    ]);
  }
  if (branchView === "areas" && baseHasCatalogs && !base.areas) {
    fail(`${refs.base} (--base) has only the legacy monoliths, but ${refs.branch} (--branch) has only the split areas; pass a split-layout --base (the split commit the branch is based on), or adopt the branch's legacy commits first.`);
  }
  const empty = () => Object.fromEntries(languages.map((language) => [language, {}]));
  const chosen = {
    base: (baseHasCatalogs && branchView ? reference(branchView) : null) ?? empty(),
    branch: (branch.monolith ?? branch.areas) ?? empty(),
    onto: (onto.areas ?? onto.monolith) ?? empty(),
  };
  if (onto.areas && onto.monolith) {
    const lost = languages.flatMap((language) => Object.entries(onto.monolith[language])
      .filter(([key, value]) => !Object.values(chosen).some((catalog) => catalog[language][key] === value))
      .map(([key]) => `${language} ${JSON.stringify(key)}`));
    if (lost.length) {
      fail([
        `${refs.onto} (--onto) holds both the legacy monoliths and the split areas, and ${lost.length} value(s) exist only in its legacy monoliths (${lost.slice(0, 3).join(", ")}${lost.length > 3 ? ", ..." : ""}); adopting would lose them.`,
        "Move those values into the split areas, commit, and rerun.",
      ]);
    }
  }
  return { ...chosen, areaDelta: mixed ? { base: reference("areas"), branch: branch.areas } : null };
}

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
