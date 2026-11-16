/**
 * Mobile adoption matrix, driven by the shared engine (shared/i18n/testing/mixed-matrix-engine.mjs:
 * histories, case enumeration, and the model the expected outcomes come from): the exhaustive
 * enumeration of three fixed mixed-branch histories, plus seeded random histories covering legacy
 * and keep-monolith merges in any number and order, merge/cherry-pick/rebase routes, intermediate
 * resolutions, per-language edits, plural families, placement, re-landed commits and the history
 * shapes adoption refuses as a whole. Each clean case is also checked against model-independent
 * invariants. Test names carry the seed and case. I18N_MATRIX_SEEDS=<n> runs seeds 1..n; repositories
 * run with bounded concurrency (I18N_MATRIX_CONCURRENCY, else the available cores).
 */
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { after, before, describe, test } from "node:test";
import {
  HISTORIES, NOTHING_IN_PROGRESS, REFUSALS, buildCases, cleanupAll, historyFacts, inParallel, invariantViolations, keyName, mergeRoles, randomCases, randomHistory, rehearse,
} from "../../shared/i18n/testing/mixed-matrix-engine.mjs";
import { LANGUAGES, auditCatalogs } from "./lib/i18n-catalogs.mjs";

const RANDOM_SEEDS = Array.from({ length: Number(process.env.I18N_MATRIX_SEEDS ?? 24) }, (_, index) => index + 1);
const mobile = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pkg = {
  name: "mobile",
  source: mobile,
  copy: [
    ...["adopt-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"].map((file) => [path.join(mobile, "scripts", file), `mobile/scripts/${file}`]),
    [path.join(mobile, "../shared/i18n/catalog-core.mjs"), "shared/i18n/catalog-core.mjs"],
  ],
  adoptScript: "scripts/adopt-i18n.mjs",
  adoptCommand: "pnpm adopt:i18n",
  languages: LANGUAGES,
  families: [],
  areaRoot: "src/i18n/locales",
};
const options = { splitLabel: "mobile" };
const keysOf = (testCase) => testCase.plural ? [keyName(testCase, "one"), keyName(testCase, "other")] : [testCase.key];
const plans = [
  ...Object.entries(HISTORIES).map(([id, history]) => ({ id, history, ...buildCases(history, ["app"], LANGUAGES, options), extra: [] })),
  ...RANDOM_SEEDS.map((seed) => {
    const history = randomHistory(seed);
    const { clean, conflicts, sentinels } = randomCases(history, ["app"], LANGUAGES, { clean: 30, conflicts: 12, ...options });
    return { id: `seed ${seed}`, history, clean, conflicts, extra: sentinels };
  }),
];

before(async () => {
  const started = Date.now();
  await inParallel(plans.map((plan) => async () => {
    const [clean, conflicts] = await Promise.all([
      rehearse(plan.history, plan.clean, pkg),
      plan.conflicts.length && !historyFacts(plan.history).refusal ? rehearse(plan.history, [...plan.conflicts, ...plan.extra], pkg) : Promise.resolve(null),
    ]);
    plan.runs = { clean, conflicts };
    if (!historyFacts(plan.history).refusal && clean.final?.status === 0) {
      // One case at a time per history: the histories already run in parallel up to the concurrency bound.
      plan.violations = new Map();
      for (const testCase of plan.clean) plan.violations.set(testCase, await invariantViolations(plan.history, testCase, clean, LANGUAGES));
    }
  }));
  process.stderr.write(`i18n adoption matrix (mobile): ${plans.length} histories rehearsed in ${((Date.now() - started) / 1000).toFixed(1)} s\n`);
}, { timeout: 600_000 });
after(() => cleanupAll());

for (const plan of plans) {
  const facts = historyFacts(plan.history);
  describe(`mobile ${plan.id} (${plan.history.title})`, () => {
    if (facts.refusal) {
      test(`refuses as a whole (${facts.refusal}): no command is printed, nothing is written`, () => {
        const run = plan.runs.clean;
        assert.notEqual(run.final?.status, 0);
        if (run.nothingInProgress) assert.match(run.final.stderr, NOTHING_IN_PROGRESS);
        else {
          assert.match(run.final.stderr, REFUSALS[facts.refusal]);
          assert.ok(run.final.stderr.includes("No files written."), run.final.stderr);
        }
        assert.equal(run.command, undefined);
        assert.deepEqual(run.after, run.state);
      });
      return;
    }
    if (plan.history.route === "merge" && facts.mixed) {
      test("refuses the auto-detected merge and prints the legacy-fork command", () => {
        const run = plan.runs.clean;
        const roles = mergeRoles(plan.history);
        assert.notEqual(run.refused.status, 0);
        assert.ok(run.refused.stderr.includes(`${roles.branch} (--branch) keeps the legacy monoliths beside split areas it inherited from main`), run.refused.stderr);
        assert.equal(run.command, `  pnpm adopt:i18n --branch ${roles.branch} --base ${run.fork} --onto ${roles.onto}`);
      });
    }

    describe(`${plan.clean.length} clean cases`, () => {
      test("adopts into a valid split without any refusal, duplicate or leftover monolith", () => {
        const run = plan.runs.clean;
        assert.equal(run.final?.status, 0, run.final?.stderr);
        assert.deepEqual(run.monolithsLeft, []);
        for (const language of LANGUAGES) assert.deepEqual(run.duplicates(language), [], language);
        assert.deepEqual(auditCatalogs(run.locales), []);
      });
      for (const testCase of plan.clean) {
        test(testCase.name, () => {
          const run = plan.runs.clean;
          for (const key of keysOf(testCase)) {
            for (const language of LANGUAGES) assert.equal(run.lookup("app", language)[key], testCase.model.values[language][key], `${key} ${language}`);
            if (testCase.model.values[LANGUAGES[0]][key] !== undefined) assert.equal(run.ownerOf(key), testCase.model.owners[key], `${key} area`);
          }
          assert.deepEqual(plan.violations?.get(testCase), [], "model-independent invariants");
        });
      }
    });

    if (!plan.conflicts.length) return;
    const first = Math.min(...plan.conflicts.map((testCase) => testCase.model.conflictAt));
    const reached = plan.conflicts.filter((testCase) => testCase.model.conflictAt === first);
    describe(`${plan.conflicts.length} genuine conflicts (${reached.length} refused at the first ${first ? `commit ${first + 1}` : "adoption"})`, () => {
      test("refuses with exactly the conflicting keys and writes nothing", () => {
        const run = plan.runs.conflicts;
        assert.notEqual(run.final?.status, 0);
        assert.ok(run.final.stderr.includes("No files written."), run.final.stderr);
        if (plan.history.route === "rebase") assert.equal(run.refusedAt, first);
        for (const language of LANGUAGES) {
          const expected = reached.flatMap((testCase) => keysOf(testCase).filter((key) => testCase.model.conflicts[language]?.includes(key)));
          assert.deepEqual([...run.refusedKeys(language)].sort(), expected.sort(), language);
        }
      });
      for (const testCase of reached) {
        test(testCase.name, () => {
          const run = plan.runs.conflicts;
          for (const language of LANGUAGES) {
            for (const key of keysOf(testCase).filter((candidate) => testCase.model.conflicts[language]?.includes(candidate))) assert.ok(run.refusedKeys(language).includes(key), `${key} ${language}`);
          }
        });
      }
    });
  });
}

describe("mobile invariants without the model", () => {
  test("flag a cherry-pick that also applies edits from commits it did not pick (the N8 class, injected)", { timeout: 120_000 }, async () => {
    // Inject N8 into the shared core: measure the picked commit from the older merge-base instead of its parent.
    const corePath = path.join(mobile, "../shared/i18n/catalog-core.mjs");
    const core = fs.readFileSync(corePath, "utf8");
    const anchor = 'order: mergeBaseRef ? referenceOrder(refs.base, mergeBaseRef) : "base"';
    assert.ok(core.includes(anchor));
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-mobile-i18n-n8-"));
    try {
      const mutated = path.join(dir, "catalog-core.mjs");
      fs.writeFileSync(mutated, core.replace(anchor, 'order: mergeBaseRef ? "mergeBase" : "base"'));
      // A cherry-pick whose earlier, unpicked commits edited keys the picked commit leaves alone.
      const history = randomHistory(32);
      assert.equal(history.route, "cherryPick");
      const { clean } = randomCases(history, ["app"], LANGUAGES, { clean: 30, conflicts: 0, ...options });
      const run = await rehearse(history, clean, { ...pkg, copy: pkg.copy.map(([from, to]) => [to === "shared/i18n/catalog-core.mjs" ? mutated : from, to]) });
      assert.equal(run.final?.status, 0, run.final?.stderr);
      const violations = [];
      for (const testCase of clean) violations.push(...await invariantViolations(history, testCase, run, LANGUAGES));
      assert.ok(violations.some((violation) => / is neither main's latest .* nor a branch value| removed by main but reappeared /.test(violation)), violations.join("\n"));
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
