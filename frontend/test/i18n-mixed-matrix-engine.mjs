/**
 * Matrix engine for adoption of legacy-layout branches, including "mixed" branches that merged the
 * split main while keeping the legacy monoliths. Each case owns one key (or a plural family); one
 * real-git history hosts every case of a run. Expected outcomes come from an abstract model of the
 * history (`simulateCase`), independent of the tool; the rehearsal builds the history in git, adopts
 * it the way a user would (following any command the tool prints) and is checked against the model.
 *
 * The rule the model encodes (decided from first principles: never lose, resurrect or revert data,
 * and never refuse falsely):
 * - Layouts learn main's values at different merges: the monoliths at legacy merges, the split areas
 *   at merges that keep the monoliths (or never, if the branch made its own areas), the unsplit
 *   family catalogs at every merge. Each layout's edits are measured from its reference: on a merge
 *   route the latest such reconciliation; on a cherry-pick or rebase, the parent commit.
 * - A branch edit wins if the branch changed the key; otherwise main's latest value is kept. A key is
 *   refused when main changed it since the branch's reference and ended differently, or when the
 *   branch's monoliths and split areas changed it differently. A split-area edit takes precedence
 *   over a monolith edit of the same key (it was made against the newer main).
 * - A conflict at an intermediate merge is resolved the way the case says: keep the branch's value,
 *   take main's, or write a third value. Edits that touch one language only are modelled per language.
 * - Placement: onto's area for a key (or its plural sibling); else the branch's area for a key that
 *   arrived through its split-area edits (or its sibling's); else literal usage (unattributed here,
 *   since no source references the keys). Main adds new keys to "billing", the branch to "settings",
 *   unless a plural sibling already lives elsewhere.
 * - A mixed branch whose split areas have no reference (it made them itself, or onto shares no history
 *   with it) is refused as a whole, with no command printed.
 */
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";

/* ------------------------------------------------------------------ histories */

/**
 * Steps: `main` (edit commit on main at `point`), `split`, `branch` (commit with edits at `points`;
 * names containing "Areas" edit the split areas, others the monoliths), `merge` (merge main into the
 * branch: `keep` keeps the monoliths beside the split areas, otherwise a legacy merge; resolution
 * edits at `point`; `id` names it for resolution policies), `localAreas` (the branch creates its own
 * split areas from its monoliths). `route`: "merge" (merge main again and adopt, following any
 * printed command), "cherryPick" (cherry-pick the last branch commit onto main), "rebase" (rebase
 * the last `count` branch commits onto main, adopting at each stop), "unrelatedOnto" (adopt with an
 * --onto that shares no history with the branch).
 */
export const HISTORIES = {
  oneMerge: {
    title: "one merge keeping the monoliths (M), then X",
    route: "merge",
    steps: [
      { t: "main", point: "mainPreSplit" }, { t: "split" }, { t: "main", point: "mainBeforeM" },
      { t: "branch", points: ["branchBeforeM"] }, { t: "merge", keep: true, point: "resolutionM", id: "M" },
      { t: "branch", points: ["branchX", "branchAreas"] }, { t: "main", point: "mainAfterM" },
    ],
  },
  twoKeepMerges: {
    title: "two merges keeping the monoliths (M1, M2) with edits between, then X",
    route: "merge",
    steps: [
      { t: "main", point: "mainPreSplit" }, { t: "split" }, { t: "main", point: "mainBeforeM1" },
      { t: "branch", points: ["branchBeforeM1"] }, { t: "merge", keep: true, point: "resolutionM1", id: "M1" },
      { t: "branch", points: ["branchBetween", "branchAreasBetween"] }, { t: "main", point: "mainBetween" },
      { t: "merge", keep: true, point: "resolutionM2", id: "M2" }, { t: "branch", points: ["branchX", "branchAreas"] },
      { t: "main", point: "mainAfterM" },
    ],
  },
  splitBetweenMerges: {
    title: "a legacy merge (M1), the split on main, a merge keeping the monoliths (M2), then X",
    route: "merge",
    steps: [
      { t: "main", point: "mainBeforeM1" }, { t: "branch", points: ["branchBeforeM1"] },
      { t: "merge", keep: false, point: "resolutionM1", id: "M1" }, { t: "branch", points: ["branchBetween"] },
      { t: "main", point: "mainBeforeSplit" }, { t: "split" }, { t: "main", point: "mainAfterSplit" },
      { t: "merge", keep: true, point: "resolutionM2", id: "M2" }, { t: "branch", points: ["branchX", "branchAreas"] },
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

/** Run-level facts: whether the branch ends mixed, whether its areas lack a reference (N9). */
export function historyFacts(history) {
  let keep = 0, local = false;
  for (const step of history.steps) {
    if (step.t === "merge" && step.keep) keep++;
    if (step.t === "localAreas") local = true;
  }
  const mixed = keep > 0 || local;
  const unreferencedAreas = mixed && (history.route === "unrelatedOnto" || (local && keep === 0 && history.route === "merge"));
  return { mixed, unreferencedAreas };
}

/* ------------------------------------------------------------------ model */

const formsOf = (testCase) => testCase.plural ? ["one", "other"] : [""];
export const keyName = (testCase, form) => form ? `${testCase.key}_${form}` : testCase.key;
const valueOf = (tag, key, language) => `${tag}-${key} ${language}`;
const siblingOf = (key, keys) => keys.find((other) => other !== key);

/**
 * Simulate one case through the history. Returns `{ values: { [language]: { [key]: string|undefined } },
 * owners: { [key]: area }, conflicts: { [language]: key[] }, conflictAt }`, where `conflictAt` is the
 * index of the adopted commit that refuses (0 for merge and cherry-pick routes; the commit index for
 * rebase) or null. Throws "invalid" for an edit the history does not allow.
 */
export function simulateCase(history, testCase, languages) {
  const keys = formsOf(testCase).map((form) => keyName(testCase, form));
  const blank = () => Object.fromEntries(languages.map((language) => [language, {}]));
  const copy = (layer) => Object.fromEntries(languages.map((language) => [language, { ...layer[language] }]));
  const initial = blank();
  if (testCase.present) for (const language of languages) for (const key of keys) initial[language][key] = valueOf("init", key, language);
  const app = testCase.catalog === "app";
  let main = copy(initial), monolith = copy(initial), areas = blank(), family = copy(initial);
  let legacyBase = copy(initial), areaBase = null, familyBase = copy(initial);
  let hasAreas = false, split = false;
  let mainOwner = {}, branchOwner = {};
  const branchStates = [];
  const present = (layer, key) => layer[languages[0]][key] !== undefined;
  const apply = (layer, op, ownerOf, defaultOwner) => {
    const opKeys = op.forms ? op.forms.map((form) => keyName(testCase, form)) : keys;
    const opLanguages = op.only ? [op.only] : languages;
    if (op.only && op.kind !== "change") throw "invalid";
    for (const key of opKeys) {
      for (const language of opLanguages) {
        const current = layer[language][key];
        if (op.kind === "add" ? current !== undefined : current === undefined) throw "invalid";
        if (op.kind === "remove") delete layer[language][key];
        else layer[language][key] = valueOf(op.tag ?? op.point, key, language);
      }
      if (ownerOf) {
        if (op.kind === "remove") delete ownerOf[key];
        else if (op.kind === "add" && defaultOwner) {
          const sibling = siblingOf(key, keys);
          ownerOf[key] = sibling && present(layer, sibling) && ownerOf[sibling] ? ownerOf[sibling] : defaultOwner;
        }
      }
    }
  };
  const editsAt = (point) => testCase.ops.filter((op) => op.point === point);
  /** Merge resolution as a user would do it: per key, and a conflict in any language resolves the whole key by the policy. */
  const merge3 = (base, ours, theirs, mergeId) => {
    const policy = testCase.resolutions?.[mergeId] ?? "ours";
    const result = blank();
    for (const key of keys) {
      const conflict = languages.some((language) => {
        const [b, o, t] = [base[language][key], ours[language][key], theirs[language][key]];
        return o !== b && t !== b && t !== o;
      });
      for (const language of languages) {
        const [b, o, t] = [base[language][key], ours[language][key], theirs[language][key]];
        const value = !conflict ? (o === b ? t : o) : policy === "ours" ? o : policy === "theirs" ? t : valueOf(`third${mergeId}`, key, language);
        if (value !== undefined) result[language][key] = value;
      }
    }
    return result;
  };
  const record = () => branchStates.push({ monolith: copy(monolith), areas: copy(areas), hasAreas, family: copy(family), branchOwner: { ...branchOwner } });
  record(); // F, the fork: the parent of the first branch commit
  for (const step of history.steps) {
    if (step.t === "main") {
      for (const op of editsAt(step.point)) apply(main, op, split && app ? mainOwner : null, split && app ? "billing" : null);
    } else if (step.t === "split") {
      split = true;
      mainOwner = Object.fromEntries(keys.filter((key) => present(main, key)).map((key) => [key, "unattributed"]));
    } else if (step.t === "branch") {
      for (const point of step.points) {
        for (const op of editsAt(point)) {
          if (!app) { if (isAreaPoint(point)) throw "invalid"; apply(family, op); }
          else if (isAreaPoint(point)) { if (!hasAreas) throw "invalid"; apply(areas, op, branchOwner, "settings"); }
          else apply(monolith, op);
        }
      }
      record();
    } else if (step.t === "localAreas") {
      areas = copy(monolith); hasAreas = true; areaBase = null;
      branchOwner = Object.fromEntries(keys.filter((key) => present(areas, key)).map((key) => [key, "settings"]));
      record();
    } else if (step.t === "merge") {
      family = merge3(familyBase, family, main, step.id); familyBase = copy(main);
      if (step.keep) {
        const before = copy(areas);
        areas = hasAreas ? merge3(areaBase ?? blank(), areas, main, step.id) : copy(main);
        const owners = {};
        for (const key of keys.filter((candidate) => present(areas, candidate))) {
          const sibling = siblingOf(key, keys);
          owners[key] = present(before, key) && branchOwner[key] ? branchOwner[key]
            : sibling && present(before, sibling) && branchOwner[sibling] ? branchOwner[sibling] : mainOwner[key];
        }
        branchOwner = owners; areaBase = copy(main); hasAreas = true;
      } else {
        monolith = merge3(legacyBase, monolith, main, step.id); legacyBase = copy(main);
      }
      for (const op of editsAt(step.point)) app ? apply(monolith, op) : apply(family, op);
      record();
    }
  }
  const current = { monolith, areas, hasAreas, family, branchOwner };

  /** Adopt `branch` against references `ref` onto `onto`; returns { values, owners, conflicts }. */
  const adopt = (ref, branch, onto, ontoOwner) => {
    const values = blank(), conflicts = {};
    const areaChanged = new Set();
    const threeWay = (base, mine, theirs) => mine === base ? { value: theirs } : theirs === base || theirs === mine ? { value: mine } : { conflict: true };
    for (const language of languages) {
      for (const key of keys) {
        let outcome;
        if (!app) outcome = threeWay(ref.family[language][key], branch.family[language][key], onto[language][key]);
        else {
          // N14: a monolith change equal to the area reference is main's own change, not a branch edit.
          const monolithChanged = ref.monolith[language][key] !== branch.monolith[language][key]
            && !(branch.hasAreas && branch.monolith[language][key] === ref.areas[language][key]);
          const areasChanged = branch.hasAreas && ref.areas[language][key] !== branch.areas[language][key];
          if (areasChanged) areaChanged.add(key);
          outcome = monolithChanged && areasChanged && branch.monolith[language][key] !== branch.areas[language][key] ? { conflict: true }
            : areasChanged ? threeWay(ref.areas[language][key], branch.areas[language][key], onto[language][key])
              : monolithChanged ? threeWay(ref.monolith[language][key], branch.monolith[language][key], onto[language][key])
                : { value: onto[language][key] };
        }
        if (outcome.conflict) (conflicts[language] ??= []).push(key);
        else if (outcome.value !== undefined) values[language][key] = outcome.value;
      }
    }
    const owners = {};
    if (app) {
      const preferred = (key) => areaChanged.has(key) && branch.hasAreas && branch.areas[languages[0]][key] !== undefined ? branch.branchOwner[key] : undefined;
      for (const key of keys.filter((candidate) => values[languages[0]][candidate] !== undefined)) {
        const sibling = siblingOf(key, keys);
        owners[key] = (present(onto, key) ? ontoOwner[key] : undefined)
          ?? (sibling && present(onto, sibling) ? ontoOwner[sibling] : undefined)
          ?? preferred(key) ?? (sibling ? preferred(sibling) : undefined) ?? "unattributed";
      }
    }
    return { values, owners, conflicts };
  };

  if (history.route === "cherryPick" || history.route === "rebase") {
    const count = history.route === "rebase" ? history.count : 1;
    let onto = copy(main), ontoOwner = { ...mainOwner };
    for (let index = 0; index < count; index++) {
      const position = branchStates.length - count + index;
      const result = adopt(branchStates[position - 1], branchStates[position], onto, ontoOwner);
      if (Object.keys(result.conflicts).length) return { ...result, conflictAt: index };
      onto = result.values; ontoOwner = result.owners;
    }
    return { values: onto, owners: ontoOwner, conflicts: {}, conflictAt: null };
  }
  const reference = { monolith: legacyBase, areas: areaBase ?? blank(), family: familyBase };
  const result = adopt(reference, current, main, mainOwner);
  return { ...result, conflictAt: Object.keys(result.conflicts).length ? 0 : null };
}

/* ------------------------------------------------------------------ case generation */

const simulated = (history, testCase, languages) => {
  try { return simulateCase(history, testCase, languages); } catch (error) { if (error === "invalid") return null; throw error; }
};
const describeOp = (op) => `${op.kind}${op.forms ? `[${op.forms.join("+")}]` : ""}${op.only ? `[${op.only} only]` : ""} ${op.point}${op.tag ? `(${op.tag})` : ""}`;
const nameCase = (testCase, label, prefix = "") => {
  const ops = testCase.ops.map(describeOp).join(" then ");
  const policies = testCase.resolutions ? ` {${Object.entries(testCase.resolutions).map(([id, policy]) => `${id}:${policy}`).join(", ")}}` : "";
  const outcome = testCase.model.conflictAt !== null ? `refused${testCase.model.conflictAt ? ` at commit ${testCase.model.conflictAt + 1}` : ""}` : "adopted";
  return `${prefix}${testCase.catalog === "app" ? label : testCase.catalog} ${testCase.key}${testCase.plural ? " (plural)" : ""} [${testCase.group}]: ${testCase.present ? "present" : "absent"}, ${ops}${policies} -> ${outcome}`;
};

function* choose(list, count, start = 0) {
  if (!count) { yield []; return; }
  for (let index = start; index < list.length; index++) for (const rest of choose(list, count - 1, index + 1)) yield [list[index], ...rest];
}
function* kindSequences(count) {
  if (!count) { yield []; return; }
  for (const kind of KINDS) for (const rest of kindSequences(count - 1)) yield [kind, ...rest];
}

/**
 * Every valid case of a fixed history: each single edit; every ordered pair; every sequence of three
 * edits across main and the branch; every alternating main/branch sequence of four; and the same
 * add/change/remove made on two sides (main and branch, or the branch's monoliths and areas).
 */
export function buildCases(history, catalogs, languages, { splitLabel = "app" } = {}) {
  const points = historyPoints(history);
  const cases = [];
  const consider = (catalog, ops, group) => {
    for (const present of [true, false]) {
      const testCase = { catalog, present, ops, group, key: "" };
      const model = simulated(history, { ...testCase, key: "x" }, languages);
      if (model) cases.push(testCase);
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
  return splitByOutcome(history, cases, languages, (testCase) => nameCase(testCase, splitLabel));
}

function splitByOutcome(history, cases, languages, name) {
  const keyed = (list, prefix) => list.map((testCase, index) => {
    const key = `${prefix}${index}`;
    const model = simulateCase(history, { ...testCase, key }, languages);
    const result = { ...testCase, key, model };
    return { ...result, name: name(result) };
  });
  const all = cases.map((testCase) => ({ testCase, conflict: simulateCase(history, { ...testCase, key: "x" }, languages).conflictAt !== null }));
  return { clean: keyed(all.filter((entry) => !entry.conflict).map((entry) => entry.testCase), "k"), conflicts: keyed(all.filter((entry) => entry.conflict).map((entry) => entry.testCase), "c") };
}

/** Deterministic PRNG (mulberry32). */
export function random(seed) {
  let state = seed >>> 0;
  const next = () => {
    state = (state + 0x6D2B79F5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  return { next, int: (low, high) => low + Math.floor(next() * (high - low + 1)), pick: (list) => list[Math.floor(next() * list.length)], chance: (p) => next() < p };
}

/**
 * A seeded random history: 0-3 legacy merges, the split, 0-4 merges keeping the monoliths, branch
 * commits and main edits in between, and a final route (merge, cherry-pick or rebase). Some
 * histories instead exercise an unreferenced split-area layout (the branch made its own areas, or
 * --onto shares no history with it).
 */
export function randomHistory(seed) {
  const rand = random(seed);
  const steps = [];
  let mainIndex = 0, branchIndex = 0, mergeIndex = 0, keep = 0;
  const mainEdit = () => steps.push({ t: "main", point: `m${++mainIndex}` });
  const branchCommit = () => {
    branchIndex++;
    steps.push({ t: "branch", points: keep ? [`b${branchIndex}`, `b${branchIndex}Areas`] : [`b${branchIndex}`] });
  };
  // Every tenth seed exercises each unreferenced-areas variant; routes cycle so each appears.
  const variant = seed % 10 === 3 ? "localAreas" : seed % 10 === 7 ? "unrelatedOnto" : "ordinary";
  if (rand.chance(0.7)) mainEdit();
  branchCommit();
  for (let index = rand.int(0, variant === "localAreas" ? 1 : 3); index > 0; index--) {
    if (rand.chance(0.7)) mainEdit();
    if (rand.chance(0.6)) branchCommit();
    mergeIndex++;
    steps.push({ t: "merge", keep: false, point: `r${mergeIndex}`, id: `M${mergeIndex}` });
  }
  if (rand.chance(0.6)) mainEdit();
  steps.push({ t: "split" });
  if (variant === "localAreas") {
    if (rand.chance(0.5)) branchCommit();
    steps.push({ t: "localAreas" });
    if (rand.chance(0.6)) mainEdit();
    return { title: `random seed ${seed}: the branch made its own split areas`, seed, route: "merge", steps };
  }
  const keeps = variant === "unrelatedOnto" ? rand.int(1, 2) : rand.int(0, 4);
  for (let index = 0; index < keeps; index++) {
    if (rand.chance(0.7)) mainEdit();
    if (rand.chance(0.6)) branchCommit();
    mergeIndex++; keep++;
    steps.push({ t: "merge", keep: true, point: `r${mergeIndex}`, id: `M${mergeIndex}` });
  }
  const trailing = rand.int(1, 3);
  for (let index = 0; index < trailing; index++) branchCommit();
  const route = ["merge", "cherryPick", "rebase", "merge"][seed % 4];
  // A merge route needs main to have moved since the branch's last merge, or there is nothing to adopt.
  if (route === "merge" || rand.chance(0.8)) mainEdit();
  if (variant === "unrelatedOnto") return { title: `random seed ${seed}: --onto shares no history with the branch`, seed, route: "unrelatedOnto", steps };
  const count = route === "rebase" ? Math.min(trailing, seed % 8 === 2 ? 2 : rand.int(1, 2)) : undefined;
  const description = `${steps.filter((step) => step.t === "merge" && !step.keep).length} legacy and ${keep} keep-monolith merges, ${route}${count ? ` of ${count}` : ""}`;
  return { title: `random seed ${seed}: ${description}`, seed, route, count, steps };
}

/**
 * Random cases for a history: 1-7 edits at distinct points in time order, random kinds, some
 * one-language changes, some plural families (editing one or both forms), and a random resolution
 * policy for each merge. Every branch commit also gets a sentinel key added in its monoliths, so a
 * cherry-pick or rebase of it always stops for adoption.
 */
export function randomCases(history, catalogs, languages, { clean: cleanTarget, conflicts: conflictTarget, splitLabel = "app" }) {
  const rand = random(history.seed * 7919 + 17);
  const points = historyPoints(history);
  const mergeIds = history.steps.filter((step) => step.t === "merge").map((step) => step.id);
  const accepted = [], refused = [];
  for (let attempt = 0; attempt < 4000 && (accepted.length < cleanTarget || refused.length < conflictTarget); attempt++) {
    const catalog = rand.pick(catalogs);
    const plural = catalog === "app" && rand.chance(0.25);
    const length = rand.pick([1, 2, 3, 3, 4, 5, 5, 6, 7]);
    const shuffled = [...points];
    for (let index = shuffled.length - 1; index > 0; index--) { const other = rand.int(0, index); [shuffled[index], shuffled[other]] = [shuffled[other], shuffled[index]]; }
    const chosen = shuffled.slice(0, length).sort((a, b) => points.indexOf(a) - points.indexOf(b));
    const ops = chosen.map((point) => {
      const kind = rand.pick(KINDS);
      const op = { point: point.name, kind };
      if (kind === "change" && rand.chance(0.25)) op.only = rand.pick(languages);
      if (plural && rand.chance(0.4)) op.forms = [rand.pick(["one", "other"])];
      return op;
    });
    const resolutions = mergeIds.length && rand.chance(0.7) ? Object.fromEntries(mergeIds.map((id) => [id, rand.pick(["ours", "ours", "theirs", "third"])])) : undefined;
    const testCase = { catalog, plural, present: rand.chance(0.6), ops, resolutions, group: "random", key: "x" };
    const model = simulated(history, testCase, languages);
    if (!model) continue;
    if (model.conflictAt === null ? accepted.length < cleanTarget : refused.length < conflictTarget) (model.conflictAt === null ? accepted : refused).push(testCase);
  }
  const sentinels = history.steps.filter((step) => step.t === "branch").map((step) => ({ catalog: "app", present: false, ops: [{ point: step.points[0], kind: "add" }], group: "sentinel", key: "x" }));
  const prefix = `seed ${history.seed} `;
  const name = (testCase) => nameCase(testCase, splitLabel, prefix);
  const clean = splitByOutcome(history, [...accepted, ...sentinels], languages, name).clean;
  const conflicts = splitByOutcome(history, refused, languages, name).conflicts;
  return { clean, conflicts, sentinels: clean.filter((testCase) => testCase.group === "sentinel") };
}

/* ------------------------------------------------------------------ real git */

const exec = (command, args, options) => new Promise((resolve) => {
  execFile(command, args, { maxBuffer: 1 << 28, ...options }, (error, stdout, stderr) => resolve({ status: error ? (typeof error.code === "number" ? error.code : 1) : 0, stdout, stderr }));
});
const canonical = (object) => `${JSON.stringify(Object.fromEntries(Object.keys(object).sort((a, b) => a < b ? -1 : a > b ? 1 : 0).map((key) => [key, object[key]])), null, 2)}\n`;

/**
 * Build the history in a fresh repository and adopt it. `pkg`: `{ name, source, copy: [[from, to]],
 * adoptScript, adoptCommand, languages, families, areaRoot }` (paths relative to the package;
 * `areaRoot` holds `<area>/<language>.json`). Every case must carry its model (`buildCases`/`randomCases`).
 */
export async function rehearse(history, cases, pkg) {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), `nyxid-${pkg.name}-i18n-matrix-`));
  const root = path.join(repo, pkg.name);
  const locales = path.join(root, "src/i18n/locales");
  const areaRoot = path.join(root, pkg.areaRoot);
  const git = async (...args) => {
    const result = await exec("git", args, { cwd: repo });
    if (result.status !== 0) throw new Error(`git ${args.join(" ")}: ${result.stderr}`);
    return result.stdout.trim();
  };
  const tryGit = async (...args) => { const result = await exec("git", args, { cwd: repo }); return result.status === 0 ? result.stdout.trim() : null; };
  const write = (file, text) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); };
  const read = (file) => fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {};
  const adopt = (...args) => exec(process.execPath, [path.join(root, pkg.adoptScript), ...args], { cwd: root });
  const relative = (file) => path.relative(repo, file).split(path.sep).join("/");
  const monolithFile = (language) => path.join(locales, `${language}.json`);
  const familyFile = (family, language) => path.join(locales, `${family}-${language}.json`);
  const areaFiles = (language) => fs.existsSync(areaRoot) ? fs.readdirSync(areaRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory())
    .map((entry) => path.join(areaRoot, entry.name, `${language}.json`)).filter((file) => fs.existsSync(file)) : [];
  const commit = async (message) => { await git("add", "-A", pkg.name); await git("commit", "-q", "--allow-empty", "-m", message); return git("rev-parse", "HEAD"); };
  const keysOf = (testCase) => formsOf(testCase).map((form) => keyName(testCase, form));

  /** Apply each case's edits at `point` (`side`: main or branch) to the layout that point writes. */
  const edit = (point, side) => {
    for (const language of pkg.languages) {
      const files = new Map();
      const load = (file) => { if (!files.has(file)) files.set(file, read(file)); return files.get(file); };
      for (const testCase of cases) {
        const keys = keysOf(testCase);
        for (const op of testCase.ops.filter((candidate) => candidate.point === point)) {
          if (op.only && op.only !== language) continue;
          for (const key of op.forms ? op.forms.map((form) => keyName(testCase, form)) : keys) {
            let file;
            if (testCase.catalog !== "app") file = familyFile(testCase.catalog, language);
            else if (side === "main" ? fs.existsSync(monolithFile(language)) : !isAreaPoint(point)) file = monolithFile(language);
            else {
              const sibling = siblingOf(key, keys);
              file = areaFiles(language).find((candidate) => Object.hasOwn(load(candidate), key))
                ?? (sibling ? areaFiles(language).find((candidate) => Object.hasOwn(load(candidate), sibling)) : undefined)
                ?? path.join(areaRoot, side === "main" ? "billing" : "settings", `${language}.json`);
            }
            const catalog = load(file);
            if (op.kind === "remove") delete catalog[key];
            else catalog[key] = valueOf(op.tag ?? op.point, key, language);
          }
        }
      }
      // An area emptied by hand edits stays as `{}` (a branch never drops its split areas by deleting keys).
      for (const [file, catalog] of files) write(file, canonical(catalog));
    }
  };
  const show = async (ref, file) => { const text = await tryGit("show", `${ref}:${relative(file)}`); return text === null ? null : JSON.parse(text); };
  const policyOf = new Map(cases.flatMap((testCase) => keysOf(testCase).map((key) => [key, testCase.resolutions ?? {}])));
  /**
   * Key-by-key merge resolution across languages (`sides`: { [language]: [base, ours, theirs] }): main's
   * change unless the branch changed the key too; a conflict in any language resolves the whole key by
   * the case's policy. Returns { [language]: catalog }.
   */
  const merged3 = (sides, mergeId) => {
    const keysOfSides = new Set(Object.values(sides).flatMap((catalogs) => catalogs.flatMap((catalog) => Object.keys(catalog))));
    const result = Object.fromEntries(Object.entries(sides).map(([language, [, ours]]) => [language, { ...ours }]));
    for (const key of keysOfSides) {
      const conflict = Object.values(sides).some(([b, o, t]) => o[key] !== b[key] && t[key] !== b[key] && t[key] !== o[key]);
      const policy = policyOf.get(key)?.[mergeId] ?? "ours";
      for (const [language, [b, o, t]] of Object.entries(sides)) {
        const value = !conflict ? (o[key] === b[key] ? t[key] : o[key]) : policy === "ours" ? o[key] : policy === "theirs" ? t[key] : valueOf(`third${mergeId}`, key, language);
        if (value === undefined) delete result[language][key]; else result[language][key] = value;
      }
    }
    return result;
  };
  const areasAt = async (ref, language) => {
    const listing = await tryGit("ls-tree", "-r", "--name-only", ref, "--", relative(areaRoot));
    const files = (listing ?? "").split("\n").filter((file) => file.endsWith(`/${language}.json`) && path.posix.dirname(path.posix.dirname(file)) === relative(areaRoot));
    return Promise.all(files.map(async (file) => [path.join(repo, file), JSON.parse(await git("show", `${ref}:${file}`))]));
  };

  for (const [from, to] of pkg.copy) write(path.join(repo, to), fs.readFileSync(from, "utf8"));
  write(path.join(root, "package.json"), '{"type":"module"}\n');
  fs.symlinkSync(path.join(pkg.source, "node_modules"), path.join(root, "node_modules"));
  write(path.join(repo, ".gitignore"), "node_modules\n");
  await git("init", "-q", "-b", "main");
  for (const [name, setting] of [["user.name", "matrix"], ["user.email", "matrix@example.test"], ["core.hooksPath", "/dev/null"], ["commit.gpgsign", "false"]]) await git("config", name, setting);
  for (const language of pkg.languages) {
    for (const catalog of ["app", ...pkg.families]) {
      const entries = cases.filter((testCase) => testCase.catalog === catalog && testCase.present).flatMap((testCase) => keysOf(testCase).map((key) => [key, valueOf("init", key, language)]));
      write(catalog === "app" ? monolithFile(language) : familyFile(catalog, language), canonical(Object.fromEntries(entries)));
    }
  }
  let fork = await commit("F: legacy fork");
  await git("branch", "legacy");
  const branchCommits = [fork];
  for (const step of history.steps) {
    if (step.t === "main") { await git("checkout", "-q", "main"); edit(step.point, "main"); await commit(`main: ${step.point}`); }
    else if (step.t === "split") {
      await git("checkout", "-q", "main");
      const split = await adopt("--branch", "HEAD", "--base", "empty", "--onto", "HEAD");
      if (split.status !== 0) throw new Error(split.stderr);
      await commit("main: split");
    } else if (step.t === "branch") {
      await git("checkout", "-q", "legacy");
      for (const point of step.points) edit(point, "branch");
      branchCommits.push(await commit(`branch: ${step.points.join(", ")}`));
    } else if (step.t === "localAreas") {
      await git("checkout", "-q", "legacy");
      for (const language of pkg.languages) write(path.join(areaRoot, "settings", `${language}.json`), canonical(read(monolithFile(language))));
      branchCommits.push(await commit("branch: creates its own split areas"));
    } else if (step.t === "merge") {
      await git("checkout", "-q", "legacy");
      const ours = await git("rev-parse", "HEAD");
      const theirs = await git("rev-parse", "main");
      const base = await git("merge-base", ours, theirs);
      await exec("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo });
      const catalogKinds = [...pkg.families.map((family) => (language) => familyFile(family, language)), ...(step.keep ? [] : [monolithFile])];
      for (const fileOf of catalogKinds) {
        const sides = Object.fromEntries(await Promise.all(pkg.languages.map(async (language) => {
          const file = fileOf(language);
          return [language, [await show(base, file) ?? {}, await show(ours, file) ?? {}, await show(theirs, file) ?? {}]];
        })));
        for (const [language, catalog] of Object.entries(merged3(sides, step.id))) write(fileOf(language), canonical(catalog));
      }
      if (step.keep) {
        await git("checkout", ours, "--", ...pkg.languages.map((language) => relative(monolithFile(language))));
        const flatten = (pairs) => Object.assign({}, ...pairs.map(([, catalog]) => catalog));
        const areaSides = Object.fromEntries(await Promise.all(pkg.languages.map(async (language) =>
          [language, await Promise.all([areasAt(base, language), areasAt(ours, language), areasAt(theirs, language)])])));
        const areaResult = merged3(Object.fromEntries(Object.entries(areaSides).map(([language, pairs]) => [language, pairs.map(flatten)])), step.id);
        for (const language of pkg.languages) {
          const [, ourAreas, theirAreas] = areaSides[language];
          const result = areaResult[language];
          const ourOwner = new Map(ourAreas.flatMap(([file, catalog]) => Object.keys(catalog).map((key) => [key, file])));
          const theirOwner = new Map(theirAreas.flatMap(([file, catalog]) => Object.keys(catalog).map((key) => [key, file])));
          const siblingsOf = new Map(cases.flatMap((testCase) => { const keys = keysOf(testCase); return keys.map((key) => [key, siblingOf(key, keys)]); }));
          for (const file of areaFiles(language)) fs.rmSync(file);
          const byFile = new Map();
          for (const [key, text] of Object.entries(result)) {
            const sibling = siblingsOf.get(key);
            const file = ourOwner.get(key) ?? (sibling ? ourOwner.get(sibling) : undefined) ?? theirOwner.get(key) ?? path.join(areaRoot, "unattributed", `${language}.json`);
            if (!byFile.has(file)) byFile.set(file, {});
            byFile.get(file)[key] = text;
          }
          for (const [file, catalog] of byFile) write(file, canonical(catalog));
        }
      } else fork = theirs;
      edit(step.point, "branch");
      branchCommits.push(await commit(`branch: merge main${step.keep ? ", keeping the monoliths" : ""}`));
    }
  }
  const before = async () => ({ status: await git("status", "--porcelain"), monoliths: pkg.languages.filter((language) => fs.existsSync(monolithFile(language))) });
  const run = { repo, locales, fork, route: history.route, stops: [] };
  if (history.route === "merge") {
    await git("checkout", "-q", "legacy");
    await exec("git", ["merge", "--no-commit", "-q", "-X", "no-renames", "main"], { cwd: repo });
    run.state = await before();
    run.refused = await adopt();
    run.command = run.refused.stderr.split("\n").find((line) => line.startsWith(`  ${pkg.adoptCommand} --branch`));
    run.final = run.refused.status === 0 ? run.refused
      : run.command ? await exec("sh", ["-c", run.command.trim().replace(pkg.adoptCommand, `"${process.execPath}" ${pkg.adoptScript}`)], { cwd: root }) : run.refused;
  } else if (history.route === "unrelatedOnto") {
    await git("checkout", "-q", "--orphan", "unrelated", "main");
    await commit("unrelated history with main's catalogs");
    await git("checkout", "-q", "legacy");
    run.state = await before();
    run.final = run.refused = await adopt("--branch", "legacy", "--base", fork, "--onto", "unrelated");
    run.command = run.refused.stderr.split("\n").find((line) => line.startsWith(`  ${pkg.adoptCommand} --branch`));
  } else {
    await git("checkout", "-q", "main");
    const count = history.route === "rebase" ? history.count : 1;
    const picked = branchCommits.slice(-count);
    if (history.route === "cherryPick") await exec("git", ["cherry-pick", picked[0]], { cwd: repo });
    else {
      await git("checkout", "-q", "legacy");
      await exec("git", ["rebase", "--onto", "main", branchCommits[branchCommits.length - count - 1]], { cwd: repo });
    }
    run.state = await before();
    for (let index = 0; index < count; index++) {
      const result = await adopt();
      run.stops.push(result);
      run.final = result;
      if (result.status !== 0) { run.refusedAt = index; break; }
      await git("add", "-A", relative(locales));
      if (history.route === "rebase") {
        const continued = await exec("git", ["-c", "core.editor=true", "rebase", "--continue"], { cwd: repo });
        if (continued.status !== 0 && index === count - 1) throw new Error(`rebase --continue: ${continued.stderr}`);
      }
    }
  }
  run.after = await before();
  run.lookup = (catalog, language) => catalog === "app" ? Object.assign({}, ...areaFiles(language).map(read)) : read(familyFile(catalog, language));
  run.ownerOf = (key) => {
    const owners = areaFiles(pkg.languages[0]).filter((file) => Object.hasOwn(read(file), key)).map((file) => path.basename(path.dirname(file)));
    return owners.length === 1 ? owners[0] : owners.length ? owners : undefined;
  };
  run.duplicates = (language) => {
    const seen = new Map();
    for (const file of areaFiles(language)) for (const key of Object.keys(read(file))) seen.set(key, (seen.get(key) ?? 0) + 1);
    return [...seen].filter(([, count]) => count > 1).map(([key]) => key);
  };
  run.overrides = () => read(path.join(locales, "app/en-overrides.json"));
  run.monolithsLeft = pkg.languages.filter((language) => fs.existsSync(monolithFile(language)));
  run.refusedKeys = (language) => [...new Set((run.final?.stderr ?? "").split("\n")
    .filter((line) => line.includes(`/${language}: `)).map((line) => line.slice(line.lastIndexOf(": ") + 2)))];
  run.cleanup = () => fs.rmSync(repo, { recursive: true, force: true });
  return run;
}

/** Run async jobs with bounded concurrency. */
export async function inParallel(jobs, limit = Math.max(2, os.availableParallelism?.() ?? os.cpus().length)) {
  const results = new Array(jobs.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, jobs.length) }, async () => {
    while (next < jobs.length) { const index = next++; results[index] = await jobs[index](); }
  }));
  return results;
}
