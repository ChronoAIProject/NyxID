import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  initialLanguage,
  LANGUAGE_STORAGE_KEY,
  resolveLanguage,
} from "./language";

beforeEach(() => {
  localStorage.clear();
  vi.spyOn(navigator, "languages", "get").mockReturnValue(["en-US"]);
});
afterEach(() => vi.restoreAllMocks());

describe("supported language negotiation", () => {
  it.each([
    ["en", "en"],
    ["en-GB", "en"],
    ["zh", "zh-CN"],
    ["zh-CN", "zh-CN"],
    ["zh-SG", "zh-CN"],
    ["zh-Hans", "zh-CN"],
    ["zh-Hans-TW", "zh-CN"],
    ["zh-TW", "zh-TW"],
    ["zh-HK", "zh-TW"],
    ["zh-MO", "zh-TW"],
    ["zh-Hant", "zh-TW"],
    ["zh-Hant-CN", "zh-TW"],
    ["zh_HK", "zh-TW"],
  ])("maps %s to %s, respecting explicit scripts", (input, expected) => {
    expect(resolveLanguage(input)).toBe(expected);
  });

  it.each(["fr", "invalid_locale!", "", null, undefined])(
    "ignores unsupported value %s",
    (value) => {
      expect(resolveLanguage(value)).toBeUndefined();
    },
  );

  it("uses the first supported browser preference", () => {
    vi.spyOn(navigator, "languages", "get").mockReturnValue([
      "fr-FR",
      "zh-HK",
      "en",
    ]);
    expect(initialLanguage()).toBe("zh-TW");
  });

  it("restores an explicit choice before browser preferences", () => {
    vi.spyOn(navigator, "languages", "get").mockReturnValue(["zh-TW"]);
    localStorage.setItem(LANGUAGE_STORAGE_KEY, "en");
    expect(initialLanguage()).toBe("en");
  });

  it("keeps the previous onboarding choice until a shared choice is saved", () => {
    localStorage.setItem("nyxbot-onboarding-language", "zh-CN");
    expect(initialLanguage()).toBe("zh-CN");
    localStorage.setItem(LANGUAGE_STORAGE_KEY, "zh-TW");
    expect(initialLanguage()).toBe("zh-TW");
  });

  it("falls back from corrupt preferences and unavailable storage", () => {
    vi.spyOn(navigator, "languages", "get").mockReturnValue(["zh-SG"]);
    localStorage.setItem(LANGUAGE_STORAGE_KEY, "unsupported");
    expect(initialLanguage()).toBe("zh-CN");
    vi.spyOn(localStorage, "getItem").mockImplementation(() => {
      throw new Error("Storage denied");
    });
    expect(initialLanguage()).toBe("zh-CN");
  });
});
