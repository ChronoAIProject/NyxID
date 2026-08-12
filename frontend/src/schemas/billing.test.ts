import { describe, expect, it } from "vitest";
import {
  BILLING_USAGE_PERIODS,
  billingReadOnlyBlockSchema,
  billingWalletResponseSchema,
  topUpBillingRequestSchema,
  topUpBillingResponseSchema,
  billingUsageResponseSchema,
  billingUsageRowSchema,
} from "./billing";

describe("billing schemas", () => {
  it("accepts the read-only usage response shape returned by `/billing/usage`", () => {
    const result = billingUsageResponseSchema.safeParse({
      owner_id: "owner-1",
      period: "30d",
      rows: [
        {
          service_slug: "openai",
          service_id: "svc-1",
          metric: "tokens",
          lago_metric_code: "openai.tokens",
          layer: "resale",
          quantity: 42,
          requests: 0,
          bytes: 0,
          events: 2,
          lago_acked: true,
          estimated_credits_micros: 4200,
        },
      ],
      totals: {
        quantity: 42,
        requests: 0,
        bytes: 0,
        events: 2,
        estimated_credits_micros: 4200,
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

  it("keeps the display contract read-only and approximate", () => {
    expect(
      billingReadOnlyBlockSchema.safeParse({
        charging_enabled: true,
        lago_configured: true,
        source: "lago_wallet",
        rates_are_approximate: false,
      }).success,
    ).toBe(false);
  });

  it("lists only backend-supported usage periods", () => {
    expect(BILLING_USAGE_PERIODS).toEqual(["24h", "7d", "30d", "90d", "all"]);
  });

  it("accepts the wallet response shape returned by `/billing/wallet`", () => {
    const result = billingWalletResponseSchema.safeParse({
      owner_id: "owner-1",
      plan_kind: "prepaid",
      collection_state: "good",
      balance_credits: 100,
      reserved_credits: 10,
      pending_lago_debits: 5,
      available_credits: 85,
      available_with_overdraft_credits: 85,
      has_payment_instrument: false,
      overdraft_cap_credits: 0,
      suspended: false,
      lago_customer_id: "customer-1",
      lago_subscription_id: "subscription-1",
      lago_wallet_id: "wallet-1",
      balance_synced_at: "2026-06-26T00:00:00Z",
      created_at: "2026-06-26T00:00:00Z",
      updated_at: "2026-06-26T00:00:00Z",
      created: false,
    });

    expect(result.success).toBe(true);
  });

  it("requires positive top-up credits and parses checkout responses", () => {
    expect(
      topUpBillingRequestSchema.safeParse({
        amount_credits: 0,
        idempotency_key: "topup-12345678",
      }).success,
    ).toBe(false);

    expect(
      topUpBillingResponseSchema.safeParse({
        owner_id: "owner-1",
        amount_credits: 50,
        idempotency_key: "topup-12345678",
        checkout_url: "https://checkout.example.com/session",
        payment_provider: "stripe",
        lago_wallet_transaction_id: "txn-1",
        lago_invoice_id: "invoice-1",
        status: "checkout_created",
        reused: false,
      }).success,
    ).toBe(true);
  });
});

describe("billing token breakdown", () => {
  it("accepts rows with a token breakdown and defaults cache fields", () => {
    const row = billingUsageRowSchema.parse({
      metric: "tokens",
      lago_metric_code: "platform_tokens",
      layer: "platform",
      quantity: 160,
      requests: 0,
      bytes: 0,
      events: 1,
      lago_acked: true,
      token_breakdown: { prompt_tokens: 120, completion_tokens: 40 },
    });
    expect(row.token_breakdown).toEqual({
      prompt_tokens: 120,
      completion_tokens: 40,
      cached_tokens: 0,
      cache_creation_tokens: 0,
    });
  });

  it("accepts rows without a breakdown", () => {
    const row = billingUsageRowSchema.parse({
      metric: "requests",
      lago_metric_code: "platform_requests",
      layer: "platform",
      quantity: 3,
      requests: 3,
      bytes: 0,
      events: 3,
      lago_acked: true,
    });
    expect(row.token_breakdown).toBeUndefined();
  });
});
