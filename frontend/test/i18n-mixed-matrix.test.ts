// @vitest-environment node
/**
 * Adoption matrix (see i18n-mixed-matrix-engine.mjs for the histories, the case enumeration and the
 * model the expected outcomes come from): the exhaustive enumeration of three fixed mixed-branch
 * histories, plus seeded random histories covering legacy and keep-monolith merges in any number
 * and order, merge/cherry-pick/rebase routes, intermediate resolutions, per-language edits, plural
 * families, placement, en-overrides and unreferenced split areas. Test names carry the seed and case.
 */
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { FAMILIES, LANGUAGES, englishOverrides } from "../scripts/lib/i18n-catalogs.mjs";
import { HISTORIES, buildCases, historyFacts, inParallel, keyName, randomCases, randomHistory, rehearse } from "./i18n-mixed-matrix-engine.mjs";

// I18N_MATRIX_SEEDS=<n> runs seeds 1..n (e.g. a deeper sweep before changing the adoption tool).
const RANDOM_SEEDS = Array.from({ length: Number(process.env.I18N_MATRIX_SEEDS ?? 16) }, (_, index) => index + 1);
const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pkg = {
  name: "frontend",
  source: frontend,
  copy: ["adopt-legacy-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"].map((file) => [path.join(frontend, "scripts", file), `frontend/scripts/${file}`]),
  adoptScript: "scripts/adopt-legacy-i18n.mjs",
  adoptCommand: "npm run adopt:i18n --",
  languages: LANGUAGES,
  families: FAMILIES,
  areaRoot: "src/i18n/locales/app",
};
const CATALOGS = ["app", ...FAMILIES];
type Case = { key: string; name: string; catalog: string; plural?: boolean; model: { values: Record<string, Record<string, string>>; owners: Record<string, string>; conflicts: Record<string, string[]>; conflictAt: number | null } };
type Run = Awaited<ReturnType<typeof rehearse>>;
type Plan = { id: string; history: ReturnType<typeof randomHistory>; clean: Case[]; conflicts: Case[]; extra: Case[]; runs?: { clean: Run; conflicts: Run | null } };

const keysOf = (testCase: Case) => testCase.plural ? [keyName(testCase, "one"), keyName(testCase, "other")] : [testCase.key];
const plans: Plan[] = [
  ...Object.entries(HISTORIES).map(([id, history]) => ({ id, history, ...buildCases(history, CATALOGS, LANGUAGES), extra: [] }) as Plan),
  ...RANDOM_SEEDS.map((seed) => {
    const history = randomHistory(seed);
    const { clean, conflicts, sentinels } = randomCases(history, CATALOGS, LANGUAGES, { clean: 30, conflicts: 12 });
    return { id: `seed ${seed}`, history, clean, conflicts, extra: sentinels } as Plan;
  }),
];

beforeAll(async () => {
  await inParallel(plans.map((plan) => async () => {
    const [clean, conflicts] = await Promise.all([
      rehearse(plan.history, plan.clean, pkg),
      plan.conflicts.length ? rehearse(plan.history, [...plan.conflicts, ...plan.extra], pkg) : Promise.resolve(null),
    ]);
    plan.runs = { clean, conflicts };
  }));
}, 180_000);
afterAll(() => { for (const plan of plans) { plan.runs?.clean.cleanup(); plan.runs?.conflicts?.cleanup(); } });

for (const plan of plans) {
  const facts = historyFacts(plan.history);
  describe(`${plan.id} (${plan.history.title})`, () => {
    if (facts.unreferencedAreas) {
      it("refuses as a whole: the branch's split areas have no reference, no command is printed, nothing is written", () => {
        const run = plan.runs!.clean;
        expect(run.final?.status).not.toBe(0);
        expect(run.final?.stderr).toMatch(/neither .* has split areas/);
        expect(run.command).toBeUndefined();
        expect(run.after).toEqual(run.state);
      });
      return;
    }
    if (plan.history.route === "merge" && facts.mixed) {
      it("refuses the auto-detected merge and prints the legacy-fork command", () => {
        const run = plan.runs!.clean;
        expect(run.refused.status).not.toBe(0);
        expect(run.refused.stderr).toContain("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main");
        expect(run.command).toBe(`  npm run adopt:i18n -- --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
      });
    }

    describe(`${plan.clean.length} clean cases`, () => {
      it("adopts without any refusal, duplicate or leftover monolith, and regenerates en-overrides", () => {
        const run = plan.runs!.clean;
        expect(run.final?.status, run.final?.stderr).toBe(0);
        expect(run.monolithsLeft).toEqual([]);
        for (const language of LANGUAGES) expect(run.duplicates(language), language).toEqual([]);
        expect(run.overrides()).toEqual(englishOverrides(run.lookup("app", "en")));
      });
      it.each(plan.clean)("$name", (testCase) => {
        const run = plan.runs!.clean;
        for (const key of keysOf(testCase)) {
          for (const language of LANGUAGES) expect(run.lookup(testCase.catalog, language)[key], `${key} ${language}`).toBe(testCase.model.values[language]![key]);
          if (testCase.catalog === "app" && testCase.model.values[LANGUAGES[0]]![key] !== undefined) expect(run.ownerOf(key), `${key} area`).toBe(testCase.model.owners[key]);
        }
      });
    });

    if (!plan.conflicts.length) return;
    const first = Math.min(...plan.conflicts.map((testCase) => testCase.model.conflictAt!));
    const reached = plan.conflicts.filter((testCase) => testCase.model.conflictAt === first);
    describe(`${plan.conflicts.length} genuine conflicts (${reached.length} refused at the first ${first ? `commit ${first + 1}` : "adoption"})`, () => {
      it("refuses with exactly the conflicting keys and writes nothing", () => {
        const run = plan.runs!.conflicts!;
        expect(run.final?.status).not.toBe(0);
        expect(run.final?.stderr).toContain("No files written.");
        if (plan.history.route === "rebase") expect(run.refusedAt).toBe(first);
        for (const language of LANGUAGES) {
          const expected = reached.flatMap((testCase) => keysOf(testCase).filter((key) => testCase.model.conflicts[language]?.includes(key)));
          expect([...run.refusedKeys(language)].sort(), language).toEqual(expected.sort());
        }
      });
      it.each(reached)("$name", (testCase) => {
        const run = plan.runs!.conflicts!;
        for (const language of LANGUAGES) {
          for (const key of keysOf(testCase).filter((candidate) => testCase.model.conflicts[language]?.includes(candidate))) expect(run.refusedKeys(language), `${key} ${language}`).toContain(key);
        }
      });
    });
  });
}
