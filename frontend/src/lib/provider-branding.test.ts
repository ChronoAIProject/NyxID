import { describe, it, expect } from "vitest";
import { getProviderBrand, hasKnownBrand } from "./provider-branding";

describe("getProviderBrand", () => {
  it("returns OpenAI brand for 'openai' slug", () => {
    const brand = getProviderBrand("openai");
    expect(brand.label).toBe("OpenAI");
    expect(brand.initial).toBe("AI");
    expect(brand.color).toBe("#10a37f");
  });

  it("returns Anthropic brand for 'anthropic' slug", () => {
    const brand = getProviderBrand("anthropic");
    expect(brand.label).toBe("Anthropic");
    expect(brand.initial).toBe("An");
  });

  it("returns Google AI brand for 'google-ai' slug", () => {
    const brand = getProviderBrand("google-ai");
    expect(brand.label).toBe("Google AI");
    expect(brand.initial).toBe("G");
  });

  it("returns Mistral brand for 'mistral' slug", () => {
    const brand = getProviderBrand("mistral");
    expect(brand.label).toBe("Mistral");
    expect(brand.initial).toBe("Mi");
  });

  it("returns Cohere brand for 'cohere' slug", () => {
    const brand = getProviderBrand("cohere");
    expect(brand.label).toBe("Cohere");
    expect(brand.initial).toBe("Co");
  });

  it("returns DeepSeek brand for 'deepseek' slug", () => {
    const brand = getProviderBrand("deepseek");
    expect(brand.label).toBe("DeepSeek");
    expect(brand.initial).toBe("DS");
  });

  it("returns Codex brand for 'openai-codex' slug", () => {
    const brand = getProviderBrand("openai-codex");
    expect(brand.label).toBe("Codex");
    expect(brand.initial).toBe("CX");
  });

  it("returns Twitter brand for 'twitter' slug", () => {
    const brand = getProviderBrand("twitter");
    expect(brand.label).toBe("X (Twitter)");
    expect(brand.initial).toBe("X");
    expect(brand.color).toBe("#000000");
  });

  it("returns Lark brand for 'lark' slug", () => {
    const brand = getProviderBrand("lark");
    expect(brand.label).toBe("Lark");
    expect(brand.initial).toBe("La");
  });

  it("returns Telegram brand for 'telegram' slug", () => {
    const brand = getProviderBrand("telegram");
    expect(brand.label).toBe("Telegram");
    expect(brand.initial).toBe("Tg");
  });

  it("returns default brand for unknown slug", () => {
    const brand = getProviderBrand("unknown-provider");
    expect(brand.label).toBe("");
    expect(brand.initial).toBe("?");
    expect(brand.bgClass).toBe("bg-muted");
    expect(brand.textClass).toBe("text-muted-foreground");
  });
});

describe("hasKnownBrand", () => {
  it("returns true for known slugs", () => {
    expect(hasKnownBrand("openai")).toBe(true);
    expect(hasKnownBrand("anthropic")).toBe(true);
    expect(hasKnownBrand("google-ai")).toBe(true);
    expect(hasKnownBrand("mistral")).toBe(true);
    expect(hasKnownBrand("cohere")).toBe(true);
    expect(hasKnownBrand("openai-codex")).toBe(true);
    expect(hasKnownBrand("deepseek")).toBe(true);
    expect(hasKnownBrand("twitter")).toBe(true);
    expect(hasKnownBrand("lark")).toBe(true);
    expect(hasKnownBrand("telegram")).toBe(true);
  });

  it("returns false for unknown slugs", () => {
    expect(hasKnownBrand("unknown")).toBe(false);
    expect(hasKnownBrand("")).toBe(false);
  });
});
