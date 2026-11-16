#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  LANGUAGES, FAMILIES, analyzePortions, applyDelta, consolidateDuplicates,
  emptyLanguages, englishOverrides, isPlacementSource, placeCatalogs, writeJson, writePortions,
} from "./lib/i18n-catalogs.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd: root, encoding: "utf8" }).trim();
const git = (...args) => execFileSync("git", args, { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trimEnd();
const prefix = path.relative(repoRoot, root).split(path.sep).join("/");
const locales = path.join(root, "src/i18n/locales");
const args = process.argv.slice(2);

function resolveRefs() {
  const refs = {};
  for (let i = 0; i < args.length; i += 2) {
    if (!["--branch", "--base", "--onto"].includes(args[i]) || !args[i + 1]) throw new Error("usage: npm run adopt:i18n -- --branch <ref> --base <ref|empty> --onto <ref>");
    refs[args[i].slice(2)] = args[i + 1];
  }
  if (Object.keys(refs).length) {
    if (Object.keys(refs).length !== 3) throw new Error("--branch, --base and --onto must be supplied together");
    return refs;
  }
  const exists = (name) => fs.existsSync(path.resolve(repoRoot, git("rev-parse", "--git-path", name)));
  if (exists("rebase-merge") || exists("rebase-apply")) return { branch: "REBASE_HEAD", base: "REBASE_HEAD^", onto: "HEAD" };
  if (exists("MERGE_HEAD")) return { branch: "HEAD", onto: "MERGE_HEAD", base: git("merge-base", "HEAD", "MERGE_HEAD") };
  throw new Error("no merge or rebase detected; provide --branch, --base and --onto");
}

function snapshot(ref) {
  if (ref === "empty") return { app: emptyLanguages(), portions: {}, families: Object.fromEntries(FAMILIES.map((family) => [family, emptyLanguages()])), sources: [] };
  const files = git("ls-tree", "-r", "--name-only", ref, "--", `${prefix}/src`).split("\n");
  const present = new Set(files);
  const read = (relative) => {
    const file = `${prefix}/${relative}`;
    return present.has(file) ? JSON.parse(git("show", `${ref}:${file}`)) : {};
  };
  const portions = {};
  for (const file of files) {
    const match = file.slice(prefix.length + 1).match(/^src\/i18n\/locales\/app\/([^/]+)\/(en|zh-CN|zh-TW)\.json$/);
    if (!match) continue;
    portions[match[1]] ??= emptyLanguages();
    portions[match[1]][match[2]] = read(file.slice(prefix.length + 1));
  }
  const analysis = analyzePortions(portions);
  if (analysis.issues.length) throw new Error(`${ref}: ${analysis.issues.join("\n")}`);
  const app = Object.keys(portions).length ? analysis.merged : Object.fromEntries(LANGUAGES.map((language) => [language, read(`src/i18n/locales/${language}.json`)]));
  const families = Object.fromEntries(FAMILIES.map((family) => [family, Object.fromEntries(LANGUAGES.map((language) => [language, read(`src/i18n/locales/${family}-${language}.json`)]))]));
  const sources = files.map((file) => file.slice(prefix.length + 1)).filter(isPlacementSource).map((file) => [file, git("show", `${ref}:${prefix}/${file}`)]);
  return { app, portions, families, sources };
}

try {
  const refs = resolveRefs();
  const base = snapshot(refs.base);
  const branch = snapshot(refs.branch);
  const onto = snapshot(refs.onto);
  const conflicts = [];
  const merged = emptyLanguages();
  const families = Object.fromEntries(FAMILIES.map((family) => [family, emptyLanguages()]));
  for (const language of LANGUAGES) {
    const delta = applyDelta(base.app[language], branch.app[language], refs.base === "empty" ? {} : onto.app[language], `app/${language}`);
    merged[language] = delta.result;
    conflicts.push(...delta.conflicts);
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
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
