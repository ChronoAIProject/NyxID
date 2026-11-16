import { describe, expect, it } from "vitest";
import {
  analyzePortions, applyDelta, canonicalText, catalogImportIssue,
  consolidateDuplicates, emptyLanguages, placeCatalogs, scanUsage, stripPluralSuffix,
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

  it("uses code-unit canonical text and exposes duplicate JSON keys", () => {
    expect(canonicalText({ z: "last", A: "first", a: "middle" })).toBe('{\n  "A": "first",\n  "a": "middle",\n  "z": "last"\n}\n');
    const duplicate = '{\n  "A": "first",\n  "A": "second"\n}\n';
    expect(canonicalText(JSON.parse(duplicate))).not.toBe(duplicate);
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
