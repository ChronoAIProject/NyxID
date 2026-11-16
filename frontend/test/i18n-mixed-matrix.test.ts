// @vitest-environment node
/**
 * Exhaustive matrix for the "mixed branch" recovery route: a legacy branch commits a merge M of
 * the split main that kept the monoliths, then commit X, then merges main again. Every case owns
 * one key and edits it at one or more points of that history; one real-git history hosts all
 * cases of a run. Adopt must refuse the auto-detected merge with the legacy-fork command, and that
 * command must apply every branch edit and keep main's latest value otherwise (never a silent
 * loss, resurrection or revert), or refuse genuine conflicts with the exact diverged keys.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { FAMILIES, LANGUAGES, analyzePortions, canonicalText, loadPortions } from "../scripts/lib/i18n-catalogs.mjs";

const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
type Catalog = "app" | "common" | "errors" | "wizard";
type Point = "mainPreSplit" | "mainBeforeM" | "branchBeforeM" | "resolutionM" | "branchX" | "branchAreas" | "mainAfterM";
type Kind = "add" | "change" | "remove";
type Op = { point: Point; kind: Kind; tag?: string };
type Case = { name: string; catalog: Catalog; key: string; present: boolean; ops: Op[]; conflict?: string };
type Map = Record<string, string>;

const CATALOGS: Catalog[] = ["app", ...FAMILIES as Catalog[]];
const MAIN_POINTS: Point[] = ["mainPreSplit", "mainBeforeM", "mainAfterM"];
const BRANCH_POINTS: Point[] = ["branchBeforeM", "resolutionM", "branchX", "branchAreas"];
const KINDS: Kind[] = ["add", "change", "remove"];
// Valid add/change/remove sequences on one key: [first, second, present initially].
const SEQUENCES: [Kind, Kind, boolean][] = [["add", "change", false], ["add", "remove", false], ["change", "change", true], ["change", "remove", true], ["remove", "add", true]];

const value = (key: string, op: Op, language: string) => `${op.tag ?? op.point}-${key} ${language}`;
const initial = (key: string, language: string) => `init-${key} ${language}`;
const describeOps = (ops: Op[]) => ops.map((op) => `${op.kind} ${op.point}${op.tag ? `(${op.tag})` : ""}`).join(" then ");
const isMain = (op: Op) => MAIN_POINTS.includes(op.point);

/** The correct outcome: the branch's last edit if the branch touched the key, else main's latest. */
function expected(testCase: Case, language: string): string | undefined {
  const ops = testCase.ops.some((op) => !isMain(op)) ? testCase.ops.filter((op) => !isMain(op)) : testCase.ops;
  let state = testCase.present ? initial(testCase.key, language) : undefined;
  for (const op of ops) state = op.kind === "remove" ? undefined : value(testCase.key, op, language);
  return state;
}

function buildCases() {
  const clean: Omit<Case, "key">[] = [];
  const conflicts: Omit<Case, "key">[] = [];
  for (const catalog of CATALOGS) {
    const points = [...MAIN_POINTS, ...BRANCH_POINTS].filter((point) => catalog === "app" || point !== "branchAreas");
    for (const point of points) for (const kind of KINDS) clean.push({ name: "", catalog, present: kind !== "add", ops: [{ point, kind }] });
    for (const [first, second] of [["mainPreSplit", "mainAfterM"], ["mainBeforeM", "mainAfterM"], ["branchBeforeM", "branchX"], ["resolutionM", "branchX"]] as [Point, Point][]) {
      for (const [a, b, present] of SEQUENCES) clean.push({ name: "", catalog, present, ops: [{ point: first, kind: a }, { point: second, kind: b }] });
    }
    for (const kind of ["change", "remove"] as Kind[]) {
      conflicts.push({ name: "", catalog, present: true, ops: [{ point: "mainAfterM", kind: "change" }, { point: "branchX", kind }], conflict: kind === "change" ? "changed" : "removed" });
    }
    conflicts.push({ name: "", catalog, present: true, ops: [{ point: "mainAfterM", kind: "remove" }, { point: "branchX", kind: "change" }], conflict: "changed" });
  }
  // Edits to the inherited split areas, alone, after main's own edit, and mirrored in the monoliths.
  for (const [a, b, present] of SEQUENCES) clean.push({ name: "", catalog: "app", present, ops: [{ point: "mainBeforeM", kind: a }, { point: "branchAreas", kind: b }] });
  for (const kind of KINDS) {
    clean.push({ name: "", catalog: "app", present: kind !== "add", ops: [{ point: "branchX", kind, tag: "mirrored" }, { point: "branchAreas", kind, tag: "mirrored" }] });
  }
  conflicts.push({ name: "", catalog: "app", present: true, ops: [{ point: "mainAfterM", kind: "change" }, { point: "branchAreas", kind: "change" }], conflict: "changed", });
  conflicts.push({ name: "", catalog: "app", present: true, ops: [{ point: "branchX", kind: "change" }, { point: "branchAreas", kind: "change" }], conflict: "changed" });
  const named = (cases: Omit<Case, "key">[], prefix: string): Case[] => cases.map((testCase, index) => {
    const key = `${prefix}${index}`;
    const outcome = testCase.conflict ? `refused (${testCase.conflict} key diverged)` : (() => {
      const result = expected({ ...testCase, key, name: "" }, "en");
      return result === undefined ? "absent" : result.startsWith("init-") ? "initial kept" : `${result.split("-")[0]} wins`;
    })();
    return { ...testCase, key, name: `${testCase.catalog} ${key}: ${testCase.present ? "present" : "absent"}, ${describeOps(testCase.ops)} -> ${outcome}` };
  });
  return { clean: named(clean, "k"), conflicts: named(conflicts, "c") };
}

/** One real-git history hosting every case, adopted by following the tool's printed guidance. */
function rehearse(cases: Case[]) {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-i18n-matrix-"));
  const root = path.join(repo, "frontend");
  const locales = path.join(root, "src/i18n/locales");
  const git = (...args: string[]) => execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  const write = (file: string, text: string) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); };
  const read = (file: string): Map => fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {};
  const adopt = (...args: string[]) => spawnSync(process.execPath, [path.join(root, "scripts/adopt-legacy-i18n.mjs"), ...args], { cwd: root, encoding: "utf8" });
  const familyFile = (family: string, language: string) => path.join(locales, `${family}-${language}.json`);
  const monolithFile = (language: string) => path.join(locales, `${language}.json`);
  const commit = (message: string) => { git("add", "-A", "frontend"); git("commit", "-q", "--allow-empty", "-m", message); return git("rev-parse", "HEAD"); };

  /** Apply each case's edits for `point` to the layout that point writes. */
  const edit = (point: Point) => {
    for (const language of LANGUAGES) {
      const files = new Map<string, Map>();
      const load = (file: string) => { if (!files.has(file)) files.set(file, read(file)); return files.get(file)!; };
      for (const testCase of cases) {
        for (const op of testCase.ops.filter((candidate) => candidate.point === point)) {
          let file: string;
          if (testCase.catalog !== "app") file = familyFile(testCase.catalog, language);
          else if (fs.existsSync(monolithFile(language)) && point !== "branchAreas") file = monolithFile(language);
          else {
            const area = fs.readdirSync(path.join(locales, "app"), { withFileTypes: true }).filter((entry) => entry.isDirectory())
              .map((entry) => path.join(locales, "app", entry.name, `${language}.json`)).find((candidate) => Object.hasOwn(load(candidate), testCase.key));
            file = area ?? path.join(locales, "app/unattributed", `${language}.json`);
          }
          const catalog = load(file);
          if (op.kind === "remove") delete catalog[testCase.key];
          else catalog[testCase.key] = value(testCase.key, op, language);
        }
      }
      for (const [file, catalog] of files) write(file, canonicalText(catalog));
    }
  };

  fs.mkdirSync(path.join(root, "scripts/lib"), { recursive: true });
  for (const file of ["adopt-legacy-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"]) fs.copyFileSync(path.join(frontend, "scripts", file), path.join(root, "scripts", file));
  write(path.join(root, "package.json"), '{"type":"module"}\n');
  fs.symlinkSync(path.join(frontend, "node_modules"), path.join(root, "node_modules"));
  write(path.join(repo, ".gitignore"), "node_modules\n");
  git("init", "-q", "-b", "main");
  for (const [name, setting] of [["user.name", "matrix"], ["user.email", "matrix@example.test"], ["core.hooksPath", "/dev/null"], ["commit.gpgsign", "false"]]) git("config", name, setting);
  // F: the legacy fork, with every initially present key.
  for (const language of LANGUAGES) {
    for (const catalog of CATALOGS) {
      const entries = cases.filter((testCase) => testCase.catalog === catalog && testCase.present).map((testCase) => [testCase.key, initial(testCase.key, language)]);
      write(catalog === "app" ? monolithFile(language) : familyFile(catalog, language), canonicalText(Object.fromEntries(entries)));
    }
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
  // A correct resolution of M: keep the monoliths, and merge the unsplit family files key by key.
  git("checkout", legacy, "--", ...LANGUAGES.map((language) => path.relative(repo, monolithFile(language))));
  for (const family of FAMILIES) {
    for (const language of LANGUAGES) {
      const relative = path.relative(repo, familyFile(family, language));
      const show = (ref: string): Map => JSON.parse(git("show", `${ref}:${relative}`));
      const [base, ours, theirs] = [show(fork), show(legacy), show("main")];
      const result = { ...ours };
      for (const key of new Set([...Object.keys(base), ...Object.keys(theirs)])) {
        if (base[key] === theirs[key]) continue;
        if (Object.hasOwn(theirs, key)) result[key] = theirs[key]; else delete result[key];
      }
      write(familyFile(family, language), canonicalText(result));
    }
  }
  edit("resolutionM"); commit("M: merge main, keeping the monoliths");
  edit("branchX"); edit("branchAreas"); commit("X: more branch edits");
  git("checkout", "-q", "main");
  edit("mainAfterM"); commit("S2: main after M");
  git("checkout", "-q", "legacy");
  spawnSync("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo, encoding: "utf8" });
  // Follow the tool exactly as a user would: run adopt, then the command it prints.
  const refused = adopt();
  const command = refused.stderr.split("\n").find((line) => line.startsWith("  npm run adopt:i18n -- --branch"));
  const guided = command ? spawnSync("sh", ["-c", command.trim().replace("npm run adopt:i18n --", `"${process.execPath}" scripts/adopt-legacy-i18n.mjs`)], { cwd: root, encoding: "utf8" }) : null;
  // Read lazily: after a refusal the second merge's conflicted catalogs are left in the working tree.
  const lookup = (catalog: Catalog, language: string): Map =>
    catalog === "app" ? (analyzePortions(loadPortions(locales)).merged as Record<string, Map>)[language]! : read(familyFile(catalog, language));
  const monolithsLeft = LANGUAGES.filter((language) => fs.existsSync(monolithFile(language)));
  return { repo, fork, refused, command, guided, lookup, monolithsLeft };
}

const { clean, conflicts } = buildCases();

describe(`mixed-branch adoption matrix: ${clean.length} clean cases`, () => {
  let run: ReturnType<typeof rehearse>;
  beforeAll(() => { run = rehearse(clean); }, 60_000);
  afterAll(() => fs.rmSync(run.repo, { recursive: true, force: true }));

  it("refuses the auto-detected merge and prints the legacy-fork command", () => {
    expect(run.refused.status).not.toBe(0);
    expect(run.refused.stderr).toContain("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main");
    expect(run.command).toBe(`  npm run adopt:i18n -- --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
  });

  it("adopts with the printed command, without any refusal, and removes the monoliths", () => {
    expect(run.guided?.status, run.guided?.stderr).toBe(0);
    expect(run.monolithsLeft).toEqual([]);
  });

  it.each(clean)("$name", (testCase) => {
    for (const language of LANGUAGES) expect(run.lookup(testCase.catalog, language)[testCase.key], language).toBe(expected(testCase, language));
  });
});

describe(`mixed-branch adoption matrix: ${conflicts.length} genuine conflicts`, () => {
  let run: ReturnType<typeof rehearse>;
  beforeAll(() => { run = rehearse(conflicts); }, 60_000);
  afterAll(() => fs.rmSync(run.repo, { recursive: true, force: true }));

  it("refuses with the exact diverged keys and writes nothing", () => {
    expect(run.command).toBe(`  npm run adopt:i18n -- --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
    expect(run.guided?.status).not.toBe(0);
    expect(run.guided?.stderr).toContain("No files written. Resolve these catalog divergences explicitly:");
    const reported = run.guided!.stderr.split("\n").filter((line) => line.includes("key diverged") || line.includes("changed differently"));
    const expectedLines = conflicts.flatMap((testCase) => LANGUAGES.map((language) => {
      const label = testCase.catalog === "app" && testCase.ops.some((op) => op.point === "branchAreas") ? `app split areas/${language}` : `${testCase.catalog}/${language}`;
      // The branch's monoliths and areas changing one key differently is reported as a contradiction.
      if (testCase.ops.some((op) => op.point === "branchX") && testCase.ops.some((op) => op.point === "branchAreas")) return `${label}: monoliths and split areas changed differently: ${testCase.key}`;
      return `${label}: ${testCase.conflict} key diverged: ${testCase.key}`;
    }));
    expect([...reported].sort()).toEqual([...expectedLines].sort());
    expect(run.monolithsLeft).toEqual(LANGUAGES);
  });

  it.each(conflicts)("$name", (testCase) => {
    expect(run.guided?.stderr).toMatch(new RegExp(`: (?:${testCase.conflict} key diverged|monoliths and split areas changed differently): ${testCase.key}\n`));
  });
});
