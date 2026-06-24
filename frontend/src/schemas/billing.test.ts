import { describe, expect, it } from "vitest";
import {
  billingUsageResponseSchema,
  billingWalletResponseSchema,
} from "./billing";

describe("billingUsageResponseSchema", () => {
  it("accepts the read-only usage response contract", () => {
    const result = billingUsageResponseSchema.safeParse({
      owner_id: "owner-1",
      period: "7d",
      rows: [
        {
          service_slug: "llm-openai",
          service_id: "svc-1",
          metric: "tokens",
          lago_metric_code: "resale_tokens",
          layer: "resale",
          quantity: 1250,
          requests: 0,
          bytes: 0,
          events: 3,
          lago_acked: true,
          estimated_credits_micros: 2500000,
        },
      ],
      totals: {
        quantity: 1250,
        requests: 0,
        bytes: 0,
        events: 3,
        estimated_credits_micros: 2500000,
      },
      billing: {
        charging_enabled: false,
        lago_configured: true,
        source: "usage_meter",
        rates_are_approximate: true,
      },
    });

    expect(result.success).toBe(true);
  });

  it("rejects unknown metric values", () => {
    expect(
      billingUsageResponseSchema.safeParse({
        owner_id: "owner-1",
        period: "7d",
        rows: [
          {
            service_slug: "svc",
            service_id: "svc-1",
            metric: "usd",
            lago_metric_code: "usd",
            layer: "resale",
            quantity: 1,
            requests: 0,
            bytes: 0,
            events: 1,
            lago_acked: false,
            estimated_credits_micros: null,
          },
        ],
        totals: {
          quantity: 1,
          requests: 0,
          bytes: 0,
          events: 1,
          estimated_credits_micros: null,
        },
        billing: {
          charging_enabled: false,
          lago_configured: false,
          source: "usage_meter",
          rates_are_approximate: true,
        },
      }).success,
    ).toBe(false);
  });
});

describe("billingWalletResponseSchema", () => {
  it("accepts explicit not-configured wallet state", () => {
    expect(
      billingWalletResponseSchema.safeParse({
        owner_id: "owner-1",
        charging_enabled: false,
        lago_configured: true,
        wallet_configured: false,
        status: "not_configured",
        balance_credits: null,
        reserved_credits: null,
        pending_lago_debits: null,
        available_credits: null,
        source: "usage_meter",
        invoices: [],
      }).success,
    ).toBe(true);
  });
});
