// @vitest-environment node
/**
 * Exhaustive matrix for the mixed-branch adoption route (see i18n-mixed-matrix-engine.mjs for the
 * histories, the case enumeration and the model the expected outcomes come from).
 */
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { FAMILIES, LANGUAGES } from "../scripts/lib/i18n-catalogs.mjs";
import { HISTORIES, buildCases, expectedValue, rehearse } from "./i18n-mixed-matrix-engine.mjs";

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

for (const [id, history] of Object.entries(HISTORIES)) {
  const { clean, conflicts } = buildCases(history, ["app", ...FAMILIES]);

  describe(`${id} (${history.title}): ${clean.length} clean cases`, () => {
    let run: ReturnType<typeof rehearse>;
    beforeAll(() => { run = rehearse(history, clean, pkg); }, 120_000);
    afterAll(() => run?.cleanup());

    it("refuses the auto-detected merge and prints the legacy-fork command", () => {
      expect(run.refused.status).not.toBe(0);
      expect(run.refused.stderr).toContain("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main");
      expect(run.command).toBe(`  npm run adopt:i18n -- --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
    });

    it("adopts with the printed command, without any refusal, duplicate or leftover monolith", () => {
      expect(run.guided?.status, run.guided?.stderr).toBe(0);
      expect(run.monolithsLeft).toEqual([]);
      for (const language of LANGUAGES) expect(run.duplicates(language), language).toEqual([]);
    });

    it.each(clean)("$name", (testCase) => {
      for (const language of LANGUAGES) expect(run.lookup(testCase.catalog, language)[testCase.key], language).toBe(expectedValue(testCase, language));
    });
  });

  describe(`${id} (${history.title}): ${conflicts.length} genuine conflicts`, () => {
    let run: ReturnType<typeof rehearse>;
    beforeAll(() => { run = rehearse(history, conflicts, pkg); }, 120_000);
    afterAll(() => run?.cleanup());

    it("refuses with exactly the conflicting keys and writes nothing", () => {
      expect(run.command).toBe(`  npm run adopt:i18n -- --branch HEAD --base ${run.fork} --onto MERGE_HEAD`);
      expect(run.guided?.status).not.toBe(0);
      expect(run.guided?.stderr).toContain("No files written.");
      for (const language of LANGUAGES) {
        expect([...new Set(run.refusedKeys(language))].sort(), language).toEqual(conflicts.map((testCase) => testCase.key).sort());
      }
      expect(run.monolithsLeft).toEqual(LANGUAGES);
    });

    it.each(conflicts)("$name", (testCase) => {
      for (const language of LANGUAGES) expect(run.refusedKeys(language), language).toContain(testCase.key);
    });
  });
}
