/**
 * Matrix engine shared by the web (frontend/test/i18n-mixed-matrix.test.ts) and native
 * (mobile/scripts/i18n-mixed-matrix.test.mjs) suites, for adoption of legacy-layout branches,
 * including "mixed" branches that merged the
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
 *   with it) is refused as a whole, with no command printed; so is one that merged main's areas into
 *   areas it made itself, a rebase that drops merge commits, and a cherry-pick of a merge commit.
 * - A monolith change equal to the split-area reference is main's own change, which the areas inherited
 *   at the merge (the same change on both sides), so it is not a branch edit (N14): only a genuine
 *   branch change can win or conflict.
 * - A commit already contained in onto (re-landed with cherry-pick after a revert) is adopted against
 *   its parent, like any other cherry-pick (N12).
 * Independently of the model, `invariantViolations` checks every adoption against the history itself.
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

/**
 * Main lands the legacy branch (before the split), one key across all languages (`sides`:
 * { [language]: [base, main, branch] }): the branch's value where it changed the key, else main's.
 * If that would leave the key in only some languages, the branch's values win in all of them (a
 * user resolving the conflict keeps a translatable key whole). Returns { [language]: value }.
 */
export function landKey(sides) {
  const result = Object.fromEntries(Object.entries(sides).map(([language, [base, main, branch]]) => [language, branch === base ? main : branch]));
  const present = Object.values(result).map((value) => value !== undefined);
  return present.every(Boolean) || !present.some(Boolean) ? result : Object.fromEntries(Object.entries(sides).map(([language, [, , branch]]) => [language, branch]));
}

/**
 * Main reverts a commit, one key across all languages (`sides`: { [language]: [parent, commit,
 * main] }): the parent's values only if main still has the commit's values in every language, else
 * main keeps its own (git would conflict there). Returns { [language]: value }.
 */
export function revertKey(sides) {
  const untouched = Object.values(sides).every(([, commit, main]) => main === commit);
  return Object.fromEntries(Object.entries(sides).map(([language, [parent, , main]]) => [language, untouched ? parent : main]));
}

/**
 * Run-level facts: whether the branch ends mixed, and the refusal the whole run must end in, if any:
 * "unreferenced" (split areas without a reference), "selfMadeAreas" (main's areas merged into areas the
 * branch made itself), "droppedMerges" (a full rebase through merge commits), "mergeCherryPick".
 */
export function historyFacts(history) {
  let keep = 0, local = false, keepAfterLocal = 0, merges = 0;
  for (const step of history.steps) {
    if (step.t === "merge") { merges++; if (step.keep) { keep++; if (local) keepAfterLocal++; } }
    if (step.t === "localAreas") local = true;
  }
  const mixed = keep > 0 || local;
  const refusal = history.route === "cherryPickMerge" ? "mergeCherryPick"
    : history.route === "rebase" && history.all && merges ? "droppedMerges"
      : history.route === "unrelatedOnto" && mixed ? "unreferenced"
        : local && keepAfterLocal && history.route === "merge" ? "selfMadeAreas"
          : local && keep === 0 && history.route === "merge" ? "unreferenced" : null;
  return { mixed, refusal };
}

/**
 * The message each run-level refusal must print. A `git cherry-pick -m` that applied cleanly leaves
 * nothing in progress (`run.nothingInProgress`); adoption then says so instead (NOTHING_IN_PROGRESS).
 */
/** The --branch and --onto an auto-detected merge adopts: the legacy side onto the split side. */
export const mergeRoles = (history) => history.reverse ? { branch: "MERGE_HEAD", onto: "HEAD" } : { branch: "HEAD", onto: "MERGE_HEAD" };
export const NOTHING_IN_PROGRESS = /no merge, rebase or cherry-pick detected/;
export const REFUSALS = {
  unreferenced: /neither .* has split areas/,
  selfMadeAreas: /merged split areas into split areas it had made itself/,
  droppedMerges: /This rebase drops merge commits? /,
  mergeCherryPick: /is a merge commit \(cherry-pick -m\)/,
};

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
    } else if (step.t === "mainMerge") {
      // Main merges the legacy branch (before the split); a conflict keeps the branch's value.
      const take = (base, mine, theirs) => {
        const result = blank();
        for (const key of keys) {
          const landed = landKey(Object.fromEntries(languages.map((language) => [language, [base[language][key], mine[language][key], theirs[language][key]]])));
          for (const language of languages) if (landed[language] !== undefined) result[language][key] = landed[language];
        }
        return result;
      };
      const fork = branchStates[0];
      main = app ? take(fork.monolith, main, monolith) : take(fork.family, main, family);
    } else if (step.t === "revert") {
      // Main reverts the branch's last commit, per key (revertKey).
      const [parent, commit] = branchStates.slice(-2).map((state) => app ? state.monolith : state.family);
      for (const key of keys) {
        const reverted = revertKey(Object.fromEntries(languages.map((language) => [language, [parent[language][key], commit[language][key], main[language][key]]])));
        for (const language of languages) { if (reverted[language] === undefined) delete main[language][key]; else main[language][key] = reverted[language]; }
      }
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
    const count = history.route === "rebase" ? (history.all ? branchStates.length - 1 : history.count) : 1;
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
    // N14: main and the branch's monoliths make the same change, then the branch edits the inherited areas.
    if (catalog === "app") {
      for (const [first, second, third] of choose(points, 3)) {
        if (first.side !== "main" || second.side !== "branch" || second.areas || !third.areas) continue;
        for (const kind of ["change", "remove"]) {
          for (const areaKind of KINDS) consider(catalog, [{ point: first.name, kind, tag: "same" }, { point: second.name, kind, tag: "same" }, { point: third.name, kind: areaKind }], "same change, then area edit");
        }
      }
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
 * commits and main edits in between, and a final route (merge, cherry-pick or rebase). Seeds also
 * cycle through special classes, so every class appears in any 20 consecutive seeds: areas the branch
 * made itself (refused: no reference, or merged with main's areas), an --onto with unrelated history
 * (refused), a legacy commit re-landed by cherry-pick after main merged and reverted it (N12), a full
 * rebase of a branch without merges (modelled) or with merges (refused), and a cherry-pick of a merge
 * commit (refused).
 */
export function randomHistory(seed) {
  const rand = random(seed);
  const steps = [];
  let mainIndex = 0, branchIndex = 0, mergeIndex = 0, keep = 0, mainMoved = false;
  const mainEdit = () => { mainMoved = true; steps.push({ t: "main", point: `m${++mainIndex}` }); };
  const branchCommit = () => {
    branchIndex++;
    steps.push({ t: "branch", points: keep ? [`b${branchIndex}`, `b${branchIndex}Areas`] : [`b${branchIndex}`] });
  };
  // Every merge brings something in (with main unchanged, git merge records no merge commit).
  const merge = (keepMonoliths) => { if (!mainMoved) mainEdit(); mainMoved = false; mergeIndex++; if (keepMonoliths) keep++; steps.push({ t: "merge", keep: keepMonoliths, point: `r${mergeIndex}`, id: `M${mergeIndex}` }); };
  const variant = { 3: "localAreas", 7: "unrelatedOnto", 1: "selfMadeAreas", 9: "reland", 5: seed % 20 === 5 ? "rebaseAllLinear" : "rebaseAllMerges", 0: "cherryPickMerge" }[seed % 10] ?? "ordinary";
  if (variant === "reland") {
    if (rand.chance(0.6)) mainEdit();
    branchCommit();
    steps.push({ t: "mainMerge" });
    if (rand.chance(0.6)) mainEdit();
    steps.push({ t: "revert" });
    if (rand.chance(0.6)) mainEdit();
    steps.push({ t: "split" });
    mainEdit();
    return { title: `random seed ${seed}: a legacy commit main merged and reverted, re-landed by cherry-pick after the split`, seed, route: "cherryPick", steps };
  }
  if (rand.chance(0.7)) mainEdit();
  branchCommit();
  const legacyMerges = variant === "rebaseAllLinear" || variant === "localAreas" || variant === "selfMadeAreas" ? 0 : rand.int(variant === "rebaseAllMerges" || variant === "cherryPickMerge" ? 1 : 0, 3);
  for (let index = legacyMerges; index > 0; index--) {
    if (rand.chance(0.7)) mainEdit();
    if (rand.chance(0.6)) branchCommit();
    merge(false);
  }
  if (rand.chance(0.6)) mainEdit();
  steps.push({ t: "split" });
  mainMoved = true;
  if (variant === "localAreas" || variant === "selfMadeAreas") {
    if (rand.chance(0.5)) branchCommit();
    steps.push({ t: "localAreas" });
    if (variant === "selfMadeAreas") {
      for (let index = rand.int(1, 2); index > 0; index--) { mainEdit(); if (rand.chance(0.6)) branchCommit(); merge(true); }
      branchCommit();
    }
    mainEdit();
    const reverse = Math.floor(seed / 10) % 2 === 1;
    return { title: `random seed ${seed}: the branch made its own split areas${variant === "selfMadeAreas" ? ", then merged main's" : ""}${reverse ? ", merged into main" : ""}`, seed, route: "merge", reverse, steps };
  }
  const keeps = variant === "rebaseAllLinear" ? 0 : variant === "unrelatedOnto" ? rand.int(1, 2) : rand.int(0, 4);
  for (let index = 0; index < keeps; index++) {
    if (rand.chance(0.7)) mainEdit();
    if (rand.chance(0.6)) branchCommit();
    merge(true);
  }
  const trailing = rand.int(1, 3);
  for (let index = 0; index < trailing; index++) branchCommit();
  const route = variant === "rebaseAllLinear" || variant === "rebaseAllMerges" ? "rebase"
    : variant === "cherryPickMerge" ? "cherryPickMerge" : variant === "unrelatedOnto" ? "unrelatedOnto"
      : ["merge", "cherryPick", "rebase", "merge"][((seed % 10) / 2) % 4]; // ordinary seeds end in 2, 4, 6, 8
  // A merge route needs main to have moved since the branch's last merge, or there is nothing to adopt.
  if (route === "merge" || rand.chance(0.8)) mainEdit();
  if (route === "unrelatedOnto") return { title: `random seed ${seed}: --onto shares no history with the branch`, seed, route, steps };
  const all = variant === "rebaseAllLinear" || variant === "rebaseAllMerges";
  const count = route === "rebase" && !all ? Math.min(trailing, (Math.floor(seed / 10) % 2) + 1) : undefined;
  // Half the merge routes merge the legacy branch into main (split checked out) instead.
  const reverse = route === "merge" && Math.floor(seed / 10) % 2 === 1;
  const description = `${steps.filter((step) => step.t === "merge" && !step.keep).length} legacy and ${keep} keep-monolith merges, ${all ? "rebase of the whole branch" : reverse ? "merge into main" : route}${count ? ` of ${count}` : ""}`;
  return { title: `random seed ${seed}: ${description}`, seed, route, reverse, count, all, steps };
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
/** Every repository rehearse created and has not removed; `cleanupAll` removes them (also on exit). */
const created = new Set();
export function cleanupAll() {
  for (const repo of created) fs.rmSync(repo, { recursive: true, force: true });
  created.clear();
}
process.once("exit", cleanupAll);

export async function rehearse(history, cases, pkg) {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), `nyxid-${pkg.name}-i18n-matrix-`));
  created.add(repo);
  try {
    return await rehearseIn(repo, history, cases, pkg);
  } catch (error) {
    fs.rmSync(repo, { recursive: true, force: true });
    created.delete(repo);
    throw new Error(`${history.title}: ${error.message}`);
  }
}

async function rehearseIn(repo, history, cases, pkg) {
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
  let resolvedForBranch = new Set(); // keys the current merge resolved with the branch's or a third value
  const branchResolutions = new Map(); // key -> merge commits that resolved it for the branch
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
      if (conflict && policy !== "theirs") resolvedForBranch.add(key);
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
      // Every run keeps one untouched app key, so the split always has an area (a real catalog is never empty).
      if (catalog === "app") entries.push(["matrix anchor", `anchor ${language}`]);
      write(catalog === "app" ? monolithFile(language) : familyFile(catalog, language), canonical(Object.fromEntries(entries)));
    }
  }
  let fork = await commit("F: legacy fork");
  await git("branch", "legacy");
  const branchCommits = [fork];
  const commitOfPoint = new Map(); // branch point -> the branch commit holding its edits
  let lastMergedMain = fork, mergeCommits = [];
  for (const step of history.steps) {
    if (step.t === "main") { await git("checkout", "-q", "main"); edit(step.point, "main"); await commit(`main: ${step.point}`); }
    else if (step.t === "mainMerge") {
      await git("checkout", "-q", "main");
      const [ours, theirs] = [await git("rev-parse", "HEAD"), await git("rev-parse", "legacy")];
      const base = await git("merge-base", ours, theirs);
      await exec("git", ["merge", "--no-ff", "--no-commit", "-q", "legacy"], { cwd: repo });
      // A conflict keeps the branch's value (the branch lands on main), per key (landKey).
      for (const fileOf of [monolithFile, ...pkg.families.map((family) => (language) => familyFile(family, language))]) {
        const sides = Object.fromEntries(await Promise.all(pkg.languages.map(async (language) =>
          [language, [await show(base, fileOf(language)) ?? {}, await show(ours, fileOf(language)) ?? {}, await show(theirs, fileOf(language)) ?? {}]])));
        const results = Object.fromEntries(pkg.languages.map((language) => [language, { ...sides[language][1] }]));
        for (const key of new Set(Object.values(sides).flatMap((catalogs) => catalogs.flatMap(Object.keys)))) {
          const landed = landKey(Object.fromEntries(pkg.languages.map((language) => [language, sides[language].map((catalog) => catalog[key])])));
          for (const language of pkg.languages) { if (landed[language] === undefined) delete results[language][key]; else results[language][key] = landed[language]; }
        }
        for (const language of pkg.languages) write(fileOf(language), canonical(results[language]));
      }
      await commit("main: merge the legacy branch");
    } else if (step.t === "revert") {
      // Main reverts the branch's last commit, per key (revertKey).
      await git("checkout", "-q", "main");
      const [parent, last] = branchCommits.slice(-2);
      for (const fileOf of [monolithFile, ...pkg.families.map((family) => (language) => familyFile(family, language))]) {
        const sides = Object.fromEntries(await Promise.all(pkg.languages.map(async (language) =>
          [language, [await show(parent, fileOf(language)) ?? {}, await show(last, fileOf(language)) ?? {}, read(fileOf(language))]])));
        const results = Object.fromEntries(pkg.languages.map((language) => [language, { ...sides[language][2] }]));
        for (const key of new Set(Object.values(sides).flatMap((catalogs) => catalogs.flatMap(Object.keys)))) {
          const reverted = revertKey(Object.fromEntries(pkg.languages.map((language) => [language, sides[language].map((catalog) => catalog[key])])));
          for (const language of pkg.languages) { if (reverted[language] === undefined) delete results[language][key]; else results[language][key] = reverted[language]; }
        }
        for (const language of pkg.languages) write(fileOf(language), canonical(results[language]));
      }
      await commit("main: revert the branch's commit");
    }
    else if (step.t === "split") {
      await git("checkout", "-q", "main");
      const split = await adopt("--branch", "HEAD", "--base", "empty", "--onto", "HEAD");
      if (split.status !== 0) throw new Error(split.stderr);
      await commit("main: split");
    } else if (step.t === "branch") {
      await git("checkout", "-q", "legacy");
      for (const point of step.points) edit(point, "branch");
      branchCommits.push(await commit(`branch: ${step.points.join(", ")}`));
      for (const point of step.points) commitOfPoint.set(point, branchCommits.at(-1));
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
      lastMergedMain = theirs;
      edit(step.point, "branch");
      branchCommits.push(await commit(`branch: merge main${step.keep ? ", keeping the monoliths" : ""}`));
      mergeCommits.push(branchCommits.at(-1));
      for (const key of resolvedForBranch) branchResolutions.set(key, [...(branchResolutions.get(key) ?? []), branchCommits.at(-1)]);
      resolvedForBranch = new Set();
      commitOfPoint.set(step.point, branchCommits.at(-1));
    }
  }
  const before = async () => ({ status: await git("status", "--porcelain"), monoliths: pkg.languages.filter((language) => fs.existsSync(monolithFile(language))) });
  const run = { repo, locales, fork, route: history.route, stops: [] };
  if (history.route === "merge") {
    // `reverse`: the legacy branch is merged into main (split checked out); adoption swaps the roles.
    await git("checkout", "-q", history.reverse ? "main" : "legacy");
    await exec("git", ["merge", "--no-commit", "-q", "-X", "no-renames", history.reverse ? "legacy" : "main"], { cwd: repo });
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
  } else if (history.route === "cherryPickMerge") {
    await git("checkout", "-q", "main");
    await exec("git", ["cherry-pick", "-m", "1", mergeCommits.at(-1)], { cwd: repo });
    // Git stops only on a conflict; a merge whose changes apply cleanly leaves nothing to adopt.
    run.nothingInProgress = await tryGit("rev-parse", "--verify", "--quiet", "CHERRY_PICK_HEAD") === null;
    run.state = await before();
    run.final = run.refused = await adopt();
  } else {
    await git("checkout", "-q", "main");
    const count = history.route === "rebase" ? (history.all ? branchCommits.length - 1 : history.count) : 1;
    const picked = branchCommits.slice(-count);
    run.adopted = picked;
    if (history.route === "cherryPick") await exec("git", ["cherry-pick", picked[0]], { cwd: repo });
    else {
      await git("checkout", "-q", "legacy");
      await exec("git", history.all ? ["rebase", "main"] : ["rebase", "--onto", "main", branchCommits[branchCommits.length - count - 1]], { cwd: repo });
    }
    run.state = await before();
    for (let index = 0; index < count; index++) {
      const result = await adopt();
      run.stops.push(result);
      run.final = result;
      if (result.status !== 0) { run.refusedAt = index; run.refused = result; break; }
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
  // Sync point of each branch edit, per layout: where that layout last took main's values before it.
  // Merge routes: the monoliths at the legacy fork (or last legacy merge), split areas and families at the
  // last main the branch merged; cherry-pick/rebase: the adopted commit's parent.
  const legacyMerges = mergeCommits.filter((sha, index) => !history.steps.filter((step) => step.t === "merge")[index].keep);
  run.commitOfPoint = commitOfPoint;
  run.branchResolutions = branchResolutions;
  const commitRoute = history.route === "cherryPick" || history.route === "rebase";
  run.syncCommit = (point, layout) => {
    if (commitRoute) { const commitSha = commitOfPoint.get(point); return commitSha ? branchCommits[branchCommits.indexOf(commitSha) - 1] : undefined; }
    return layout === "monolith" ? run.fork : lastMergedMain;
  };
  run.adoptedPoint = (point, layout) => {
    const commitSha = commitOfPoint.get(point);
    if (!commitSha) return false;
    if (run.adopted) return run.adopted.includes(commitSha);
    const last = (layout === "monolith" ? legacyMerges : mergeCommits).at(-1);
    return !last || branchCommits.indexOf(commitSha) >= branchCommits.indexOf(last);
  };
  const shown = new Map();
  const showCached = async (sha, file) => { const id = `${sha}:${file}`; if (!shown.has(id)) shown.set(id, await show(sha, file)); return shown.get(id); };
  /** A key's value at a commit: its split areas if it has any, else its monoliths (or the family file). */
  run.valueAt = async (sha, catalog, key, language, layout) => {
    if (catalog !== "app") return (await showCached(sha, familyFile(catalog, language)))?.[key];
    const areas = layout === "monolith" ? [] : await areasAt(sha, language);
    if (areas.length || layout === "areas") return areas.length ? Object.assign({}, ...areas.map(([, catalog]) => catalog))[key] : null;
    return (await showCached(sha, monolithFile(language)))?.[key];
  };
  const mainSha = await git("rev-parse", "main");
  run.mainTip = (catalog, key, language) => run.valueAt(mainSha, catalog, key, language);
  run.cleanup = () => { fs.rmSync(repo, { recursive: true, force: true }); created.delete(repo); };
  return run;
}

/**
 * Invariants checked against the history itself, independently of the model, for an adopted case:
 * (1) a value that is not main's latest must come from a branch edit, or from a conflict the branch
 * resolved in its own favour at a merge (no revert to an older value); on a cherry-pick or rebase,
 * only from an edit in a commit that was actually picked;
 * (2) a key main no longer has reappears only with a branch value; (3) the branch's last edit to a
 * key in the adopted commits (after its layout last took main's values: the monoliths at the legacy
 * fork, split areas and families at the last merged main, or the adopted commit's parent) is applied,
 * unless it only restates the value that layout took from main (for a monolith edit, also the split
 * areas' value: main made the same change, N14), or is a removal that main has since undone by
 * re-adding or changing the key. Returns a list of violations.
 */
export async function invariantViolations(history, testCase, run, languages) {
  const branchTags = new Set([...historyPoints(history).filter((point) => point.side === "branch").map((point) => point.name)]);
  const tagOf = (value) => value === undefined ? undefined : value.slice(0, value.indexOf("-"));
  const layoutOf = (point) => testCase.catalog !== "app" ? "family" : isAreaPoint(point) ? "areas" : "monolith";
  // On a cherry-pick or rebase only the picked commits are the branch's: a value from any other branch
  // commit (or a merge resolution, never picked) must not appear. A merge adopts the branch's history.
  const commitRoute = history.route === "cherryPick" || history.route === "rebase";
  const adoptedTags = new Set(testCase.ops.filter((op) => branchTags.has(op.point) && run.adoptedPoint(op.point, layoutOf(op.point))).map((op) => op.tag ?? op.point));
  const isBranchValue = (value) => commitRoute ? adoptedTags.has(tagOf(value))
    : branchTags.has(tagOf(value)) || /^third/.test(tagOf(value) ?? "") || tagOf(value) === "same";
  const violations = [];
  for (const form of formsOf(testCase)) {
    const key = keyName(testCase, form);
    for (const language of languages) {
      const value = run.lookup(testCase.catalog, language)[key];
      const mainLatest = await run.mainTip(testCase.catalog, key, language);
      // A conflict the branch resolved in its own favour at a merge is a branch decision too.
      let resolved = false;
      for (const mergeCommit of commitRoute ? [] : run.branchResolutions.get(key) ?? []) if (value !== undefined && value === await run.valueAt(mergeCommit, testCase.catalog, key, language)) resolved = true;
      const branchValue = isBranchValue(value) || resolved;
      if (value !== undefined && value !== mainLatest && !branchValue) violations.push(`${key} ${language}: ${value} is neither main's latest (${mainLatest}) nor a branch value`);
      if (mainLatest === undefined && value !== undefined && !branchValue) violations.push(`${key} ${language}: removed by main but reappeared as ${value}`);
      const last = testCase.ops.filter((op) => branchTags.has(op.point) && run.adoptedPoint(op.point, layoutOf(op.point)) && (!op.only || op.only === language)
        && (!op.forms || op.forms.includes(form))).at(-1);
      if (!last) continue;
      const layout = layoutOf(last.point);
      const wanted = last.kind === "remove" ? undefined : `${last.tag ?? last.point}-${key} ${language}`;
      if (value === wanted) continue;
      const synced = await run.valueAt(run.syncCommit(last.point, layout), testCase.catalog, key, language, layout);
      if (wanted === synced) continue; // the edit only restated the value this layout last took from main
      if (layout === "monolith") {
        // N14: a monolith edit equal to the areas' value from the same sync is the same change main made.
        const areaSynced = await run.valueAt(run.syncCommit(last.point, "areas"), testCase.catalog, key, language, "areas");
        if (areaSynced !== null && wanted === areaSynced) continue;
      }
      if (last.kind === "remove" && value === mainLatest && mainLatest !== synced) continue;
      violations.push(`${key} ${language}: the branch's last edit (${last.kind} at ${last.point}) was not applied: got ${value}`);
    }
  }
  return violations;
}

/**
 * Run async jobs with bounded concurrency (I18N_MATRIX_CONCURRENCY, else the available cores). Every
 * job settles before the first error is rethrown, so finished repositories are still cleaned up.
 */
export async function inParallel(jobs, limit = Number(process.env.I18N_MATRIX_CONCURRENCY) || Math.max(2, os.availableParallelism?.() ?? os.cpus().length)) {
  const results = new Array(jobs.length);
  const errors = [];
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, jobs.length) }, async () => {
    while (next < jobs.length) {
      const index = next++;
      try { results[index] = await jobs[index](); } catch (error) { errors.push(error); }
    }
  }));
  if (errors.length) throw new AggregateError(errors, errors.map((error) => error.message).join("\n"));
  return results;
}
