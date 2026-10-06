import { describe, expect, it } from "vitest";
import type { ServiceBillingExplanation } from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";
import { plainBilling } from "./billing-plain";

const twitter = {
  id: "t",
  label: "Twitter / X API",
  slug: "api-twitter",
  catalog_service_name: "Twitter / X API",
  credential_type: "oauth2",
  credential_source: { type: "personal" },
} as KeyInfo;
const deepseek = {
  id: "d",
  label: "DeepSeek API",
  slug: "llm-deepseek",
  catalog_service_name: "DeepSeek API",
  credential_type: "api_key",
  credential_source: { type: "personal" },
} as KeyInfo;
const bill = (
  overrides: Partial<ServiceBillingExplanation>,
): ServiceBillingExplanation => ({
  status: "conditional",
  credential_class: null,
  credential_label: "",
  account: null,
  charge_status: "conditional",
  rates: [],
  provider_billing: "unknown",
  context: "configuration",
  notes: [],
  ...overrides,
});
const rate = (credits_per_unit: string, metric = "requests") => ({
  layer: "platform",
  metric,
  credits_per_unit,
  currency: "credits",
  source: "configuration",
  sync_status: "synced",
});

describe("plainBilling", () => {
  it("says NyxID-app OAuth with no price is free, naming whose app it is", () => {
    const plain = plainBilling(
      twitter,
      bill({
        credential_class: "nyxid_platform_oauth_app",
        credential_supplier: "nyxid",
        credit_billing_configured: false,
        charge_status: "not_charged",
      }),
    );
    expect(plain).toMatchObject({
      verdict: "free",
      headline: "Free on NyxID",
      detail:
        "This connection uses NyxID's Twitter / X app, and NyxID doesn't charge for it right now.",
      key: { title: "NyxID's Twitter / X app" },
      short: "Free on NyxID",
    });
  });

  it("explains an organization's own app is free on NyxID but billed by the provider", () => {
    const plain = plainBilling(
      {
        ...twitter,
        credential_source: {
          type: "org",
          org_id: "o",
          org_name: "ChronoAI",
        },
      } as KeyInfo,
      bill({
        credential_class: "user_owned",
        credential_supplier: "own",
        credit_billing_configured: false,
      }),
    );
    expect(plain.key.title).toBe("ChronoAI's own Twitter / X app");
    expect(plain.detail).toBe(
      "This connection uses ChronoAI's own app, so NyxID doesn't charge for it. Twitter / X may bill ChronoAI directly.",
    );
  });

  it("states the price and payer for a charged NyxID key in one sentence", () => {
    const plain = plainBilling(
      deepseek,
      bill({
        credential_class: "nyxid_managed_master",
        credential_supplier: "nyxid",
        credit_billing_configured: true,
        rates: [rate("0.000001", "tokens")],
      }),
    );
    expect(plain).toMatchObject({
      verdict: "charged",
      headline: "Uses NyxID credits",
      detail:
        "Each token costs 0.000001 NyxID credits, paid by whoever makes the call, from their personal credits.",
      price: "0.000001 credits per token",
      short: "Caller pays · 0.000001 credits/token",
    });
    expect(plain.tips[0]).toMatch(/free allowances first/);
  });

  it("adds the provider caveat when your own key carries a NyxID fee", () => {
    const plain = plainBilling(
      deepseek,
      bill({
        credential_class: "user_owned",
        credential_supplier: "own",
        credit_billing_configured: true,
        rates: [rate("0.01")],
      }),
    );
    expect(plain.detail).toBe(
      "Each request costs 0.01 NyxID credits, paid by you, from your personal credits. This is a NyxID fee on top of anything DeepSeek bills you directly.",
    );
    expect(plain.short).toBe("You pay · 0.01 credits/request");
  });

  it("does not guess when a paid service's app is unknown", () => {
    const plain = plainBilling(
      twitter,
      bill({ credential_supplier: "unknown", credit_billing_configured: true }),
    );
    expect(plain).toMatchObject({
      verdict: "unconfirmed",
      headline: "Cost not confirmed",
      key: { title: "Not confirmed" },
    });
  });

  it("keeps restricted billing private", () => {
    expect(plainBilling(twitter, bill({ status: "restricted" }))).toMatchObject(
      {
        verdict: "hidden",
        headline: "You can't see billing for this connection",
        short: "Billing unavailable",
      },
    );
  });
});
