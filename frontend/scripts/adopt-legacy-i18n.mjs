#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  LANGUAGES, FAMILIES, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES, analyzePortions, applyDelta, chooseCatalogViews,
  consolidateDuplicates, emptyLanguages, englishOverrides, isPlacementSource, mayImportLegacyCatalog,
  moduleSpecifiers, placeCatalogs, rewriteLegacyImports, walkFiles, writeJson, writePortions,
} from "./lib/i18n-catalogs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: root, encoding: "utf8" }).trim();
const git = (...args) => execFileSync("git", args, { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trimEnd();
const prefix = path.relative(repoRoot, root).split(path.sep).join("/");
const locales = path.join(root, "src/i18n/locales");
const args = process.argv.slice(2);

const USAGE = "usage: npm run adopt:i18n -- --branch <ref> --base <ref|empty> --onto <ref> | --imports-only";

function resolveRefs() {
  const refs = {};
  for (let i = 0; i < args.length; i += 2) {
    if (!["--branch", "--base", "--onto"].includes(args[i]) || !args[i + 1]) throw new Error(USAGE);
    refs[args[i].slice(2)] = args[i + 1];
  }
  if (Object.keys(refs).length) {
    if (Object.keys(refs).length !== 3) throw new Error("--branch, --base and --onto must be supplied together");
    return refs;
  }
  const exists = (name) => fs.existsSync(path.resolve(repoRoot, git("rev-parse", "--git-path", name)));
  const resolves = (ref) => {
    try { git("rev-parse", "--verify", "--quiet", `${ref}^{commit}`); return true; } catch { return false; }
  };
  if (exists("rebase-apply/applying")) {
    throw new Error(`git am is in progress; a mailbox patch has no commit to read the old catalogs from. Pass the legacy commit explicitly (npm run adopt:i18n -- --branch <legacy-commit> --base <legacy-commit>^ --onto HEAD), or run git am --abort and merge, rebase or cherry-pick the branch instead.\n${USAGE}`);
  }
  if (exists("rebase-merge") || exists("rebase-apply")) {
    if (resolves("REBASE_HEAD")) return { branch: "REBASE_HEAD", base: "REBASE_HEAD^", onto: "HEAD" };
    throw new Error(`a rebase is in progress but REBASE_HEAD does not resolve to the commit being applied; pass the refs explicitly (--branch <commit-being-applied> --base <its-parent> --onto HEAD).\n${USAGE}`);
  }
  if (resolves("CHERRY_PICK_HEAD")) return { branch: "CHERRY_PICK_HEAD", base: "CHERRY_PICK_HEAD^", onto: "HEAD" };
  if (exists("MERGE_HEAD")) return { branch: "HEAD", onto: "MERGE_HEAD", base: git("merge-base", "HEAD", "MERGE_HEAD") };
  throw new Error(`no merge, rebase or cherry-pick detected; provide --branch, --base and --onto.\n${USAGE}`);
}

const tryGit = (...args) => { try { return git(...args); } catch { return null; } };

/**
 * The branch's legacy fork candidates: the newest pre-split commits it is based
 * on through its own history. The split is the commit in onto's history that
 * deleted the monoliths; boundary commits of `rev-list <branch> ^<split>` that
 * still have every monolith are the legacy commits the branch grew from. Main's
 * own pre-split commits are excluded because they are reachable from the split.
 */
function legacyForks(branchRef, ontoRef) {
  const monoliths = LANGUAGES.map((language) => `${prefix}/src/i18n/locales/${language}.json`);
  const split = tryGit("log", "-1", "--format=%H", "--diff-filter=D", ontoRef, "--", ...monoliths);
  if (!split) return [];
  const boundary = (tryGit("rev-list", "--boundary", branchRef, `^${split}`) ?? "").split("\n")
    .filter((line) => line.startsWith("-")).map((line) => line.slice(1));
  const legacy = boundary.filter((commit) => monoliths.every((file) => tryGit("cat-file", "-e", `${commit}:${file}`) !== null));
  return legacy.length ? git("merge-base", "--independent", ...legacy).split("\n").filter(Boolean) : [];
}

/** Read many blobs from one ref with a single `git cat-file --batch` process. */
function readBlobs(ref, files) {
  if (!files.length) return new Map();
  const output = execFileSync("git", ["cat-file", "--batch"], {
    cwd: repoRoot, input: files.map((file) => `${ref}:${file}\n`).join(""), maxBuffer: 1 << 30,
  });
  const blobs = new Map();
  let offset = 0;
  for (const file of files) {
    const newline = output.indexOf(10, offset);
    const header = output.toString("utf8", offset, newline);
    const match = header.match(/^[0-9a-f]+ blob (\d+)$/);
    if (!match) throw new Error(`${ref}:${file}: unexpected git cat-file header: ${header}`);
    const start = newline + 1;
    const end = start + Number(match[1]);
    blobs.set(file, output.toString("utf8", start, end));
    offset = end + 1;
  }
  return blobs;
}

function snapshot(ref, { sources: withSources }) {
  if (ref === "empty") return { areas: null, monolith: null, monolithLanguages: [], portions: {}, families: Object.fromEntries(FAMILIES.map((family) => [family, emptyLanguages()])), sources: [] };
  const files = execFileSync("git", ["ls-tree", "-r", "-z", "--name-only", ref, "--", `${prefix}/src`], { cwd: repoRoot, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0").filter(Boolean).map((file) => file.slice(prefix.length + 1));
  const catalogPaths = files.filter((file) => /^src\/i18n\/locales\/.*\.json$/.test(file));
  const sourcePaths = withSources ? files.filter(isPlacementSource) : [];
  const blobs = readBlobs(ref, [...catalogPaths, ...sourcePaths].map((file) => `${prefix}/${file}`));
  const text = (file) => blobs.get(`${prefix}/${file}`);
  const read = (file) => blobs.has(`${prefix}/${file}`) ? JSON.parse(text(file)) : {};
  const portions = {};
  for (const file of catalogPaths) {
    const match = file.match(/^src\/i18n\/locales\/app\/([^/]+)\/(en|zh-CN|zh-TW)\.json$/);
    if (!match) continue;
    portions[match[1]] ??= emptyLanguages();
    portions[match[1]][match[2]] = read(file);
  }
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`${ref}: ${analysis.issues.join("\n")}`);
  // Both layouts are reported; chooseCatalogViews decides which one each role uses.
  const areas = Object.keys(portions).length ? analysis.merged : null;
  const monolithLanguages = LANGUAGES.filter((language) => blobs.has(`${prefix}/src/i18n/locales/${language}.json`));
  const monolith = monolithLanguages.length ? Object.fromEntries(LANGUAGES.map((language) => [language, read(`src/i18n/locales/${language}.json`)])) : null;
  const families = Object.fromEntries(FAMILIES.map((family) => [family, Object.fromEntries(LANGUAGES.map((language) => [language, read(`src/i18n/locales/${family}-${language}.json`)]))]));
  const sources = sourcePaths.map((file) => [file, text(file)]);
  return { areas, monolith, monolithLanguages, portions, families, sources };
}

/**
 * Point imports of the pre-split catalog files (tests included) at their
 * replacements in the working tree, printing each rewrite.
 */
function rewriteImports() {
  const sources = walkFiles(path.join(root, "src"))
    .map((file) => [path.relative(root, file).split(path.sep).join("/"), file])
    .filter(([file]) => /\.tsx?$/.test(file))
    .map(([file, absolute]) => [file, fs.readFileSync(absolute, "utf8")])
    .filter(([, text]) => mayImportLegacyCatalog(text, LEGACY_CATALOG_IMPORTS));
  const { changed, rewrites } = rewriteLegacyImports(sources, moduleSpecifiers, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES);
  for (const [file, text] of changed) fs.writeFileSync(path.join(root, file), text);
  for (const { file, line, from, to, removedAttributes } of rewrites) console.log(`i18n: rewrote ${file}:${line} ${from} -> ${to}${removedAttributes ? " (removed its JSON import attributes)" : ""}`);
  if (changed.length) {
    console.log(`i18n: stage the rewritten imports too: git add ${changed.map(([file]) => file).join(" ")}`);
    console.log("i18n: rewritten imports now load Record<string, string> catalogs, so indexing one with a literal key can be string | undefined; fix any such npx tsc -b error by hand.");
  }
  console.log("i18n: run npx tsc -b to type-check the result.");
}

try {
  if (args.length === 1 && args[0] === "--imports-only") {
    rewriteImports();
    process.exit();
  }
  const refs = resolveRefs();
  // Placement reads branch sources, plus onto sources unless adopting from an empty base.
  const base = snapshot(refs.base, { sources: false });
  const branch = snapshot(refs.branch, { sources: true });
  const onto = snapshot(refs.onto, { sources: refs.base !== "empty" });
  // A mixed branch (monoliths kept beside split areas) already contains main up to the latest main it
  // merged, the merge-base with onto: its split areas are diffed against that.
  const mixed = Boolean(branch.areas && branch.monolith);
  const mergeBase = mixed ? tryGit("merge-base", refs.branch, refs.onto) : null;
  const latestMerged = mergeBase ? snapshot(mergeBase, { sources: false }) : null;
  const app = chooseCatalogViews({ base, branch, onto }, refs, {
    languages: LANGUAGES, adopt: "npm run adopt:i18n --", inherited: latestMerged?.areas ?? null,
    legacyFork: () => legacyForks(refs.branch, refs.onto),
  });
  const conflicts = [];
  const merged = emptyLanguages();
  const families = Object.fromEntries(FAMILIES.map((family) => [family, emptyLanguages()]));
  for (const language of LANGUAGES) {
    const delta = applyDelta(app.base[language], app.branch[language], refs.base === "empty" ? {} : app.onto[language], `app/${language}`);
    merged[language] = delta.result;
    conflicts.push(...delta.conflicts);
    if (app.areaDelta) {
      // Then the branch's own edits to the split areas it inherited (mirrored edits are no-ops).
      const areaDelta = applyDelta(app.areaDelta.base[language], app.areaDelta.branch[language], merged[language], `app split areas/${language}`);
      merged[language] = areaDelta.result;
      conflicts.push(...areaDelta.conflicts);
    }
    for (const family of FAMILIES) {
      const delta = applyDelta(base.families[family][language], branch.families[family][language], refs.base === "empty" ? {} : onto.families[family][language], `${family}/${language}`);
      families[family][language] = delta.result;
      conflicts.push(...delta.conflicts);
    }
  }
  if (conflicts.length) throw new Error(`No files written. Resolve these catalog divergences explicitly:\n${conflicts.join("\n")}`);
  const sources = refs.base === "empty" ? branch.sources : [...onto.sources, ...branch.sources];
  const portions = consolidateDuplicates(placeCatalogs(merged, sources, refs.base === "empty" ? {} : onto.portions));
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`No files written. ${analysis.issues.join("\n")}`);
  writePortions(locales, portions);
  for (const family of FAMILIES) for (const language of LANGUAGES) writeJson(path.join(locales, `${family}-${language}.json`), families[family][language]);
  writeJson(path.join(locales, "app/en-overrides.json"), englishOverrides(merged.en));
  for (const file of [...LANGUAGES.map((language) => `${language}.json`), "en-overrides.json"]) {
    const full = path.join(locales, file);
    if (fs.existsSync(full)) fs.rmSync(full);
  }
  console.log(`i18n: adopted ${refs.branch} (base ${refs.base}) onto ${refs.onto}; ${Object.keys(merged.en).length} app keys. Stage the catalog files with git add -A src/i18n/locales.`);
  rewriteImports();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
