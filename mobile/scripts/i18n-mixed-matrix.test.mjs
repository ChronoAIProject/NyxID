/**
 * Exhaustive matrix for the mobile "mixed branch" recovery route: a legacy branch commits a merge
 * M of the split main that kept the monoliths, then commit X, then merges main again. Every case
 * owns one key and edits it at one or more points of that history; one real-git history hosts all
 * cases of a run. Adopt must refuse the auto-detected merge with the legacy-fork command, and that
 * command must apply every branch edit and keep main's latest value otherwise (never a silent
 * loss, resurrection or revert), or refuse genuine conflicts with the exact diverged keys.
 */
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { after, before, describe, test } from "node:test";
import { canonicalText } from "../../shared/i18n/catalog-core.mjs";
import { LANGUAGES, auditCatalogs } from "./lib/i18n-catalogs.mjs";

const mobile = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MAIN_POINTS = ["mainPreSplit", "mainBeforeM", "mainAfterM"];
const BRANCH_POINTS = ["branchBeforeM", "resolutionM", "branchX", "branchAreas"];
const KINDS = ["add", "change", "remove"];
// Valid add/change/remove sequences on one key: [first, second, present initially].
const SEQUENCES = [["add", "change", false], ["add", "remove", false], ["change", "change", true], ["change", "remove", true], ["remove", "add", true]];

const value = (key, op, language) => `${op.tag ?? op.point}-${key} ${language}`;
const initial = (key, language) => `init-${key} ${language}`;
const describeOps = (ops) => ops.map((op) => `${op.kind} ${op.point}${op.tag ? `(${op.tag})` : ""}`).join(" then ");
const isMain = (op) => MAIN_POINTS.includes(op.point);

/** The correct outcome: the branch's last edit if the branch touched the key, else main's latest. */
function expected(testCase, language) {
  const ops = testCase.ops.some((op) => !isMain(op)) ? testCase.ops.filter((op) => !isMain(op)) : testCase.ops;
  let state = testCase.present ? initial(testCase.key, language) : undefined;
  for (const op of ops) state = op.kind === "remove" ? undefined : value(testCase.key, op, language);
  return state;
}

function buildCases() {
  const clean = [];
  const conflicts = [];
  for (const point of [...MAIN_POINTS, ...BRANCH_POINTS]) for (const kind of KINDS) clean.push({ present: kind !== "add", ops: [{ point, kind }] });
  for (const [first, second] of [["mainPreSplit", "mainAfterM"], ["mainBeforeM", "mainAfterM"], ["branchBeforeM", "branchX"], ["resolutionM", "branchX"], ["mainBeforeM", "branchAreas"]]) {
    for (const [a, b, present] of SEQUENCES) clean.push({ present, ops: [{ point: first, kind: a }, { point: second, kind: b }] });
  }
  for (const kind of KINDS) clean.push({ present: kind !== "add", ops: [{ point: "branchX", kind, tag: "mirrored" }, { point: "branchAreas", kind, tag: "mirrored" }] });
  for (const kind of ["change", "remove"]) conflicts.push({ present: true, ops: [{ point: "mainAfterM", kind: "change" }, { point: "branchX", kind }], conflict: kind === "change" ? "changed" : "removed" });
  conflicts.push({ present: true, ops: [{ point: "mainAfterM", kind: "remove" }, { point: "branchX", kind: "change" }], conflict: "changed" });
  conflicts.push({ present: true, ops: [{ point: "mainAfterM", kind: "change" }, { point: "branchAreas", kind: "change" }], conflict: "changed" });
  conflicts.push({ present: true, ops: [{ point: "branchX", kind: "change" }, { point: "branchAreas", kind: "change" }], conflict: "changed" });
  const named = (cases, prefix) => cases.map((testCase, index) => {
    const key = `${prefix}${index}`;
    const result = testCase.conflict ? null : expected({ ...testCase, key }, "zh-CN");
    const outcome = testCase.conflict ? `refused (${testCase.conflict} key diverged)` : result === undefined ? "absent" : result.startsWith("init-") ? "initial kept" : `${result.split("-")[0]} wins`;
    return { ...testCase, key, name: `mobile ${key}: ${testCase.present ? "present" : "absent"}, ${describeOps(testCase.ops)} -> ${outcome}` };
  });
  return { clean: named(clean, "k"), conflicts: named(conflicts, "c") };
}

/** One real-git history hosting every case, adopted by following the tool's printed guidance. */
function rehearse(cases) {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-mobile-i18n-matrix-"));
  const root = path.join(repo, "mobile");
  const locales = path.join(root, "src/i18n/locales");
  const git = (...args) => execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  const write = (file, text) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); };
  const read = (file) => fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {};
  const adopt = (...args) => spawnSync(process.execPath, [path.join(root, "scripts/adopt-i18n.mjs"), ...args], { cwd: root, encoding: "utf8" });
  const monolithFile = (language) => path.join(locales, `${language}.json`);
  const areaFiles = (language) => fs.readdirSync(locales, { withFileTypes: true }).filter((entry) => entry.isDirectory())
    .map((entry) => path.join(locales, entry.name, `${language}.json`)).filter((file) => fs.existsSync(file));
  const commit = (message) => { git("add", "-A", "mobile"); git("commit", "-q", "--allow-empty", "-m", message); return git("rev-parse", "HEAD"); };

  /** Apply each case's edits for `point` to the layout that point writes. */
  const edit = (point) => {
    for (const language of LANGUAGES) {
      const files = new Map();
      const load = (file) => { if (!files.has(file)) files.set(file, read(file)); return files.get(file); };
      for (const testCase of cases) {
        for (const op of testCase.ops.filter((candidate) => candidate.point === point)) {
          const file = fs.existsSync(monolithFile(language)) && point !== "branchAreas" ? monolithFile(language)
            : areaFiles(language).find((candidate) => Object.hasOwn(load(candidate), testCase.key)) ?? path.join(locales, "unattributed", `${language}.json`);
          const catalog = load(file);
          if (op.kind === "remove") delete catalog[testCase.key];
          else catalog[testCase.key] = value(testCase.key, op, language);
        }
      }
      for (const [file, catalog] of files) write(file, canonicalText(catalog));
    }
  };

  for (const file of ["adopt-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"]) write(path.join(root, "scripts", file), fs.readFileSync(path.join(mobile, "scripts", file), "utf8"));
  write(path.join(repo, "shared/i18n/catalog-core.mjs"), fs.readFileSync(path.join(mobile, "../shared/i18n/catalog-core.mjs"), "utf8"));
  fs.symlinkSync(path.join(mobile, "node_modules"), path.join(root, "node_modules"));
  write(path.join(repo, ".gitignore"), "node_modules\n");
  git("init", "-q", "-b", "main");
  for (const [name, setting] of [["user.name", "matrix"], ["user.email", "matrix@example.test"], ["core.hooksPath", "/dev/null"], ["commit.gpgsign", "false"]]) git("config", name, setting);
  // F: the legacy fork, with every initially present key.
  for (const language of LANGUAGES) {
    write(monolithFile(language), canonicalText(Object.fromEntries(cases.filter((testCase) => testCase.present).map((testCase) => [testCase.key, initial(testCase.key, language)]))));
  }
  const fork = commit("F: legacy fork");
  git("checkout", "-q", "-b", "legacy");
  git("checkout", "-q", "main");
  edit("mainPreSplit"); const preSplit = commit("P: main before the split");
  const split = adopt("--branch", preSplit, "--base", "empty", "--onto", preSplit);
  if (split.status !== 0) throw new Error(split.stderr);
  commit("S0: split");
  edit("mainBeforeM"); commit("S1: main before M");
  git("checkout", "-q", "legacy");
  edit("branchBeforeM"); const legacy = commit("L: branch before M");
  spawnSync("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo, encoding: "utf8" });
  // M's resolution keeps the monoliths.
  git("checkout", legacy, "--", ...LANGUAGES.map((language) => path.relative(repo, monolithFile(language))));
  edit("resolutionM"); commit("M: merge main, keeping the monoliths");
  edit("branchX"); edit("branchAreas"); commit("X: more branch edits");
  git("checkout", "-q", "main");
  edit("mainAfterM"); commit("S2: main after M");
  git("checkout", "-q", "legacy");
  spawnSync("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo, encoding: "utf8" });
  // Follow the tool exactly as a user would: run adopt, then the command it prints.
  const refused = adopt();
  const command = refused.stderr.split("\n").find((line) => line.startsWith("  pnpm adopt:i18n --branch"));
  const guided = command ? spawnSync("sh", ["-c", command.trim().replace("pnpm adopt:i18n", `"${process.execPath}" scripts/adopt-i18n.mjs`)], { cwd: root, encoding: "utf8" }) : null;
  // Read lazily: after a refusal the second merge's conflicted catalogs are left in the working tree.
  const lookup = (language) => Object.assign({}, ...areaFiles(language).map(read));
  const monolithsLeft = LANGUAGES.filter((language) => fs.existsSync(monolithFile(language)));
  return { repo, locales, fork, refused, command, guided, lookup, monolithsLeft };
}

const { clean, conflicts } = buildCases();

describe(`mobile mixed-branch adoption matrix: ${clean.length} clean cases`, () => {
  let run;
  before(() => { run = rehearse(clean); });
  after(() => fs.rmSync(run.repo, { recursive: true, force: true }));

  test("refuses the auto-detected merge and prints the legacy-fork command", () => {
    assert.notEqual(run.refused.status, 0);
    assert.ok(run.refused.stderr.includes("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main"), run.refused.stderr);
    assert.equal(run.command, `  pnpm adopt:i18n --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
  });

  test("adopts with the printed command, without any refusal, and leaves a valid split", () => {
    assert.equal(run.guided?.status, 0, run.guided?.stderr);
    assert.deepEqual(run.monolithsLeft, []);
    assert.deepEqual(auditCatalogs(run.locales), []);
  });

  for (const testCase of clean) {
    test(testCase.name, () => {
      for (const language of LANGUAGES) assert.equal(run.lookup(language)[testCase.key], expected(testCase, language), language);
    });
  }
});

describe(`mobile mixed-branch adoption matrix: ${conflicts.length} genuine conflicts`, () => {
  let run;
  before(() => { run = rehearse(conflicts); });
  after(() => fs.rmSync(run.repo, { recursive: true, force: true }));

  test("refuses with the exact diverged keys and writes nothing", () => {
    assert.equal(run.command, `  pnpm adopt:i18n --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
    assert.notEqual(run.guided?.status, 0);
    assert.ok(run.guided.stderr.includes("No files written. Resolve these catalog divergences explicitly:"), run.guided.stderr);
    const reported = run.guided.stderr.split("\n").filter((line) => line.includes("key diverged") || line.includes("changed differently")).sort();
    const expectedLines = conflicts.flatMap((testCase) => LANGUAGES.map((language) => {
      const label = testCase.ops.some((op) => op.point === "branchAreas") ? `mobile split areas/${language}` : `mobile/${language}`;
      // The branch's monoliths and areas changing one key differently is reported as a contradiction.
      if (testCase.ops.some((op) => op.point === "branchX") && testCase.ops.some((op) => op.point === "branchAreas")) return `${label}: monoliths and split areas changed differently: ${testCase.key}`;
      return `${label}: ${testCase.conflict} key diverged: ${testCase.key}`;
    })).sort();
    assert.deepEqual(reported, expectedLines);
    assert.deepEqual(run.monolithsLeft, LANGUAGES);
  });

  for (const testCase of conflicts) {
    test(testCase.name, () => assert.match(run.guided.stderr, new RegExp(`: (?:${testCase.conflict} key diverged|monoliths and split areas changed differently): ${testCase.key}\n`)));
  }
});
