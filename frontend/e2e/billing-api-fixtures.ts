import { type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import {
  billingAllowance,
  billingCatalog,
  billingGrant,
  billingRow,
  billingUsage,
  billingWallet,
  billingAgentUsage,
} from "../src/test/billing-fixture";

export function benefitPreview(
  allowances = [
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "images",
  ].map((metric) => billingAllowance(metric as Parameters<typeof billingAllowance>[0])),
  page = 1,
) {
  return {
    summary: {
      remaining: "2",
      unreserved: "1.5",
      held: "0.5",
      pending_activation: "0",
      pending_reclaim: "0",
      expired_awaiting_cleanup: "0",
      restricted: "0",
      generated_at: "2026-10-01T00:00:00Z",
    },
    grants: [{ ...billingGrant({ expires_at: null }), available: "1.5", held: "0.5" }],
    allowances,
    total_grants: 1,
    page,
    per_page: 50,
  };
}

export async function billingApi(page: Page) {
  await mockDashboard(page);
  await page.route("**/api/v1/users/me", (route) =>
    route.fulfill({
      json: {
        id: "test-user",
        email: "test@example.test",
        display_name: "Billing test",
        is_admin: false,
        is_active: true,
        email_verified: true,
        created_at: "2026-09-01T00:00:00Z",
        onboarding_completed: true,
        capabilities: { billing_available: true },
      },
    }),
  );
  await page.route("**/api/v1/catalog?**", (route) =>
    route.fulfill({ json: { entries: billingCatalog } }),
  );
  await page.route("**/api/v1/api-keys/usage?**", (route) =>
    route.fulfill({
      json: {
        usage: [billingAgentUsage()],
        days: 7,
        since: "2026-10-01T00:00:00Z",
      },
    }),
  );
  await page.route("**/api/v1/billing/wallet", (route) => route.fulfill({ json: billingWallet() }));
  await page.route("**/api/v1/billing/benefits?**", (route) =>
    route.fulfill({ json: benefitPreview() }),
  );
  await page.route("**/api/v1/billing/grants", (route) =>
    route.fulfill({
      json: { grants: [billingGrant()], page: 1, per_page: 50, total: 1 },
    }),
  );
  await page.route("**/api/v1/billing/allowances", (route) =>
    route.fulfill({
      json: {
        allowances: (
          [
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "images",
          ] as const
        ).map((metric) => billingAllowance(metric)),
      },
    }),
  );
  await page.route("**/api/v1/billing/usage?**", (route) =>
    route.fulfill({
      json: billingUsage(
        new URL(route.request().url()).searchParams.get("period") === "24h"
          ? []
          : [
              billingRow(),
              billingRow({
                service_slug: "free-service",
                billable: false,
                estimated_credits_micros: 0,
                grant_credits_micros: 0,
              }),
            ],
      ),
    }),
  );
  await page.route("**/api/v1/billing/topups?**", (route) =>
    route.fulfill({
      json: {
        owner_id: "test-user",
        topups: [
          {
            id: "purchase",
            created_at: "2026-09-25T00:00:00Z",
            amount_credits: 100,
            status: "paid",
            receipt_available: true,
            lago_invoice_id: "invoice",
            invoice_number: "INV-1",
            credits_expire_at: "2027-09-25T00:00:00Z",
          },
        ],
        page: 1,
        per_page: 10,
        total: 1,
      },
    }),
  );
}
