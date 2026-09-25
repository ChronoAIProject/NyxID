import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import {
  billingAllowance,
  billingCatalog,
  billingGrant,
  billingRow,
  billingUsage,
  billingWallet,
} from "../src/test/billing-fixture";

async function billingApi(page: Page) {
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
  await page.route("**/api/v1/billing/wallet", (route) =>
    route.fulfill({ json: billingWallet() }),
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

for (const width of [1440, 768, 390, 320]) {
  test(`the actual billing route renders the refreshed design at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1100 });
    await billingApi(page);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/billing");
    await expect(
      page.getByRole("heading", { name: "Billing & Usage", exact: true }),
    ).toBeVisible();
    await expect(page.getByText("95 credits", { exact: true })).toBeVisible();
    await expect(
      page.getByRole("tab", { name: "Billing", exact: true }),
    ).toHaveAttribute("data-state", "active");
    await expect(
      page
        .locator(".compact-benefit-label > strong")
        .filter({ hasText: "Example LLM" }),
    ).toHaveCount(1);
    await expect(page.locator(".benefit-expanded").first()).not.toBeVisible();
    await page.getByRole("img", { name: /Allowance usage/ }).hover();
    const tooltip = page.getByRole("tooltip");
    await expect(tooltip.locator("dl > div")).toHaveCount(5);
    await expect(tooltip.locator("dd")).toHaveText([
      "10%",
      "10%",
      "10%",
      "10%",
      "10%",
    ]);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("tooltip")).toHaveCount(0);
    await page.getByRole("button", { name: "About free usage" }).click();
    await expect(page.getByRole("tooltip")).toContainText(
      "before credit grants",
    );
    await expect(page.locator(".benefit-disclosure[open]")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("tooltip")).toHaveCount(0);
    await page.getByRole("tab", { name: "Usage", exact: true }).click();
    await expect(
      page.getByRole("combobox", { name: "Time filter" }),
    ).toHaveText("Last 30 days");
    await page.getByRole("combobox", { name: "Service filter" }).click();
    await expect(
      page.getByRole("option", { name: "Unused service" }),
    ).toHaveCount(0);
    await page
      .getByRole("option", { name: "Example LLM", exact: true })
      .click();
    await expect(page.locator(".expandable-name > strong")).toHaveText([
      "Example LLM",
    ]);
    await page.reload();
    await expect(
      page.getByRole("combobox", { name: "Service filter" }),
    ).toHaveText("Example LLM");
    await page.getByRole("combobox", { name: "Time filter" }).click();
    await page.getByRole("option", { name: "Last 24 hours" }).click();
    await expect(
      page.getByRole("combobox", { name: "Service filter" }),
    ).toHaveText("All active services");
    await expect(page.getByText("No usage in this period.")).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(errors).toEqual([]);
  });
}

test("top-up and receipt buttons use the real API actions", async ({
  page,
  context,
}) => {
  await billingApi(page);
  await context.route("https://billing.example.test/**", (route) =>
    route.fulfill({ body: "Payment provider response" }),
  );
  const checkouts: Record<string, unknown>[] = [];
  await page.route("**/api/v1/billing/topup", async (route) => {
    expect(route.request().method()).toBe("POST");
    checkouts.push(route.request().postDataJSON());
    await route.fulfill({
      json: {
        owner_id: "test-user",
        checkout_url: "https://billing.example.test/checkout",
        amount_credits: 100,
        idempotency_key: checkouts[0]!.idempotency_key,
        reused: false,
        status: "checkout_created",
      },
    });
  });
  await page.route("**/api/v1/billing/invoices/invoice/download", (route) =>
    route.fulfill({
      json: { file_url: "https://billing.example.test/receipt" },
    }),
  );
  await page.goto("/billing");
  const popupPromise = page.waitForEvent("popup");
  await page.getByRole("button", { name: "Download receipt" }).click();
  const receipt = await popupPromise;
  await expect(receipt).toHaveURL("https://billing.example.test/receipt");
  await receipt.close();
  await page.getByRole("button", { name: "Add credits" }).click();
  await page.getByRole("button", { name: "Continue to payment" }).click();
  await expect(page).toHaveURL("https://billing.example.test/checkout");
  expect(checkouts).toHaveLength(1);
  expect(checkouts[0]).toMatchObject({ amount_credits: 100 });
  expect(checkouts[0]!.idempotency_key).toMatch(/^[\da-f-]{36}$/);
});

test("top-up history pages on the server and resumes a pending payment", async ({
  page,
}) => {
  await billingApi(page);
  const requests: string[] = [];
  await page.route("**/api/v1/billing/topups?**", (route) => {
    const params = new URL(route.request().url()).searchParams;
    requests.push(params.toString());
    const currentPage = Number(params.get("page"));
    return route.fulfill({
      json: {
        owner_id: "test-user",
        page: currentPage,
        per_page: 10,
        total: 11,
        topups: Array.from(
          { length: currentPage === 1 ? 10 : 1 },
          (_, index) => ({
            id: `${currentPage}-${index}`,
            created_at: "2026-09-25T00:00:00Z",
            amount_credits: 100,
            status: currentPage === 1 ? "paid" : "pending",
            receipt_available: false,
            checkout_url:
              currentPage === 2 ? "https://billing.example.test/resume" : null,
          }),
        ),
      },
    });
  });
  await page.route("https://billing.example.test/resume", (route) =>
    route.fulfill({ body: "Payment provider response" }),
  );
  await page.goto("/billing?period=90d");
  await page.getByRole("button", { name: "Next", exact: true }).click();
  await expect(page.getByText("Page 2 of 2")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Next", exact: true }),
  ).toBeDisabled();
  await page.getByRole("combobox", { name: "Top-up history period" }).click();
  await page.getByRole("option", { name: "Last 7 days" }).click();
  await expect(page.getByText("Page 1 of 2")).toBeVisible();
  expect(requests).toEqual(
    expect.arrayContaining([
      "page=1&per_page=10&period=30d",
      "page=2&per_page=10&period=30d",
      "page=1&per_page=10&period=7d",
    ]),
  );
  await expect(page).toHaveURL(/period=90d/);
  await page.getByRole("button", { name: "Next", exact: true }).click();
  await page.getByRole("button", { name: "Resume payment" }).click();
  await expect(page).toHaveURL("https://billing.example.test/resume");
});
