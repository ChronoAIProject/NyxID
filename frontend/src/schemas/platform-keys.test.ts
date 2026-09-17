import { describe, expect, it } from "vitest";
import {
  catalogInferenceSchema,
  inferenceViewSchema,
  lanePriceLabel,
  platformKeyConfigSchema,
} from "./platform-keys";
import { updateServiceSchema } from "./services";
import { catalogServiceActionParamsSchema } from "./assistant-actions";

describe("platform service contracts", () => {
  it("accepts legacy catalog entries and validates protocol and prices", () => {
    expect(catalogInferenceSchema.parse({})).toEqual({});
    expect(
      catalogInferenceSchema.safeParse({
        inference: { wire_protocol: "unknown" },
      }).success,
    ).toBe(false);
    expect(
      inferenceViewSchema.parse({
        wire_protocol: "openai_completions",
        model_list: true,
        realtime: true,
        binding: "platform",
      }).realtime,
    ).toBe(true);
    expect(
      platformKeyConfigSchema.safeParse({
        enabled: true,
        audience: "restricted",
        allowed_owner_ids: ["not-an-owner"],
      }).success,
    ).toBe(false);
    expect(
      updateServiceSchema.safeParse({
        name: "test",
        service_type: "http",
        base_url: "https://example.com",
        byok_pricing: { metric: "requests", credits_per_unit: "-1" },
      }).success,
    ).toBe(false);
  });
  it("represents free and pending lane prices without exposing Lago identifiers", () => {
    expect(lanePriceLabel(null)).toBe("free");
    expect(
      lanePriceLabel({
        metric: "tokens",
        credits_per_unit: "0.125",
        sync_status: "synced",
      }),
    ).toBe("0.125 credits / token");
    expect(
      lanePriceLabel({
        metric: "requests",
        credits_per_unit: "1",
        sync_status: "pending",
      }),
    ).toContain("current billing applies");
  });
  it("accepts the assistant platform choice without changing omitted choices", () => {
    expect(
      catalogServiceActionParamsSchema.parse({
        serviceSlug: "llm-xai",
        use_platform_key: true,
      }).use_platform_key,
    ).toBe(true);
    expect(
      catalogServiceActionParamsSchema.parse({ serviceSlug: "llm-xai" })
        .use_platform_key,
    ).toBeUndefined();
  });
});
