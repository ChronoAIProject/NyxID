import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { analyzePortions, discoverPortions, loadPortions } from "../scripts/lib/i18n-catalogs.mjs";
import en from "../src/i18n/locales/app/en-full";
import zhCN from "../src/i18n/locales/app/zh-CN";
import zhTW from "../src/i18n/locales/app/zh-TW";

const locales = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../src/i18n/locales");

describe("app catalog barrels", () => {
  it("merge areas in the same code-unit area-name order as the catalog tooling", () => {
    const areas = discoverPortions(locales);
    // The case that differs from path order: "assistant" sorts before "assistant-account".
    expect(areas.indexOf("assistant")).toBeLessThan(areas.indexOf("assistant-account"));
    const { merged } = analyzePortions(loadPortions(locales));
    for (const [language, barrel] of [["en", en], ["zh-CN", zhCN], ["zh-TW", zhTW]] as const) {
      expect(Object.keys(barrel), language).toEqual(Object.keys(merged[language]));
    }
  });
});
