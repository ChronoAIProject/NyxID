import { createInstance } from "i18next";
import { expect, it, vi } from "vitest";
import { catalogBackend, registerCatalogLoader, waitForCatalogs } from "./catalog-loader";

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
    maxRetries: 0,
    resources: { en: { "failure-test": { greeting: "Hello" } } },
  });
  await instance.changeLanguage("zh-CN");
  expect(instance.t("greeting")).toBe("Hello");
});

it("retries a transient catalog failure and renders the selected language", async () => {
  let attempts = 0;
  registerCatalogLoader("retry-test", async () => {
    if (++attempts === 1) throw new Error("Temporary connection failure");
    return { greeting: "你好" };
  });
  const instance = createInstance();
  await instance.use(catalogBackend).init({
    lng: "en",
    fallbackLng: "en",
    supportedLngs: ["en", "zh-CN"],
    ns: ["retry-test"],
    defaultNS: "retry-test",
    partialBundledLanguages: true,
    retryTimeout: 1,
    resources: { en: { "retry-test": { greeting: "Hello" } } },
  });
  await instance.changeLanguage("zh-CN");
  expect(attempts).toBe(2);
  expect(instance.t("greeting")).toBe("你好");
});

it("allows startup after a stalled catalog without abandoning its eventual load", async () => {
  vi.useFakeTimers();
  try {
    let complete!: () => void;
    const pending = new Promise<void>((resolve) => { complete = resolve; });
    const ready = waitForCatalogs(pending);
    let rendered = false;
    void ready.then(() => { rendered = true; });
    await vi.advanceTimersByTimeAsync(1499);
    expect(rendered).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(rendered).toBe(true);
    complete();
    await pending;
  } finally {
    vi.useRealTimers();
  }
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
