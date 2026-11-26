import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES, legacyImportReplacement, moduleSpecifiers, rewriteLegacyImports,
  analyzePortions, applyDelta, applyMixedDelta, canonicalText, catalogImportIssue, layoutReference,
  consolidateDuplicates, emptyLanguages, isPlacementSource, loadPortions, placeCatalogs, scanUsage, stripPluralSuffix, writePortions,
} from "../scripts/lib/i18n-catalogs.mjs";

describe("legacy i18n delta adoption", () => {
  it("applies independent additions, base changes, and removals", () => {
    const result = applyDelta(
      { "Base value": "old", "Removed value": "gone" },
      { "Base value": "new", "Added value": "added" },
      { "Base value": "old", "Removed value": "gone", "Onto value": "kept" },
    );
    expect(result.conflicts).toEqual([]);
    expect(result.result).toEqual({ "Base value": "new", "Added value": "added", "Onto value": "kept" });
  });

  it("reports a conflict when onto changed the same base value", () => {
    const result = applyDelta({ Key: "base" }, { Key: "branch" }, { Key: "onto" }, "en");
    expect(result.result).toEqual({ Key: "onto" });
    expect(result.conflicts).toEqual(["en: changed key diverged: Key"]);
  });

  it("lets a mixed branch's area edits take precedence and reports monolith/area contradictions", () => {
    const labels = { monolith: "app/en", areas: "app split areas/en" };
    // Main changed K before the merge; the branch removed K in both layouts: the area edit (made against main's value) wins.
    expect(applyMixedDelta({ monolithBase: { K: "init" }, monolith: {}, areaBase: { K: "main" }, areas: {}, onto: { K: "main" }, labels }))
      .toEqual({ result: {}, conflicts: [] });
    // The monoliths removed K while the areas (whose reference still has K) changed it: the branch contradicts itself.
    expect(applyMixedDelta({ monolithBase: { K: "init" }, monolith: {}, areaBase: { K: "init" }, areas: { K: "again" }, onto: { K: "init" }, labels }).conflicts)
      .toEqual(["app split areas/en: monoliths and split areas changed differently: K"]);
    // N14: the monolith change equals the areas reference (main made the same change before the merge),
    // so it is not a branch edit and the later area edit wins.
    expect(applyMixedDelta({ monolithBase: { K: "init" }, monolith: { K: "same" }, areaBase: { K: "same" }, areas: { K: "area" }, onto: { K: "same" }, labels }))
      .toEqual({ result: { K: "area" }, conflicts: [] });
    expect(applyMixedDelta({ monolithBase: { K: "init" }, monolith: {}, areaBase: {}, areas: { K: "again" }, onto: {}, labels }))
      .toEqual({ result: { K: "again" }, conflicts: [] });
    // Monolith-only edits still use the legacy fork as their base.
    expect(applyMixedDelta({ monolithBase: { K: "init" }, monolith: { K: "branch" }, areaBase: { K: "init" }, areas: { K: "init" }, onto: { K: "init", N: "main" }, labels }))
      .toEqual({ result: { K: "branch", N: "main" }, conflicts: [] });
  });

  it("diffs each layout against the newer of --base and the merge-base that has it", () => {
    const catalog = (value: string) => ({ en: { K: value } });
    const fork = { monolith: catalog("fork"), areas: null, familyPresent: { common: true }, families: { common: catalog("fork family") } };
    const merged = { monolith: null, areas: catalog("merged"), familyPresent: { common: true }, families: { common: catalog("merged family") } };
    const parent = { monolith: catalog("parent"), areas: catalog("parent areas"), familyPresent: { common: true }, families: { common: catalog("parent family") } };
    // The legacy-fork route: monoliths from the fork, areas and families from the newer merge-base.
    expect(layoutReference("monolith", { base: fork, mergeBase: merged, order: "mergeBase" })).toEqual(catalog("fork"));
    expect(layoutReference("areas", { base: fork, mergeBase: merged, order: "mergeBase" })).toEqual(catalog("merged"));
    expect(layoutReference("common", { base: fork, mergeBase: merged, order: "mergeBase" })).toEqual(catalog("merged family"));
    // A rebased or cherry-picked commit: its parent is newer than the merge-base for every layout.
    for (const layout of ["monolith", "areas", "common"]) expect(layoutReference(layout, { base: parent, mergeBase: merged, order: "base" })).toBe(layout === "monolith" ? parent.monolith : layout === "areas" ? parent.areas : parent.families.common);
    // A missing reference is null (refused by the caller), never an empty catalog.
    expect(layoutReference("areas", { base: fork, mergeBase: null, order: "base" })).toBeNull();
    expect(layoutReference("areas", { base: fork, mergeBase: merged, order: "diverged" })).toBeNull();
  });

  it("places keys from a mixed branch's area edits in the branch's area unless onto owns the key or a sibling", () => {
    const merged = Object.fromEntries(["en", "zh-CN", "zh-TW"].map((language) => [language, { New: "n", Owned: "o", Count_one: "one", Count_other: "other", Fresh_one: "f1", Fresh_other: "f2" }]));
    const owned = emptyLanguages();
    for (const language of ["en", "zh-CN", "zh-TW"]) { owned[language].Owned = "old"; owned[language].Count_other = "x"; }
    const preferred = new Map([["New", "settings"], ["Owned", "settings"], ["Count_one", "settings"], ["Fresh_one", "settings"]]);
    const result = placeCatalogs(merged, [], { billing: owned }, preferred);
    expect(Object.keys(result.billing.en).sort()).toEqual(["Count_one", "Count_other", "Owned"]);
    expect(Object.keys(result.settings.en).sort()).toEqual(["Fresh_one", "Fresh_other", "New"]);
  });

  it("is idempotent when onto already contains the requested result", () => {
    expect(applyDelta({ Key: "base" }, { Key: "branch" }, { Key: "branch" }).conflicts).toEqual([]);
    expect(applyDelta({ Removed: "base" }, {}, {}).conflicts).toEqual([]);
  });


  it("co-locates plural siblings and promotes keys used by several areas", () => {
    const usage = scanUsage(
      [
        ["src/components/assistant/chat-view.tsx", `t("Requests_one"); t("Requests_other"); t("Shared");`],
        ["src/pages/billing.tsx", `t("Shared");`],
      ],
      ["Requests_one", "Requests_other", "Shared", "Unseen"],
    );
    expect(stripPluralSuffix("Requests_one")).toBe("Requests");
    expect(usage.get("Requests")).toBe("assistant-chat");
    expect(usage.get("Shared")).toBe("shared");
    expect(usage.get("Unseen")).toBe("unattributed");
  });

  it("reserves unattributed exclusively for keys with no literal reference", () => {
    const usage = scanUsage([
      ["src/components/misc-widget.tsx", 'const label = "Referenced";'],
      ["src/pages/platform-setup-detail.tsx", 'const label = "Platform copy";'],
      ["src/pages/billing.test.tsx", 'const label = "Unseen";'],
    ], ["Referenced", "Platform copy", "Unseen"]);
    expect(usage.get("Referenced")).toBe("shared");
    expect(usage.get("Platform copy")).toBe("platform");
    expect(usage.get("Unseen")).toBe("unattributed");
  });

  it("scans only app sources that use the app catalog for placement", () => {
    const excluded = [
      "src/pages/billing.test.tsx", "src/components/__tests__/widget.tsx", "src/test-setup.ts",
      "src/test-utils/options.tsx", "src/dev/scope-preview.tsx", "src/lib/fixtures/billing.ts",
      "src/components/assistant/chat-fixture.ts", "src/hooks/mock-api.ts", "src/lib/mocks.ts",
      "src/features/landing/hero.tsx", "src/features/nyxbot-onboarding/step.tsx",
      "src/i18n/locales/app/zh-CN.ts", "src/pages/billing.json", "src/styles.css",
    ];
    for (const file of excluded) expect(isPlacementSource(file), file).toBe(false);
    for (const file of ["src/pages/billing.tsx", "src/lib/api-error.ts", "src/features/docs/page.tsx", "src/test-setup-helper.ts"]) {
      expect(isPlacementSource(file), file).toBe(true);
    }
    const usage = scanUsage([
      ["src/pages/billing.tsx", 'const label = "Billing only";'],
      ...excluded.map((file): [string, string] => [file, 'const labels = ["Billing only", "Fixture only"];']),
    ], ["Billing only", "Fixture only"]);
    expect(usage.get("Billing only")).toBe("billing");
    expect(usage.get("Fixture only")).toBe("unattributed");
  });

  it("consolidates identical duplicate plural families by language", () => {
    const catalogs = Object.fromEntries(["en", "zh-CN", "zh-TW"].map((language) => [language, { Requests_one: `one ${language}`, Requests_other: `other ${language}` }]));
    const portions = { billing: structuredClone(catalogs), dashboard: structuredClone(catalogs) };
    expect(analyzePortions(portions).issues).toEqual([]);
    expect(analyzePortions(portions).duplicates).toHaveLength(2);
    const next = consolidateDuplicates(portions);
    expect(next.shared).toEqual(catalogs);
    expect(next.billing.en).toEqual({});
    expect(next.dashboard.en).toEqual({});
    portions.dashboard["zh-TW"].Requests_one = "different";
    expect(() => consolidateDuplicates(portions)).toThrow("differing values");
  });

  it("rejects plural siblings stored in different portions", () => {
    const a = emptyLanguages(); const b = emptyLanguages();
    for (const language of ["en", "zh-CN", "zh-TW"]) { a[language].Count_one = "one"; b[language].Count_other = "other"; }
    expect(analyzePortions({ billing: a, dashboard: b }).issues.join("\n")).toContain("plural siblings are not co-located");
  });

  it("preserves existing placement and locates new plural siblings together", () => {
    const original = emptyLanguages(); const merged = emptyLanguages();
    for (const language of ["en", "zh-CN", "zh-TW"]) { original[language].Count_one = "one"; merged[language] = { Count_one: "one", Count_other: "other", Added: "new" }; }
    const result = placeCatalogs(merged, [["src/pages/billing.tsx", 't("Count"); t("Added");']], { dashboard: original });
    expect(result.dashboard.en).toEqual({ Count_one: "one", Count_other: "other" });
    expect(result.billing.en).toEqual({ Added: "new" });
  });

  it("prunes areas emptied by adoption or consolidation instead of writing {} files", () => {
    const locales = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-i18n-prune-"));
    try {
      const full = emptyLanguages(); full.en.Key = "Key"; full["zh-CN"].Key = "键"; full["zh-TW"].Key = "鍵";
      writePortions(locales, { billing: full, dashboard: structuredClone(full) });
      expect(Object.keys(loadPortions(locales))).toEqual(["billing", "dashboard"]);
      writePortions(locales, { billing: full, dashboard: emptyLanguages() });
      expect(Object.keys(loadPortions(locales))).toEqual(["billing"]);
      expect(fs.existsSync(path.join(locales, "app/dashboard"))).toBe(false);
    } finally {
      fs.rmSync(locales, { recursive: true, force: true });
    }
  });

  it("uses code-unit canonical text and exposes duplicate JSON keys", () => {
    expect(canonicalText({ z: "last", A: "first", a: "middle" })).toBe('{\n  "A": "first",\n  "a": "middle",\n  "z": "last"\n}\n');
    const duplicate = '{\n  "A": "first",\n  "A": "second"\n}\n';
    expect(canonicalText(JSON.parse(duplicate))).not.toBe(duplicate);
  });

  it("maps all four legacy catalog paths, keeping alias and relative prefixes", () => {
    const replace = (file: string, specifier: string) => legacyImportReplacement(file, specifier, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES);
    expect(replace("src/pages/a.test.tsx", "@/i18n/locales/zh-CN.json")).toBe("@/i18n/locales/app/zh-CN");
    expect(replace("src/pages/a.test.tsx", "@/i18n/locales/zh-TW.json")).toBe("@/i18n/locales/app/zh-TW");
    expect(replace("src/pages/a.test.tsx", "@/i18n/locales/en.json")).toBe("@/i18n/locales/app/en-full");
    expect(replace("src/pages/a.test.tsx", "@/i18n/locales/en-overrides.json")).toBe("@/i18n/locales/app/en-overrides.json");
    expect(replace("src/pages/nested/a.test.tsx", "../../i18n/locales/zh-TW.json")).toBe("../../i18n/locales/app/zh-TW");
    expect(replace("src/i18n/index.ts", "./locales/en-overrides.json")).toBe("./locales/app/en-overrides.json");
    expect(replace("src/i18n/locales/x.ts", "./en.json")).toBe("./app/en-full");
    for (const specifier of ["@/i18n/locales/common-en.json", "@/i18n/locales/app/zh-CN", "./data/zh-CN.json", "zh-CN.json", "@/i18n/locales/zh-CN.json?raw"]) {
      expect(replace("src/pages/a.test.tsx", specifier), specifier).toBeNull();
    }
  });

  it("rewrites imports, import(), vi.mock and require specifiers only, and is idempotent", () => {
    const source = [
      'import zhCN from "@/i18n/locales/zh-CN.json";',
      "import type en from '../i18n/locales/en.json';",
      'import common from "@/i18n/locales/common-en.json";',
      'const tw = await import("@/i18n/locales/zh-TW.json");',
      'vi.mock("@/i18n/locales/en-overrides.json", () => ({ default: {} }));',
      'const raw = require("../i18n/locales/zh-CN.json");',
      'const label = "@/i18n/locales/zh-CN.json";',
      "",
    ].join("\n");
    const first = rewriteLegacyImports([["src/pages/a.test.tsx", source]], moduleSpecifiers, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES);
    expect(first.rewrites).toEqual([
      { file: "src/pages/a.test.tsx", line: 1, from: "@/i18n/locales/zh-CN.json", to: "@/i18n/locales/app/zh-CN", removedAttributes: false },
      { file: "src/pages/a.test.tsx", line: 2, from: "../i18n/locales/en.json", to: "../i18n/locales/app/en-full", removedAttributes: false },
      { file: "src/pages/a.test.tsx", line: 4, from: "@/i18n/locales/zh-TW.json", to: "@/i18n/locales/app/zh-TW", removedAttributes: false },
      { file: "src/pages/a.test.tsx", line: 5, from: "@/i18n/locales/en-overrides.json", to: "@/i18n/locales/app/en-overrides.json", removedAttributes: false },
      { file: "src/pages/a.test.tsx", line: 6, from: "../i18n/locales/zh-CN.json", to: "../i18n/locales/app/zh-CN", removedAttributes: false },
    ]);
    const [[, rewritten]] = first.changed;
    expect(rewritten.split("\n")).toEqual([
      'import zhCN from "@/i18n/locales/app/zh-CN";',
      "import type en from '../i18n/locales/app/en-full';",
      'import common from "@/i18n/locales/common-en.json";',
      'const tw = await import("@/i18n/locales/app/zh-TW");',
      'vi.mock("@/i18n/locales/app/en-overrides.json", () => ({ default: {} }));',
      'const raw = require("../i18n/locales/app/zh-CN");',
      'const label = "@/i18n/locales/zh-CN.json";',
      "",
    ]);
    const second = rewriteLegacyImports([["src/pages/a.test.tsx", rewritten]], moduleSpecifiers, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES);
    expect(second).toEqual({ changed: [], rewrites: [] });
  });

  it("removes JSON import attributes when the replacement is a module, and keeps them for JSON targets", () => {
    const source = [
      'import zhCN from "@/i18n/locales/zh-CN.json" with { type: "json" };',
      "import en from '../i18n/locales/en.json' assert { type: 'json' };",
      'const tw = await import("@/i18n/locales/zh-TW.json", { with: { type: "json" } });',
      'import overrides from "@/i18n/locales/en-overrides.json" with { type: "json" };',
      'import data from "./data.json" with { type: "json" };',
      "",
    ].join("\n");
    const { changed, rewrites } = rewriteLegacyImports([["src/pages/a.test.tsx", source]], moduleSpecifiers, LEGACY_CATALOG_IMPORTS, SPECIFIER_ALIASES);
    expect(changed[0]?.[1].split("\n")).toEqual([
      'import zhCN from "@/i18n/locales/app/zh-CN";',
      "import en from '../i18n/locales/app/en-full';",
      'const tw = await import("@/i18n/locales/app/zh-TW");',
      'import overrides from "@/i18n/locales/app/en-overrides.json" with { type: "json" };',
      'import data from "./data.json" with { type: "json" };',
      "",
    ]);
    expect(rewrites.map(({ line, removedAttributes }) => [line, removedAttributes])).toEqual([[1, true], [2, true], [3, true], [4, false]]);
  });

  it("guards relative, alias, query-suffixed and full-English imports", () => {
    expect(catalogImportIssue("src/pages/billing.tsx", "@/i18n/locales/app/billing/en.json")).toContain("portion JSON");
    expect(catalogImportIssue("src/pages/billing.tsx", "../i18n/locales/app/en-full?raw")).toContain("test-only");
    expect(catalogImportIssue("src/pages/billing.tsx", "/src/i18n/locales/errors-en.json")).toContain("sanctioned");
    expect(catalogImportIssue("src/i18n/index.ts", "./locales/app/zh-CN")).toBeNull();
    expect(catalogImportIssue("src/i18n/locales/app/zh-CN.ts", "./*/zh-CN.json")).toBeNull();
    expect(catalogImportIssue("src/i18n/resources.test.ts", "./locales/app/en-full")).toBeNull();
  });
});
