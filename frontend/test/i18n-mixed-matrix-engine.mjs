/**
 * Exhaustive matrix engine for the "mixed branch" adoption route: a legacy branch that merged main
 * while keeping the legacy monoliths, edited further, and finally merges main again, adopting with
 * the command the tool prints. Each case owns one key; one real-git history hosts every case of a
 * run. Expected outcomes come from an abstract model of the history (`simulate`), independent of
 * the tool, so the real-git rehearsal checks the tool against it.
 *
 * The rule the model encodes: a branch edit wins when the branch touched the key; otherwise main's
 * latest value is kept. Genuine contradictions are refused: main changed the key after the main
 * version the branch's edit was based on and ended differently, or the branch's monoliths and split
 * areas hold different edits. Each layout learns main's values at different merges: the monoliths
 * at legacy merges (their baseline is the last one, the legacy fork), the split areas at merges
 * that keep the monoliths, the unsplit family catalogs at every merge. Intermediate merges are
 * resolved key by key, with the branch's value winning a conflict (as a user would keep theirs).
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";

/**
 * Histories, as ordered steps: `main` edits commit on main, `split` runs the split on main,
 * `branch` edits commit on the branch, `merge` merges main into the branch (`keep`: keep the
 * monoliths beside the split areas; otherwise an ordinary legacy merge) with resolution edits at
 * `point`. Every history ends with the final merge of main, adopted by following the guidance.
 * Branch points named `*Areas*` edit the inherited split areas; other branch points edit the
 * monoliths. Main points edit whatever layout main has at that time.
 */
export const HISTORIES = {
  oneMerge: {
    title: "one merge keeping the monoliths (M), then X",
    steps: [
      { t: "main", point: "mainPreSplit" }, { t: "split" }, { t: "main", point: "mainBeforeM" },
      { t: "branch", points: ["branchBeforeM"] }, { t: "merge", keep: true, point: "resolutionM" },
      { t: "branch", points: ["branchX", "branchAreas"] }, { t: "main", point: "mainAfterM" },
    ],
  },
  twoKeepMerges: {
    title: "two merges keeping the monoliths (M1, M2) with edits between, then X",
    steps: [
      { t: "main", point: "mainPreSplit" }, { t: "split" }, { t: "main", point: "mainBeforeM1" },
      { t: "branch", points: ["branchBeforeM1"] }, { t: "merge", keep: true, point: "resolutionM1" },
      { t: "branch", points: ["branchBetween", "branchAreasBetween"] }, { t: "main", point: "mainBetween" },
      { t: "merge", keep: true, point: "resolutionM2" }, { t: "branch", points: ["branchX", "branchAreas"] },
      { t: "main", point: "mainAfterM" },
    ],
  },
  splitBetweenMerges: {
    title: "a legacy merge (M1), the split on main, a merge keeping the monoliths (M2), then X",
    steps: [
      { t: "main", point: "mainBeforeM1" }, { t: "branch", points: ["branchBeforeM1"] },
      { t: "merge", keep: false, point: "resolutionM1" }, { t: "branch", points: ["branchBetween"] },
      { t: "main", point: "mainBeforeSplit" }, { t: "split" }, { t: "main", point: "mainAfterSplit" },
      { t: "merge", keep: true, point: "resolutionM2" }, { t: "branch", points: ["branchX", "branchAreas"] },
      { t: "main", point: "mainAfterM" },
    ],
  },
};

const KINDS = ["add", "change", "remove"];
const isAreaPoint = (name) => /Areas/.test(name);

/** Edit points in time order, with their side and layout. */
export function historyPoints(history) {
  return history.steps.flatMap((step) => step.t === "main" ? [{ name: step.point, side: "main" }]
    : step.t === "branch" ? step.points.map((name) => ({ name, side: "branch", areas: isAreaPoint(name) }))
      : step.t === "merge" ? [{ name: step.point, side: "branch", areas: false }] : []);
}

const merge3 = (base, ours, theirs) => ours === base ? theirs : ours;
/** Three-way outcome of one value: a branch change wins unless main also changed it differently. */
const threeWay = (base, branch, main) => branch === base ? { value: main }
  : main === base || main === branch ? { value: branch } : { conflict: true };

/**
 * The model: the final value tag of a case (`"init"`, an op tag, or undefined for absent), or
 * `{ conflict: true }`. Throws "invalid" for an edit the history does not allow (changing or
 * removing an absent key, adding a present one, editing areas before the branch has any).
 */
export function simulate(history, testCase) {
  const init = testCase.present ? "init" : undefined;
  let main = init, monolith = init, areas, hasAreas = false, legacyBase = init, areaBase, family = init, familyBase = init;
  const apply = (op, current) => {
    if (op.kind === "add" ? current !== undefined : current === undefined) throw "invalid";
    return op.kind === "remove" ? undefined : op.tag ?? op.point;
  };
  const edit = (point) => {
    for (const op of testCase.ops.filter((candidate) => candidate.point === point.name)) {
      if (point.side === "main") main = apply(op, main);
      else if (testCase.catalog !== "app") { if (point.areas) throw "invalid"; family = apply(op, family); }
      else if (point.areas) { if (!hasAreas) throw "invalid"; areas = apply(op, areas); }
      else monolith = apply(op, monolith);
    }
  };
  for (const step of history.steps) {
    if (step.t === "main") edit({ name: step.point, side: "main" });
    else if (step.t === "branch") for (const name of step.points) edit({ name, side: "branch", areas: isAreaPoint(name) });
    else if (step.t === "merge") {
      family = merge3(familyBase, family, main); familyBase = main;
      if (step.keep) { areas = hasAreas ? merge3(areaBase, areas, main) : main; areaBase = main; hasAreas = true; }
      else { monolith = merge3(legacyBase, monolith, main); legacyBase = main; }
      edit({ name: step.point, side: "branch", areas: false });
    }
  }
  if (testCase.catalog !== "app") return threeWay(familyBase, family, main);
  const monolithChanged = monolith !== legacyBase;
  const areasChanged = areas !== areaBase;
  if (monolithChanged && areasChanged && monolith !== areas) return { conflict: true };
  if (areasChanged) return threeWay(areaBase, areas, main);
  if (monolithChanged) return threeWay(legacyBase, monolith, main);
  return { value: main };
}

function* choose(list, count, start = 0) {
  if (!count) { yield []; return; }
  for (let index = start; index < list.length; index++) for (const rest of choose(list, count - 1, index + 1)) yield [list[index], ...rest];
}
function* kindSequences(count) {
  if (!count) { yield []; return; }
  for (const kind of KINDS) for (const rest of kindSequences(count - 1)) yield [kind, ...rest];
}

/**
 * Every valid case of a history: each single edit; every ordered pair; every sequence of three
 * edits across main and the branch; every alternating main/branch sequence of four; and the same
 * add/change/remove made on two sides (main and branch, or the branch's monoliths and areas).
 */
export function buildCases(history, catalogs) {
  const points = historyPoints(history);
  const cases = [];
  const consider = (catalog, ops, group) => {
    for (const present of [true, false]) {
      const testCase = { catalog, present, ops, group };
      try { cases.push({ ...testCase, outcome: simulate(history, testCase) }); } catch (error) { if (error !== "invalid") throw error; }
    }
  };
  for (const catalog of catalogs) {
    for (const count of [1, 2, 3, 4]) {
      for (const selection of choose(points, count)) {
        const sides = new Set(selection.map((point) => point.side));
        if (count >= 3 && sides.size < 2) continue;
        if (count === 4 && !selection.every((point, index) => !index || point.side !== selection[index - 1].side)) continue;
        for (const kinds of kindSequences(count)) consider(catalog, selection.map((point, index) => ({ point: point.name, kind: kinds[index] })), count === 1 ? "single" : `${count} edits`);
      }
    }
    for (const [first, second] of choose(points, 2)) {
      if (first.side === second.side && !(first.side === "branch" && first.areas !== second.areas)) continue;
      for (const kind of KINDS) consider(catalog, [{ point: first.name, kind, tag: "same" }, { point: second.name, kind, tag: "same" }], "same change");
    }
  }
  const named = (list, prefix) => list.map((testCase, index) => {
    const key = `${prefix}${index}`;
    const ops = testCase.ops.map((op) => `${op.kind} ${op.point}${op.tag ? `(${op.tag})` : ""}`).join(" then ");
    const outcome = testCase.outcome.conflict ? "refused" : testCase.outcome.value === undefined ? "absent" : `${testCase.outcome.value} kept`;
    return { ...testCase, key, name: `${testCase.catalog} ${key} [${testCase.group}]: ${testCase.present ? "present" : "absent"}, ${ops} -> ${outcome}` };
  });
  return {
    clean: named(cases.filter((testCase) => !testCase.outcome.conflict), "k"),
    conflicts: named(cases.filter((testCase) => testCase.outcome.conflict), "c"),
  };
}

/** The expected string value of a clean case in one language (undefined when absent). */
export const expectedValue = (testCase, language) => testCase.outcome.value === undefined ? undefined
  : testCase.outcome.value === "init" ? `init-${testCase.key} ${language}` : `${testCase.outcome.value}-${testCase.key} ${language}`;

const canonical = (object) => `${JSON.stringify(Object.fromEntries(Object.keys(object).sort((a, b) => a < b ? -1 : a > b ? 1 : 0).map((key) => [key, object[key]])), null, 2)}\n`;

/**
 * Build the history in a fresh repository and adopt by following the tool's printed guidance.
 * `pkg`: `{ name, source, copy: [[from, to]], adoptScript, adoptCommand, languages, families, areaRoot }`
 * with paths relative to the package (`areaRoot`: the directory holding `<area>/<language>.json`).
 */
export function rehearse(history, cases, pkg) {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), `nyxid-${pkg.name}-i18n-matrix-`));
  const root = path.join(repo, pkg.name);
  const locales = path.join(root, "src/i18n/locales");
  const areaRoot = path.join(root, pkg.areaRoot);
  const git = (...args) => execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 1 << 28 }).trim();
  const tryGit = (...args) => { try { return git(...args); } catch { return null; } };
  const write = (file, text) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); };
  const read = (file) => fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {};
  const adopt = (...args) => spawnSync(process.execPath, [path.join(root, pkg.adoptScript), ...args], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  const relative = (file) => path.relative(repo, file).split(path.sep).join("/");
  const monolithFile = (language) => path.join(locales, `${language}.json`);
  const familyFile = (family, language) => path.join(locales, `${family}-${language}.json`);
  const areaFiles = (language) => fs.existsSync(areaRoot) ? fs.readdirSync(areaRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory())
    .map((entry) => path.join(areaRoot, entry.name, `${language}.json`)).filter((file) => fs.existsSync(file)) : [];
  const commit = (message) => { git("add", "-A", pkg.name); git("commit", "-q", "--allow-empty", "-m", message); return git("rev-parse", "HEAD"); };
  const value = (key, op, language) => `${op.tag ?? op.point}-${key} ${language}`;

  /** Apply each case's edits at `point` (`side`: main or branch) to the layout that point writes. */
  const edit = (point, side) => {
    for (const language of pkg.languages) {
      const files = new Map();
      const load = (file) => { if (!files.has(file)) files.set(file, read(file)); return files.get(file); };
      for (const testCase of cases) {
        for (const op of testCase.ops.filter((candidate) => candidate.point === point)) {
          let file;
          if (testCase.catalog !== "app") file = familyFile(testCase.catalog, language);
          else if (side === "main" ? fs.existsSync(monolithFile(language)) : !isAreaPoint(point)) file = monolithFile(language);
          else file = areaFiles(language).find((candidate) => Object.hasOwn(load(candidate), testCase.key)) ?? path.join(areaRoot, "unattributed", `${language}.json`);
          const catalog = load(file);
          if (op.kind === "remove") delete catalog[testCase.key];
          else catalog[testCase.key] = value(testCase.key, op, language);
        }
      }
      for (const [file, catalog] of files) write(file, canonical(catalog));
    }
  };
  const show = (ref, file) => { const text = tryGit("show", `${ref}:${relative(file)}`); return text === null ? null : JSON.parse(text); };
  const merged3 = (base, ours, theirs) => {
    const result = { ...ours };
    for (const key of new Set([...Object.keys(base), ...Object.keys(theirs)])) {
      if (base[key] === theirs[key] || ours[key] !== base[key]) continue; // unchanged by main, or the branch's value wins
      if (Object.hasOwn(theirs, key)) result[key] = theirs[key]; else delete result[key];
    }
    return result;
  };
  /** Area catalogs of one language at a ref, as [file, catalog] pairs. */
  const areasAt = (ref, language) => (tryGit("ls-tree", "-r", "--name-only", ref, "--", relative(areaRoot)) ?? "").split("\n")
    .filter((file) => file.endsWith(`/${language}.json`) && path.posix.dirname(path.posix.dirname(file)) === relative(areaRoot))
    .map((file) => [path.join(repo, file), JSON.parse(git("show", `${ref}:${file}`))]);

  for (const [from, to] of pkg.copy) write(path.join(repo, to), fs.readFileSync(from, "utf8"));
  write(path.join(root, "package.json"), '{"type":"module"}\n');
  fs.symlinkSync(path.join(pkg.source, "node_modules"), path.join(root, "node_modules"));
  write(path.join(repo, ".gitignore"), "node_modules\n");
  git("init", "-q", "-b", "main");
  for (const [name, setting] of [["user.name", "matrix"], ["user.email", "matrix@example.test"], ["core.hooksPath", "/dev/null"], ["commit.gpgsign", "false"]]) git("config", name, setting);
  for (const language of pkg.languages) {
    for (const catalog of ["app", ...pkg.families]) {
      const entries = cases.filter((testCase) => testCase.catalog === catalog && testCase.present).map((testCase) => [testCase.key, `init-${testCase.key} ${language}`]);
      write(catalog === "app" ? monolithFile(language) : familyFile(catalog, language), canonical(Object.fromEntries(entries)));
    }
  }
  let fork = commit("F: legacy fork");
  git("branch", "legacy");
  for (const step of history.steps) {
    if (step.t === "main") { git("checkout", "-q", "main"); edit(step.point, "main"); commit(`main: ${step.point}`); }
    else if (step.t === "split") {
      git("checkout", "-q", "main");
      const split = adopt("--branch", "HEAD", "--base", "empty", "--onto", "HEAD");
      if (split.status !== 0) throw new Error(split.stderr);
      commit("main: split");
    } else if (step.t === "branch") {
      git("checkout", "-q", "legacy");
      for (const point of step.points) edit(point, "branch");
      commit(`branch: ${step.points.join(", ")}`);
    } else if (step.t === "merge") {
      git("checkout", "-q", "legacy");
      const ours = git("rev-parse", "HEAD");
      const theirs = git("rev-parse", "main");
      const base = git("merge-base", ours, theirs);
      spawnSync("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo, encoding: "utf8" });
      // A correct resolution, key by key, with the branch's value winning a conflict.
      const catalogFiles = [...pkg.families.flatMap((family) => pkg.languages.map((language) => familyFile(family, language))),
        ...(step.keep ? [] : pkg.languages.map(monolithFile))];
      for (const file of catalogFiles) write(file, canonical(merged3(show(base, file) ?? {}, show(ours, file) ?? {}, show(theirs, file) ?? {})));
      if (step.keep) {
        git("checkout", ours, "--", ...pkg.languages.map((language) => relative(monolithFile(language))));
        for (const language of pkg.languages) {
          const flatten = (pairs) => Object.assign({}, ...pairs.map(([, catalog]) => catalog));
          const [baseAreas, ourAreas, theirAreas] = [areasAt(base, language), areasAt(ours, language), areasAt(theirs, language)];
          const result = merged3(flatten(baseAreas), flatten(ourAreas), flatten(theirAreas));
          const owner = new Map([...theirAreas, ...ourAreas].flatMap(([file, catalog]) => Object.keys(catalog).map((key) => [key, file])));
          for (const file of areaFiles(language)) fs.rmSync(file);
          const byFile = new Map();
          for (const [key, text] of Object.entries(result)) {
            const file = owner.get(key) ?? path.join(areaRoot, "unattributed", `${language}.json`);
            if (!byFile.has(file)) byFile.set(file, {});
            byFile.get(file)[key] = text;
          }
          for (const [file, catalog] of byFile) write(file, canonical(catalog));
        }
      } else fork = theirs;
      edit(step.point, "branch");
      commit(`branch: merge main${step.keep ? ", keeping the monoliths" : ""}`);
    }
  }
  git("checkout", "-q", "legacy");
  spawnSync("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo, encoding: "utf8" });
  // Follow the tool exactly as a user would: run adopt, then the command it prints.
  const refused = adopt();
  const command = refused.stderr.split("\n").find((line) => line.startsWith(`  ${pkg.adoptCommand} --branch`));
  const guided = command ? spawnSync("sh", ["-c", command.trim().replace(pkg.adoptCommand, `"${process.execPath}" ${pkg.adoptScript}`)], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 }) : null;
  /** Read lazily: after a refusal the final merge's conflicted catalogs are left in the working tree. */
  const lookup = (catalog, language) => catalog === "app" ? Object.assign({}, ...areaFiles(language).map(read)) : read(familyFile(catalog, language));
  const duplicates = (language) => {
    const seen = new Map();
    for (const file of areaFiles(language)) for (const key of Object.keys(read(file))) seen.set(key, (seen.get(key) ?? 0) + 1);
    return [...seen].filter(([, count]) => count > 1).map(([key]) => key);
  };
  const monolithsLeft = pkg.languages.filter((language) => fs.existsSync(monolithFile(language)));
  /** Keys reported as refused, per language. */
  const refusedKeys = (language) => (guided?.stderr ?? "").split("\n")
    .filter((line) => line.includes(`/${language}: `)).map((line) => line.slice(line.lastIndexOf(": ") + 2));
  return { repo, fork, refused, command, guided, lookup, duplicates, monolithsLeft, refusedKeys, cleanup: () => fs.rmSync(repo, { recursive: true, force: true }) };
}
