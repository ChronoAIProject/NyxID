import { createInstance } from "i18next";
import { expect, it } from "vitest";
import { catalogBackend, registerCatalogLoader } from "./catalog-loader";

it("retains English copy when a selected catalog cannot be downloaded", async () => {
  registerCatalogLoader("failure-test", async () => {
    throw new Error("Catalog unavailable");
  });
  const instance = createInstance();
  await instance.use(catalogBackend).init({
    lng: "en",
    fallbackLng: "en",
    supportedLngs: ["en", "zh-CN"],
    ns: ["failure-test"],
    defaultNS: "failure-test",
    partialBundledLanguages: true,
    resources: { en: { "failure-test": { greeting: "Hello" } } },
  });
  await instance.changeLanguage("zh-CN");
  expect(instance.t("greeting")).toBe("Hello");
});

it("keeps the latest selection when an earlier catalog download finishes later", async () => {
  let releaseSimplified!: () => void;
  const simplified = new Promise<void>((resolve) => {
    releaseSimplified = resolve;
  });
  registerCatalogLoader("race-test", async (language) => {
    if (language === "zh-CN") await simplified;
    return { greeting: language === "zh-CN" ? "你好" : "您好" };
  });
  const instance = createInstance();
  await instance.use(catalogBackend).init({
    lng: "en",
    fallbackLng: "en",
    supportedLngs: ["en", "zh-CN", "zh-TW"],
    ns: ["race-test"],
    defaultNS: "race-test",
    partialBundledLanguages: true,
    resources: { en: { "race-test": { greeting: "Hello" } } },
  });
  const earlier = instance.changeLanguage("zh-CN");
  await instance.changeLanguage("zh-TW");
  releaseSimplified();
  await earlier;
  expect(instance.language).toBe("zh-TW");
  expect(instance.t("greeting")).toBe("您好");
});
