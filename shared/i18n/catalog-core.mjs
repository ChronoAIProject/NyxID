/**
 * Language-agnostic catalog tooling shared by the web (`frontend/scripts`) and
 * native (`mobile/scripts`) i18n scripts. Catalogs are flat JSON phrase maps
 * split into area ("portion") directories: `<dir>/<area>/<language>.json`.
 *
 * This module has no dependencies outside Node. Each package supplies its own
 * language list, area map, source predicate and literal parser.
 */
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

export const codeUnitCompare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
export const sortObject = (object) => Object.fromEntries(
  Object.keys(object).sort(codeUnitCompare).map((key) => [key, object[key]]),
);
export const canonicalText = (object) => `${JSON.stringify(sortObject(object), null, 2)}\n`;
export const stripPluralSuffix = (key) => key.replace(/_(?:zero|one|two|few|many|other)$/, "");

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

export function discoverPortions(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory()).map((entry) => entry.name).sort(codeUnitCompare);
}

export const portionFile = (dir, portion, language) => path.join(dir, portion, `${language}.json`);

/** Three-way delta of one flat catalog: apply base -> branch onto `onto`. */
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

/**
 * Apply a mixed branch's edits to `onto`. Its monoliths changed from `monolithBase` (the legacy
 * fork) and its inherited split areas changed from `areaBase` (the last main it merged). Where the
 * areas changed a key, that edit was made against the newer main and takes precedence: the
 * monolith delta skips the key, and a monolith change to a different result is a contradiction.
 * A monolith change equal to the areas reference is main's own change (the same change on both
 * sides) and is not a branch edit. Everything else goes through the ordinary three-way rules.
 */
export function applyMixedDelta({ monolithBase, monolith, areaBase, areas, onto, labels }) {
  const areaKeys = new Set([...Object.keys(areaBase), ...Object.keys(areas)].filter((key) => areaBase[key] !== areas[key]));
  // A monolith change that matches the areas reference is one main already made (and the areas
  // inherited at the merge): it is the same change on both sides, so it is not a branch edit.
  const monolithChanged = (key) => monolithBase[key] !== monolith[key] && monolith[key] !== areaBase[key];
  const contradictions = [...areaKeys]
    .filter((key) => monolithChanged(key) && monolith[key] !== areas[key])
    .map((key) => `${labels.areas}: monoliths and split areas changed differently: ${key}`);
  const skipped = new Set([...new Set([...Object.keys(monolithBase), ...Object.keys(monolith)])].filter((key) => areaKeys.has(key) || !monolithChanged(key)));
  const withoutAreaKeys = (catalog) => Object.fromEntries(Object.entries(catalog).filter(([key]) => !skipped.has(key)));
  const first = applyDelta(withoutAreaKeys(monolithBase), withoutAreaKeys(monolith), onto, labels.monolith);
  const second = applyDelta(areaBase, areas, first.result, labels.areas);
  return { result: second.result, conflicts: [...first.conflicts, ...contradictions, ...second.conflicts] };
}

/**
 * Attribute every key (plural siblings together) to the single area whose
 * placement sources reference it as a literal, `shared` when several areas do,
 * and `unattributed` when none does.
 */
export function scanUsage(sources, keys, { isSource, areaForPath, literals }) {
  const groups = new Map();
  for (const key of keys) {
    const base = stripPluralSuffix(key);
    if (!groups.has(base)) groups.set(base, new Set());
  }
  for (const [file, source] of sources) {
    if (!isSource(file)) continue;
    for (const literal of literals(file, source)) groups.get(stripPluralSuffix(literal))?.add(areaForPath(file));
  }
  return new Map([...groups].map(([key, owners]) => [key,
    owners.size === 0 ? "unattributed" : owners.size > 1 ? "shared" : [...owners][0],
  ]));
}

/**
 * Warnings for keys stored in several areas with identical values in every
 * language. These only warn (sync moves them to `shared`): two branches can add
 * the same phrase to different areas and git merges both cleanly, so failing
 * would turn main red with nothing to resolve. Differing values remain errors.
 */
export function identicalDuplicateWarnings(analysis, action) {
  return analysis.duplicates.filter(({ identical }) => identical)
    .map(({ key, portions }) => `i18n: identical duplicate ${key} in ${portions.join(", ")}; ${action}`);
}

/** Catalog operations bound to one package's language list and issue label. */
export function catalogTools(languages, { label }) {
  const emptyLanguages = () => Object.fromEntries(languages.map((language) => [language, {}]));

  function loadPortions(dir) {
    return Object.fromEntries(discoverPortions(dir).map((portion) => [portion,
      Object.fromEntries(languages.map((language) => {
        const file = portionFile(dir, portion, language);
        return [language, fs.existsSync(file) ? readJson(file) : {}];
      })),
    ]));
  }

  function analyzePortions(portions) {
    const merged = emptyLanguages();
    const locations = new Map();
    const issues = [];
    for (const portion of Object.keys(portions).sort(codeUnitCompare)) {
      const catalogs = portions[portion];
      const keySets = languages.map((language) => canonicalText(Object.fromEntries(Object.keys(catalogs[language]).map((key) => [key, true]))));
      if (new Set(keySets).size > 1) issues.push(`${label}/${portion}: key parity differs across ${languages.join("/")}`);
      for (const language of languages) {
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
      const identical = languages.every((language) => names.every((portion) =>
        Object.hasOwn(portions[portion][language], key) &&
        portions[portion][language][key] === portions[names[0]][language][key],
      ));
      duplicates.push({ key, portions: names, identical });
      if (!identical) issues.push(`${label} duplicate key has differing values: ${key} (${names.join(", ")})`);
    }
    const pluralOwners = new Map();
    for (const [key, owners] of locations) {
      const base = stripPluralSuffix(key);
      const ownerSet = [...owners].sort(codeUnitCompare).join(", ");
      const previous = pluralOwners.get(base);
      if (previous !== undefined && previous !== ownerSet) issues.push(`${label} plural siblings are not co-located: ${base} (${previous}; ${ownerSet})`);
      pluralOwners.set(base, ownerSet);
    }
    return { merged, locations, duplicates, issues };
  }

  function consolidateDuplicates(portions) {
    const result = structuredClone(portions);
    const { duplicates, issues } = analyzePortions(portions);
    const differing = issues.filter((issue) => !issue.includes("plural siblings"));
    if (differing.length) throw new Error(differing.join("\n"));
    // Move the whole plural family when any sibling is duplicated.
    const bases = new Set(duplicates.map(({ key }) => stripPluralSuffix(key)));
    const { merged } = analyzePortions(portions);
    if (bases.size) result.shared ??= emptyLanguages();
    for (const language of languages) {
      for (const [key, value] of Object.entries(merged[language])) {
        if (!bases.has(stripPluralSuffix(key))) continue;
        for (const catalogs of Object.values(result)) delete catalogs[language][key];
        result.shared[language][key] = value;
      }
    }
    return result;
  }

  /**
   * Place every merged key: existing (onto) owners first, by key then by plural sibling; then
   * `preferred` owners (the branch's area for keys that arrive through its split-area edits), by
   * key then by sibling; then literal usage (`scan` as for scanUsage).
   */
  function placeCatalogs(merged, sources, existing, scan, preferred = new Map()) {
    const result = structuredClone(existing);
    const keys = [...new Set(languages.flatMap((language) => Object.keys(merged[language])))];
    const placement = scanUsage(sources, keys, scan);
    const existingOwners = analyzePortions(existing).locations;
    const siblingOwners = new Map([...existingOwners].map(([key, owners]) => [stripPluralSuffix(key), owners]));
    const preferredSiblings = new Map([...preferred].map(([key, owner]) => [stripPluralSuffix(key), owner]));
    for (const language of languages) {
      for (const catalogs of Object.values(result)) catalogs[language] = {};
      for (const key of keys) {
        if (!Object.hasOwn(merged[language], key)) continue;
        const owner = existingOwners.get(key) ?? siblingOwners.get(stripPluralSuffix(key));
        const portion = owner?.size === 1 ? [...owner][0]
          : preferred.get(key) ?? preferredSiblings.get(stripPluralSuffix(key)) ?? placement.get(stripPluralSuffix(key));
        result[portion] ??= emptyLanguages();
        result[portion][language][key] = merged[language][key];
      }
    }
    return result;
  }

  /** Areas with at least one key in some language; an emptied area is pruned rather than kept as `{}` files. */
  const nonEmptyPortions = (portions) => Object.fromEntries(Object.entries(portions)
    .filter(([, catalogs]) => languages.some((language) => Object.keys(catalogs[language] ?? {}).length)));

  function writePortions(dir, allPortions) {
    const portions = nonEmptyPortions(allPortions);
    for (const [portion, catalogs] of Object.entries(portions)) {
      for (const language of languages) writeJson(portionFile(dir, portion, language), catalogs[language]);
    }
    for (const portion of discoverPortions(dir)) {
      if (Object.hasOwn(portions, portion)) continue;
      for (const language of languages) {
        const file = portionFile(dir, portion, language);
        if (fs.existsSync(file)) fs.rmSync(file);
      }
      const portionDir = path.join(dir, portion);
      if (!fs.readdirSync(portionDir).length) fs.rmdirSync(portionDir);
    }
  }

  /** Portions from snapshot files whose relative path matches `pattern` (groups: area, language). */
  function collectPortions(files, pattern) {
    const portions = {};
    for (const [file, text] of files) {
      const match = file.match(pattern);
      if (!match || !languages.includes(match[2])) continue;
      portions[match[1]] ??= emptyLanguages();
      portions[match[1]][match[2]] = JSON.parse(text);
    }
    return portions;
  }

  /**
   * Apply each named catalog's base -> branch delta onto `onto` per language.
   * Snapshots are `{ [name]: { [language]: catalog } }`; with an empty base the
   * branch is adopted onto nothing. `followUps[name]` (`{ label, base, branch }`)
   * adds a mixed branch's inherited-areas delta to that catalog (see applyMixedDelta).
   */
  function mergeDeltas(base, branch, onto, names, emptyBase, followUps = {}) {
    const result = Object.fromEntries(names.map((name) => [name, emptyLanguages()]));
    const conflicts = [];
    for (const language of languages) {
      for (const name of names) {
        // A mixed branch's edits to its inherited split areas (`followUps[name]`) take precedence per key.
        const followUp = followUps[name];
        const delta = followUp
          ? applyMixedDelta({
            monolithBase: base[name][language], monolith: branch[name][language], areaBase: followUp.base[language], areas: followUp.branch[language],
            onto: emptyBase ? {} : onto[name][language], labels: { monolith: `${name}/${language}`, areas: `${followUp.label}/${language}` },
          })
          : applyDelta(base[name][language], branch[name][language], emptyBase ? {} : onto[name][language], `${name}/${language}`);
        result[name][language] = delta.result;
        conflicts.push(...delta.conflicts);
      }
    }
    return { result, conflicts };
  }

  return {
    languages, emptyLanguages, loadPortions, analyzePortions, consolidateDuplicates,
    placeCatalogs, nonEmptyPortions, writePortions, collectPortions, mergeDeltas,
  };
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
 * `options.legacyFork()` returns the fork candidates (git-derived) for the refusal guidance;
 * `options.commitLocal` says --base is a parent of --branch (a rebased or cherry-picked commit);
 * `options.baseLessAreaMerges()` lists branch merges that joined split areas without a common one.
 * Shapes it does not model are refused: a branch without monoliths or an onto that is not plain
 * split (unless --base is empty), and a mixed branch whose areas were joined by such a merge.
 */
export function chooseCatalogViews(snapshots, refs, { languages, adopt, mergeBase = null, order = "base", legacyFork = () => [], commitLocal = false, baseLessAreaMerges = () => [] }) {
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
  const emptyBase = refs.base === "empty";
  // Shapes the adoption does not model are refused before any delta is computed.
  if (!emptyBase && !branch.monolith) {
    fail(`${refs.branch} (--branch) has no legacy monoliths, so it is not a branch from before the split and there is nothing to adopt; resolve split-catalog conflicts with git, then run the sync script. (--base empty is the only way to regenerate a split commit's placement.)`);
  }
  if (!emptyBase && (onto.monolith || !onto.areas)) {
    fail(onto.monolith
      ? `${refs.onto} (--onto) still has legacy monoliths${onto.areas ? " beside its split areas" : ""}; adopt only onto a commit that has just the split catalogs (adopt that branch first, or use --base empty for the initial split).`
      : `${refs.onto} (--onto) has no split catalogs to adopt onto; pass the split main (or a commit on it) as --onto.`);
  }
  if (mixed && !baseHasCatalogs) {
    fail([`${inheritedNote} It cannot be adopted from an empty base.`, ...forkGuidance()]);
  }
  if (mixed && !commitLocal) {
    const merges = baseLessAreaMerges();
    if (merges.length) {
      fail([
        `${refs.branch} (--branch) merged split areas into split areas it had made itself (merge ${merges.join(", ")}: both sides had areas but their merge-base had none), so its own area edits cannot be told apart from main's and a resolution that kept its copy would revert main.`,
        "This history cannot be adopted automatically. Port the branch's catalog changes onto a fresh branch from main by hand (its monolith edits can still be adopted from a commit before it made its own areas, with --branch <that commit> --base <its legacy fork>).",
      ]);
    }
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
  const empty = () => Object.fromEntries(languages.map((language) => [language, {}]));
  const chosen = {
    base: (baseHasCatalogs && branchView ? reference(branchView) : null) ?? empty(),
    branch: (branch.monolith ?? branch.areas) ?? empty(),
    onto: (onto.areas ?? onto.monolith) ?? empty(),
  };
  return { ...chosen, areaDelta: mixed ? { base: reference("areas"), branch: branch.areas } : null };
}

/**
 * Refuse an adoption whose result has catalog `issues` (analyzePortions/consolidateDuplicates),
 * writing nothing; a key-parity issue (a key added or removed in only some languages) adds how to
 * fix it on the branch.
 */
export function refuseCatalogIssues(issues, { languages, label }) {
  const list = languages.length > 1 ? `${languages.slice(0, -1).join(", ")} and ${languages.at(-1)}` : languages[0];
  const parity = issues.some((issue) => issue.includes("key parity differs"))
    ? `\nEvery ${label} key needs a value in each of ${list}: add or remove the key in every language on the branch, then rerun.` : "";
  throw new Error(`No files written. ${issues.join("\n")}${parity}`);
}

/** Git access for adoption: one `ls-tree` and one `cat-file --batch` per snapshot. */
export function gitRepository(cwd) {
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd, encoding: "utf8" }).trim();
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trimEnd();
  const prefix = path.relative(root, fs.realpathSync(cwd)).split(path.sep).join("/");

  function readBlobs(ref, files) {
    if (!files.length) return new Map();
    const output = execFileSync("git", ["cat-file", "--batch"], {
      cwd: root, input: files.map((file) => `${ref}:${file}\n`).join(""), maxBuffer: 1 << 30,
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

  /**
   * Files under `dir` (relative to the package `cwd`) at `ref` that pass
   * `select`, as a Map of package-relative path to text. `empty` has no files.
   */
  function snapshotFiles(ref, dir, select) {
    if (ref === "empty") return new Map();
    const files = execFileSync("git", ["ls-tree", "-r", "-z", "--name-only", ref, "--", `${prefix}/${dir}`], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
      .split("\0").filter(Boolean).map((file) => file.slice(prefix.length + 1)).filter(select);
    const blobs = readBlobs(ref, files.map((file) => `${prefix}/${file}`));
    return new Map(files.map((file) => [file, blobs.get(`${prefix}/${file}`)]));
  }

  const tryGit = (...args) => { try { return git(...args); } catch { return null; } };

  /** The merge-base of two refs, or null when they share no history. */
  const mergeBase = (left, right) => tryGit("merge-base", left, right);

  /** Which of --base and the merge-base is newer: "base" (same or newer), "mergeBase", or "diverged". */
  const referenceOrder = (baseRef, mergeBaseRef) =>
    tryGit("merge-base", "--is-ancestor", mergeBaseRef, baseRef) !== null ? "base"
      : tryGit("merge-base", "--is-ancestor", baseRef, mergeBaseRef) !== null ? "mergeBase" : "diverged";

  /**
   * The branch's legacy fork candidates: the newest pre-split commits it is
   * based on through its own history. The split is the commit in onto's history
   * that deleted the monoliths (`monoliths`: package-relative paths); boundary
   * commits of `rev-list <branch> ^<split>` that still have every monolith are
   * the legacy commits the branch grew from. Main's own pre-split commits are
   * excluded because they are reachable from the split. Only a split on onto's
   * first-parent line is trusted.
   */
  function legacyForks(branchRef, ontoRef, monoliths) {
    const files = monoliths.map((file) => `${prefix}/${file}`);
    const split = tryGit("log", "-1", "--format=%H", "--diff-filter=D", ontoRef, "--", ...files);
    // This assumes main is linear (squash merges): if the split reached onto through a merge commit, the
    // boundary could be the split branch's own fork, so only generic guidance is printed.
    if (!split || !(tryGit("rev-list", "--first-parent", ontoRef) ?? "").split("\n").includes(split)) return [];
    const boundary = (tryGit("rev-list", "--boundary", branchRef, `^${split}`) ?? "").split("\n")
      .filter((line) => line.startsWith("-")).map((line) => line.slice(1));
    const legacy = boundary.filter((commit) => files.every((file) => tryGit("cat-file", "-e", `${commit}:${file}`) !== null));
    return legacy.length ? git("merge-base", "--independent", ...legacy).split("\n").filter(Boolean) : [];
  }

  /**
   * Explicit `--branch/--base/--onto`, else the in-progress rebase, cherry-pick or merge (git am has no
   * commit to read). `monoliths` (package-relative legacy catalog paths) lets a merge of a legacy
   * branch into a split one adopt MERGE_HEAD onto HEAD.
   */
  function resolveAdoptRefs(args, usage, { monoliths = [] } = {}) {
    const refs = {};
    for (let i = 0; i < args.length; i += 2) {
      if (!["--branch", "--base", "--onto"].includes(args[i]) || !args[i + 1]) throw new Error(usage);
      refs[args[i].slice(2)] = args[i + 1];
    }
    if (Object.keys(refs).length) {
      if (Object.keys(refs).length !== 3) throw new Error("--branch, --base and --onto must be supplied together");
      return refs;
    }
    const exists = (name) => fs.existsSync(path.resolve(root, git("rev-parse", "--git-path", name)));
    const resolves = (ref) => {
      try { git("rev-parse", "--verify", "--quiet", `${ref}^{commit}`); return true; } catch { return false; }
    };
    const adopt = usage.replace(/^usage: /, "").split(" --branch")[0];
    if (exists("rebase-apply/applying")) {
      throw new Error(`git am is in progress; a mailbox patch has no commit to read the old catalogs from. Pass the legacy commit explicitly (${adopt} --branch <legacy-commit> --base <legacy-commit>^ --onto HEAD), or run git am --abort and merge, rebase or cherry-pick the branch instead.\n${usage}`);
    }
    const command = adopt.replace(/ --$/, "");
    if (exists("rebase-merge") || exists("rebase-apply")) {
      if (!resolves("REBASE_HEAD")) throw new Error(`a rebase is in progress but REBASE_HEAD does not resolve to the commit being applied; pass the refs explicitly (--branch <commit-being-applied> --base <its-parent> --onto HEAD).\n${usage}`);
      refuseRebaseThroughMerges(exists("rebase-merge") ? "rebase-merge" : "rebase-apply", command);
      return { branch: "REBASE_HEAD", base: "REBASE_HEAD^", onto: "HEAD" };
    }
    if (resolves("CHERRY_PICK_HEAD")) {
      if (parentsOf("CHERRY_PICK_HEAD").length > 1) {
        throw new Error(`No files written. CHERRY_PICK_HEAD (${git("rev-parse", "--short", "CHERRY_PICK_HEAD")}) is a merge commit (cherry-pick -m): its changes are relative to the mainline parent you chose, which adoption cannot see. Run git cherry-pick --abort, then cherry-pick the commits the merge brought in, or merge the branch instead.`);
      }
      return { branch: "CHERRY_PICK_HEAD", base: "CHERRY_PICK_HEAD^", onto: "HEAD" };
    }
    if (exists("MERGE_HEAD")) {
      const heads = fs.readFileSync(path.resolve(root, git("rev-parse", "--git-path", "MERGE_HEAD")), "utf8").split("\n").filter(Boolean);
      if (heads.length > 1) throw new Error(`No files written. MERGE_HEAD names ${heads.length} heads (an octopus merge), but adoption reads one branch and one onto at a time; merge one branch at a time and run ${command} after each.`);
      const base = git("merge-base", "HEAD", "MERGE_HEAD");
      // A legacy branch merged into a split one: the incoming side is the branch being adopted. The
      // three-way result is the same either way round, so the roles are swapped rather than refused.
      const hasMonoliths = (ref) => monoliths.some((file) => tryGit("cat-file", "-e", `${ref}:${prefix}/${file}`) !== null);
      if (!hasMonoliths("HEAD") && hasMonoliths("MERGE_HEAD")) {
        console.log("i18n: MERGE_HEAD has the legacy monoliths and HEAD does not, so MERGE_HEAD is adopted onto HEAD (a legacy branch merged into a split one).");
        return { branch: "MERGE_HEAD", onto: "HEAD", base };
      }
      return { branch: "HEAD", onto: "MERGE_HEAD", base };
    }
    throw new Error(`no merge, rebase or cherry-pick detected; provide --branch, --base and --onto.\n${usage}`);
  }

  const parentsOf = (ref) => git("rev-list", "--parents", "-n", "1", ref).split(" ").slice(1);

  /**
   * A rebase that drops merge commits loses their own changes (including how they were resolved),
   * and one that replays a merge (--rebase-merges) applies a merge's resolution, which adoption
   * cannot model; both are refused. The replayed range starts after the first replayed commit's
   * original parent; merges between it and the original head that onto does not contain are dropped.
   */
  function refuseRebaseThroughMerges(dir, command) {
    const file = (name) => path.resolve(root, git("rev-parse", "--git-path", `${dir}/${name}`));
    const read = (name) => fs.existsSync(file(name)) ? fs.readFileSync(file(name), "utf8").trim() : "";
    const guidance = `Run git rebase --abort, then merge main instead (git merge origin/main) and run ${command}, or rebase only the commits after the branch's last merge (git rebase --onto origin/main <last merge>).`;
    const replaysMerges = dir === "rebase-merge" && /^merge /m.test(`${read("git-rebase-todo")}\n${read("done")}`);
    if (parentsOf("REBASE_HEAD").length > 1 || replaysMerges) {
      throw new Error(`No files written. The rebase replays merge commits (--rebase-merges); adoption cannot replay a merge's own resolution. ${guidance}`);
    }
    let first;
    if (dir === "rebase-merge") {
      first = read("done").split("\n").map((line) => line.trim().split(/\s+/)[1]).find((sha) => sha && tryGit("rev-parse", "--verify", "--quiet", `${sha}^{commit}`) !== null);
    } else {
      const patch = fs.existsSync(file("")) ? fs.readdirSync(file("")).filter((name) => /^\d{4}$/.test(name)).sort()[0] : undefined;
      first = patch ? fs.readFileSync(path.join(file(""), patch), "utf8").match(/^From ([0-9a-f]{40}) /)?.[1] : undefined;
    }
    const boundary = parentsOf(first ?? "REBASE_HEAD")[0];
    const origHead = read("orig-head"), ontoCommit = read("onto");
    if (!origHead || !ontoCommit) return;
    const dropped = (tryGit("rev-list", "--merges", origHead, ...(boundary ? [`^${boundary}`] : []), `^${ontoCommit}`) ?? "").split("\n").filter(Boolean);
    if (dropped.length) {
      throw new Error(`No files written. This rebase drops merge commit${dropped.length > 1 ? "s" : ""} ${dropped.map((sha) => sha.slice(0, 9)).join(", ")} from the branch's history, and with ${dropped.length > 1 ? "them" : "it"} its own changes and how it was resolved; adoption cannot recover those. ${guidance}`);
    }
    // Merges below the first replayed commit are not replayed either: git skipped the commits around them
    // (already upstream), or the rebase started after them on purpose (git rebase --onto <onto> <merge>).
    // Adoption cannot tell these apart, so it continues and says what was left behind.
    const below = boundary ? (tryGit("rev-list", "--merges", boundary, `^${ontoCommit}`) ?? "").split("\n").filter(Boolean) : [];
    if (below.length) {
      const names = below.map((sha) => sha.slice(0, 9)).join(", ");
      console.warn(`i18n: notice: this rebase does not replay merge commit${below.length > 1 ? "s" : ""} ${names}, so ${below.length > 1 ? "their" : "its"} catalog resolutions are not carried over (git drops merges when it linearizes a branch). If you rebased from after ${below.length > 1 ? "them" : "it"} on purpose (git rebase --onto origin/main <merge>), nothing is missing. Otherwise run git rebase --abort and merge main instead (git merge origin/main), then ${command}; git rebase --rebase-merges would replay ${below.length > 1 ? "them" : "it"}, but adoption refuses replayed merges.`);
    }
  }

  /**
   * Merges in the branch's own history (not in onto) that joined split areas on both sides whose
   * merge-base had none: the branch had made its own split areas instead of merging them from main.
   * `areaPattern` matches a package-relative split-area file.
   */
  function baseLessAreaMerges(branchRef, ontoRef, areaPattern) {
    const hasAreas = (ref) => (tryGit("ls-tree", "-r", "--name-only", ref, "--", `${prefix}/src/i18n/locales`) ?? "").split("\n")
      .some((file) => areaPattern.test(file.slice(prefix.length + 1)));
    return (tryGit("rev-list", "--merges", "--parents", branchRef, `^${ontoRef}`) ?? "").split("\n").filter(Boolean).map((line) => line.split(" "))
      .filter(([, ...parents]) => {
        if (!parents.every(hasAreas)) return false;
        const shared = tryGit("merge-base", ...parents);
        return !shared || !hasAreas(shared);
      })
      .map(([sha]) => sha.slice(0, 9));
  }

  /**
   * Where the branch's edits begin. Returns `commitLocal` (--base is a parent of --branch: a rebased
   * or cherry-picked commit), the merge-base to compare against (null for an empty base, unrelated
   * histories, or a branch already contained in onto, e.g. a commit re-landed with cherry-pick after
   * a revert, whose merge-base is itself) and `order` (see referenceOrder). Refuses a legacy merge
   * commit adopted against one of its parents (the other side's history would read as the branch's
   * own edits; a mixed one is left to chooseCatalogViews) and a --base off the branch's own history.
   */
  function adoptionReferences(refs, { branchHasAreas }) {
    if (refs.base === "empty") return { commitLocal: false, mergeBaseRef: null, order: "base" };
    const parents = parentsOf(refs.branch);
    const commitLocal = parents.includes(git("rev-parse", `${refs.base}^{commit}`));
    if (commitLocal && !branchHasAreas && parents.length > 1) {
      throw new Error(`No files written. ${refs.branch} (--branch) is a merge commit and ${refs.base} (--base) is one of its parents, so the other side's history would be read as the branch's own edits; adopt the merged branch's own commits, or the branch tip against its legacy fork.`);
    }
    if (tryGit("merge-base", "--is-ancestor", refs.base, refs.branch) === null) {
      throw new Error(`No files written. ${refs.base} (--base) is not an ancestor of ${refs.branch} (--branch), so the branch's edits cannot be measured from it (main's values would read as reverted); pass the branch's legacy fork, or the parent of the commit being adopted.`);
    }
    const shared = mergeBase(refs.branch, refs.onto);
    const mergeBaseRef = shared && shared !== git("rev-parse", `${refs.branch}^{commit}`) ? shared : null;
    return { commitLocal, mergeBaseRef, order: mergeBaseRef ? referenceOrder(refs.base, mergeBaseRef) : "base" };
  }

  return { root, prefix, git, readBlobs, snapshotFiles, resolveAdoptRefs, mergeBase, referenceOrder, legacyForks, baseLessAreaMerges, adoptionReferences };
}

/* ---------- Legacy catalog imports ---------- */

/**
 * Resolve a relative or aliased module specifier to a package-relative POSIX
 * path, independently of the current machine. `aliases` is a list of
 * `[prefix, replacement]` pairs (e.g. `["@/", "src/"]`). Bare package
 * specifiers are returned unchanged.
 */
export function resolveSpecifier(file, specifier, aliases) {
  const alias = aliases.find(([from]) => specifier.startsWith(from));
  if (alias) return path.posix.normalize(`${alias[1]}${specifier.slice(alias[0].length)}`);
  if (specifier.startsWith(".")) return path.posix.normalize(path.posix.join(path.posix.dirname(file), specifier));
  return specifier;
}

/**
 * The replacement for a specifier that names a legacy catalog file exactly
 * (`legacy` maps package-relative legacy paths to their replacement modules),
 * keeping its alias or relative prefix; null for anything else.
 */
export function legacyImportReplacement(file, specifier, legacy, aliases) {
  const resolved = resolveSpecifier(file, specifier, aliases);
  if (!Object.hasOwn(legacy, resolved)) return null;
  const tail = path.posix.basename(resolved);
  if (!specifier.endsWith(`/${tail}`)) return null;
  return `${specifier.slice(0, -tail.length)}${path.posix.relative(path.posix.dirname(resolved), legacy[resolved])}`;
}

/** Cheap text filter: could this source import a legacy catalog? */
export const mayImportLegacyCatalog = (source, legacy) =>
  Object.keys(legacy).some((file) => source.includes(path.posix.basename(file)));

/**
 * Rewrite legacy catalog specifiers in `sources` ([file, text] pairs).
 * `specifiers(file, text)` lists module-specifier string literals as
 * `{ start, end, text, line, attributes? }` (start/end include the quotes;
 * `attributes` is the range of its import attributes clause). Returns the
 * changed texts and one `{ file, line, from, to, removedAttributes }` entry per
 * rewrite; attributes are removed when the replacement is no longer JSON. With
 * no writes it doubles as the check for remaining legacy imports.
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
/**
 * The module-specifier literal of an import/export, import type, import(),
 * require() or vi/jest mock call. `ts` is the caller's TypeScript module.
 */
export function moduleSpecifierOf(ts, node, ast) {
  const specifier = (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) ? node.moduleSpecifier :
    ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword || MODULE_CALLS.test(node.expression.getText(ast))) ? node.arguments[0] :
    ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument) ? node.argument.literal : undefined;
  return specifier && ts.isStringLiteralLike(specifier) ? specifier : undefined;
}

/** A `specifiers(file, source)` lister for rewriteLegacyImports, bound to the caller's TypeScript module. */
export function moduleSpecifierLister(ts) {
  return (file, source) => {
    const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const found = [];
    const visit = (node) => {
      const literal = moduleSpecifierOf(ts, node, ast);
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
  };
}

/** Package-relative `.ts`/`.tsx` sources under `root/dir` that might import a legacy catalog. */
export function legacyImportCandidates(root, dir, legacy) {
  return walkFiles(path.join(root, dir))
    .map((file) => [path.relative(root, file).split(path.sep).join("/"), file])
    .filter(([file]) => /\.tsx?$/.test(file))
    .map(([file, absolute]) => [file, fs.readFileSync(absolute, "utf8")])
    .filter(([, text]) => mayImportLegacyCatalog(text, legacy));
}

/**
 * Rewrite legacy catalog imports in the working tree under `root/dir` (tests
 * included), printing each rewrite, the files to stage, an optional type
 * note and the package's type-check command. Returns the rewritten files.
 */
export function rewriteLegacyImportsInTree({ root, dir, specifiers, legacy, aliases, typecheck, typeNote, log = console.log }) {
  const { changed, rewrites } = rewriteLegacyImports(legacyImportCandidates(root, dir, legacy), specifiers, legacy, aliases);
  for (const [file, text] of changed) fs.writeFileSync(path.join(root, file), text);
  for (const { file, line, from, to, removedAttributes } of rewrites) log(`i18n: rewrote ${file}:${line} ${from} -> ${to}${removedAttributes ? " (removed its JSON import attributes)" : ""}`);
  if (changed.length) {
    log(`i18n: stage the rewritten imports too: git add ${changed.map(([file]) => file).join(" ")}`);
    if (typeNote) log(`i18n: ${typeNote}`);
  }
  log(`i18n: run ${typecheck} to type-check the result.`);
  return changed.map(([file]) => file);
}
