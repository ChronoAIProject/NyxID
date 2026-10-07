import { expect, test, type Page, type Locator } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import {
  billingAllowance,
  billingCatalog,
  billingGrant,
  billingRow,
  billingUsage,
  billingWallet,
  billingAgentUsage,
  billingExpandedAllowances,
  billingExpandedRows,
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
  await page.route("**/api/v1/api-keys/usage?**", (route) =>
    route.fulfill({
      json: {
        usage: [billingAgentUsage()],
        days: 7,
        since: "2026-10-01T00:00:00Z",
      },
    }),
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

async function selectMetrics(page: Page, scope: Locator, names: RegExp[]) {
  await scope.getByRole("button", { name: "Filter metrics" }).click();
  const picker = page.getByRole("dialog");
  const clear = picker.getByRole("button", { name: "Clear", exact: true });
  if (await clear.isEnabled()) await clear.click();
  for (const name of names) {
    await picker.getByRole("checkbox", { name }).focus();
    await page.keyboard.press("Enter");
    await expect(picker.getByRole("checkbox", { name })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  }
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await picker.getByRole("button", { name: "Done", exact: true }).click();
}

for (const width of [1440, 768, 390, 320]) {
  test(`expanded allowances and quantities stay compact at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1100 });
    await billingApi(page);
    await page.route("**/api/v1/billing/allowances", (route) =>
      route.fulfill({ json: { allowances: billingExpandedAllowances() } }),
    );
    await page.route("**/api/v1/billing/usage?**", (route) =>
      route.fulfill({ json: billingUsage(billingExpandedRows()) }),
    );
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/billing");
    await expect(
      page.getByRole("tab", { name: "Billing", exact: true }),
    ).toHaveAttribute("data-state", "active");
    await expect(
      page.getByRole("tab", { name: "Billing", exact: true }),
    ).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    const allowance = page.locator(".benefit-disclosure").filter({
      has: page.locator(".compact-benefit-label > strong", {
        hasText: "Example LLM",
      }),
    });
    await allowance.locator("summary").first().click();
    const inspector = page.getByRole("region", {
      name: "Selected allowance balances",
    });
    await expect(inspector.locator(".allowance-balance")).toHaveCount(2);
    await expect(inspector).toContainText("95,250,308");
    await expect(inspector).toContainText("536,202");
    await expect(inspector.getByText("Resets / expires")).toHaveCount(1);
    await expect(
      inspector.getByRole("meter", { name: "input tokens allowance 1 used" }),
    ).toHaveAttribute("aria-valuenow", "100");
    const remainingGauge = inspector.getByRole("meter", {
      name: "input tokens allowance 2 used",
    });
    const primaryColor = await inspector
      .locator(".allowance-gauge-legend .allowance-consumed")
      .evaluate((node) => getComputedStyle(node).backgroundColor);
    await expect(remainingGauge.locator(".allowance-consumed")).toHaveCSS(
      "background-color",
      primaryColor,
    );
    expect(
      await remainingGauge
        .locator(".allowance-reserved")
        .evaluate((node) => getComputedStyle(node).backgroundImage),
    ).toContain("repeating-linear-gradient");
    await selectMetrics(page, allowance, [/^output tokens/, /^images/]);
    await expect(
      inspector.getByText("output tokens", { exact: true }),
    ).toBeVisible();
    await expect(inspector.getByText("images", { exact: true })).toBeVisible();
    await expect(
      inspector.getByText("input tokens", { exact: true }),
    ).toHaveCount(0);
    await expect(inspector.locator(".allowance-balance")).toHaveCount(4);
    for (const header of await inspector
      .locator(".allowance-balance > header")
      .all()) {
      expect(
        await header.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
    }
    expect(
      (await allowance.locator(".benefit-expanded").boundingBox())!.height,
    ).toBeLessThan(1200);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.setViewportSize({ width, height: 2200 });
    await page.mouse.move(0, 0);
    await allowance.screenshot({
      path: `test-results/billing-allowances-expanded-${width}.png`,
      animations: "disabled",
    });
    if (width === 1440) {
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
      await allowance.screenshot({
        path: "test-results/billing-allowances-expanded-dark.png",
        animations: "disabled",
      });
      await page.getByRole("button", { name: "Switch to light mode" }).click();
    }
    await page.getByRole("tab", { name: "Usage", exact: true }).click();
    await expect(
      page.getByRole("tab", { name: "Usage", exact: true }),
    ).toHaveAttribute("data-state", "active");
    await expect(
      page.getByRole("tab", { name: "Usage", exact: true }),
    ).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    await expect(page.locator(".usage-group-disclosure")).not.toHaveAttribute(
      "open",
    );
    await expect(
      page.locator(".usage-explorer .usage-quantity-chart").first(),
    ).toBeVisible();
    await expect(page.locator(".usage-explorer table")).toHaveCount(0);
    await page
      .getByText("Explore by service, model or agent", { exact: true })
      .click();
    const service = page.locator(".expandable-service").filter({
      has: page.locator(".expandable-name > strong", {
        hasText: "Example LLM",
      }),
    });
    await service.locator("summary").first().click();
    const graph = service.locator(".usage-quantity-chart");
    await expect(graph).toBeVisible();
    await expect(graph.locator(".recharts-line-curve")).toHaveCount(1);
    await expect(graph.locator(".recharts-line-dot")).toHaveCount(8);
    await expect(
      graph.getByRole("combobox", { name: "Compare quantities by" }),
    ).toHaveText("Model");
    await expect(graph.locator(".quantity-chart-totals")).toContainText(
      "24,192,034",
    );
    await expect(
      graph.getByRole("button", { name: "Filter metrics" }),
    ).toContainText("1 selected");
    await graph.getByRole("button", { name: "Filter metrics" }).click();
    const picker = page.getByRole("dialog");
    await picker.getByRole("checkbox", { name: /^output tokens/ }).click();
    await expect(graph.locator(".recharts-line-curve")).toHaveCount(2);
    if (width === 1440) {
      await picker.screenshot({
        path: "test-results/billing-metric-picker-light.png",
        animations: "disabled",
      });
    }
    await picker.getByRole("button", { name: "Done", exact: true }).click();
    await expect(
      graph.getByRole("button", { name: "Filter metrics" }),
    ).toContainText("2 selected");
    await graph
      .getByRole("button", {
        name: "Remove metric: output tokens",
        exact: true,
      })
      .click();
    await expect(graph.locator(".recharts-line-curve")).toHaveCount(1);
    await selectMetrics(page, graph, [
      /^input tokens/,
      /^output tokens/,
      /^images/,
    ]);
    const tokens = graph.getByRole("region", {
      name: "tokens comparison",
      exact: true,
    });
    const images = graph.getByRole("region", {
      name: "images comparison",
      exact: true,
    });
    await expect(tokens.locator(".recharts-line-curve")).toHaveCount(2);
    await expect(tokens.locator(".recharts-line-dot")).toHaveCount(16);
    await expect(images.locator(".recharts-line-curve")).toHaveCount(1);
    await expect(images.locator(".quantity-chart-totals dd")).toHaveText("0");
    await expect(tokens.locator(".quantity-chart-totals")).toContainText(
      "2,814,729",
    );
    await expect(page.locator(".recharts-sector")).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.mouse.move(0, 0);
    await service.screenshot({
      path: `test-results/billing-quantities-expanded-${width}.png`,
      animations: "disabled",
    });
    await tokens.getByRole("button", { name: "Next categories" }).click();
    await expect(tokens.locator(".recharts-line-dot")).toHaveCount(6);
    await expect(images.locator(".recharts-line-dot")).toHaveCount(8);
    await graph
      .getByRole("combobox", { name: "Compare quantities by" })
      .click();
    await page.getByRole("option", { name: "Metric", exact: true }).click();
    await expect(tokens.locator(".recharts-line-dot")).toHaveCount(2);
    await expect(images.locator(".recharts-line-dot")).toHaveCount(1);
    await selectMetrics(page, graph, [
      /^input tokens/,
      /^output tokens/,
      /^cache-read tokens/,
      /^cache-write tokens/,
      /^tokens /,
    ]);
    await expect(graph.locator(".recharts-line-dot")).toHaveCount(5);
    await expect(graph.locator(".quantity-chart-totals")).toContainText(
      "837,541,120",
    );
    await graph.getByText("Exact quantities", { exact: true }).click();
    await expect(graph.locator(".quantity-exact-values")).toContainText(
      "24,192,034",
    );
    await expect(graph.locator(".quantity-exact-values")).toContainText(
      "837,541,120",
    );
    await graph
      .getByRole("button", { name: "Clear filters", exact: true })
      .click();
    await expect(
      graph.getByText("Select metrics to display usage"),
    ).toBeVisible();
    await expect(graph.locator(".quantity-unit-panel")).toHaveCount(1);
    await expect(graph.locator(".recharts-line-curve")).toHaveCount(0);
    await expect(graph.locator(".recharts-line-dot")).toHaveCount(0);
    await expect(
      graph.locator(".recharts-cartesian-grid-horizontal line"),
    ).not.toHaveCount(0);
    await expect(graph.locator(".recharts-xAxis-tick-labels")).toContainText(
      "Total",
    );
    await expect(graph.locator(".recharts-xAxis-tick-labels")).toContainText(
      "images",
    );
    await expect(
      graph.locator(
        ".recharts-yAxis-tick-labels .recharts-cartesian-axis-tick-value",
      ),
    ).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.mouse.move(0, 0);
    await graph.screenshot({
      path: `test-results/billing-quantity-empty-${width}.png`,
      animations: "disabled",
    });
    if (width === 1440) {
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
      await graph.screenshot({
        path: "test-results/billing-quantity-empty-dark.png",
        animations: "disabled",
      });
      await page.getByRole("button", { name: "Switch to light mode" }).click();
    }
    await graph
      .getByRole("button", { name: "Choose metrics", exact: true })
      .focus();
    await page.keyboard.press("Enter");
    const emptyPicker = page.getByRole("dialog");
    await expect(
      emptyPicker.getByRole("checkbox", { name: /^input tokens/ }),
    ).not.toBeChecked();
    await emptyPicker.getByRole("checkbox", { name: /^input tokens/ }).click();
    await emptyPicker.getByRole("checkbox", { name: /^output tokens/ }).click();
    await expect(graph.locator(".recharts-line-curve")).toHaveCount(1);
    await emptyPicker
      .getByRole("button", { name: "Done", exact: true })
      .click();
    await expect(
      graph.getByRole("button", { name: "Choose metrics", exact: true }),
    ).toHaveCount(0);
    await expect(graph.locator(".recharts-line-curve")).toHaveCount(1);
    await graph
      .getByRole("combobox", { name: "Compare quantities by" })
      .click();
    await page.getByRole("option", { name: "Model", exact: true }).click();
    await graph
      .getByRole("button", { name: "Clear filters", exact: true })
      .click();
    await expect(graph.getByRole("img")).toHaveAttribute(
      "aria-label",
      /^Empty quantity graph by model: Model 1;.*Model 8/,
    );
    await graph.getByRole("button", { name: "Next categories" }).click();
    await expect(graph.getByRole("img")).toHaveAttribute(
      "aria-label",
      /^Empty quantity graph by model: Model 9; Model 10; Model 11/,
    );
    await expect(
      graph.getByRole("button", { name: "Choose metrics", exact: true }),
    ).toBeVisible();
    await selectMetrics(page, graph, [/^input tokens/, /^output tokens/]);
    if (width === 1440) {
      await graph.getByText("Exact quantities", { exact: true }).click();
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
      await page.mouse.move(0, 0);
      await service.screenshot({
        path: "test-results/billing-quantities-expanded-dark.png",
        animations: "disabled",
      });
      await graph.getByRole("button", { name: "Filter metrics" }).click();
      await page.getByRole("dialog").screenshot({
        path: "test-results/billing-metric-picker-dark.png",
        animations: "disabled",
      });
      await page
        .getByRole("dialog")
        .getByRole("button", { name: "Done", exact: true })
        .click();
      await expect(
        graph.locator(".quantity-chart-totals dd").first(),
      ).toHaveCSS("color", "rgb(239, 239, 241)");
      await expect(
        graph.locator(".recharts-cartesian-axis-tick-value").first(),
      ).toHaveCSS("fill", "rgb(186, 186, 193)");
      await page.getByRole("button", { name: "Switch to light mode" }).click();
    }
    await expect(service.getByRole("tab", { name: /^Records/ })).toHaveCount(0);
    await expect(service.locator(".meter-record")).toHaveCount(0);
    await expect(
      service.getByRole("region", { name: "Funding composition" }),
    ).toBeVisible();
    await expect(service.locator(".funding-track")).toHaveAttribute(
      "aria-label",
      /Funding: Credit grants:/,
    );
    await service.screenshot({
      path: `test-results/billing-visual-expanded-${width}.png`,
      animations: "disabled",
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    expect(errors).toEqual([]);
  });

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
    // #1673 replaced the image with a focusable stacked meter. Keyboard
    // focus exposes the full allowance breakdown for every viewport.
    await page
      .getByRole("meter", { name: "Example LLM free usage used" })
      .focus();
    const tooltip = page.getByRole("tooltip");
    await expect(tooltip.locator("li")).toHaveCount(5);
    await expect(tooltip.locator("li")).toHaveText([
      "Input tokens · 800 of 1K left",
      "Output tokens · 800 of 1K left",
      "Cache-read tokens · 800 of 1K left",
      "Cache-write tokens · 800 of 1K left",
      "Images · 800 of 1K left",
    ]);
    await expect(
      page.getByRole("meter", { name: "Example LLM free usage used" }),
    ).toHaveAttribute(
      "aria-valuetext",
      /Input tokens 10% used; Output tokens 10% used; Cache-read tokens 10% used; Cache-write tokens 10% used; Images 10% used/,
    );
    // The focus-driven full breakdown closes when focus leaves the meter.
    await page.keyboard.press("Tab");
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
      page.locator(".billing-activity .recharts-line-curve"),
    ).toHaveCount(2);
    await expect(page.locator(".billing-page .recharts-sector")).toHaveCount(0);
    await expect(
      page.getByRole("img", {
        name: "Daily activity: 29 requests and 2 errors across 7 UTC days",
      }),
    ).toBeVisible();
    await page
      .locator(".billing-activity")
      .screenshot({ path: `test-results/billing-activity-${width}.png` });
    if (width < 480) await page.setViewportSize({ width, height: 1800 });
    await page
      .locator(".usage-explorer")
      .screenshot({ path: `test-results/billing-usage-${width}.png` });
    if (width < 480) await page.setViewportSize({ width, height: 1100 });
    await page
      .getByText("Explore by service, model or agent", { exact: true })
      .click();
    if (width === 1440) {
      await page.getByRole("button", { name: "Switch to dark mode" }).click();
      const foreground = await page
        .locator(".usage-explorer .usage-explorer-heading")
        .evaluate((node) => getComputedStyle(node).color);
      await expect(page.getByRole("combobox", { name: "Group by" })).toHaveCSS(
        "color",
        foreground,
      );
      await page
        .locator(".billing-activity")
        .screenshot({ path: "test-results/billing-activity-dark.png" });
      await page
        .locator(".usage-explorer")
        .screenshot({ path: "test-results/billing-usage-dark.png" });
      await page.getByRole("button", { name: "Switch to light mode" }).click();
    }
    const serviceSummary = page
      .locator(".expandable-service")
      .filter({
        has: page.locator(".expandable-name > strong", {
          hasText: "Example LLM",
        }),
      })
      .locator("summary")
      .first();
    await serviceSummary.focus();
    await page.keyboard.press("Enter");
    const expanded = page.locator(".expandable-service[open]");
    await expect(expanded.locator(".usage-quantity-chart")).toBeVisible();
    await expect(
      expanded.getByRole("region", { name: "Funding composition" }),
    ).toBeVisible();
    await expect(page.getByRole("tab", { name: /^Records/ })).toHaveCount(0);
    await expect(page.locator(".meter-record")).toHaveCount(0);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(
      page.getByRole("combobox", { name: "Time range", exact: true }),
    ).toHaveText("Last 30 days");
    await page.getByRole("button", { name: "Filter services" }).click();
    await expect(
      page
        .getByRole("dialog")
        .getByRole("checkbox", { name: /Unused service/ }),
    ).toHaveCount(0);
    // Services and Metrics share the same immediate checkbox selection.
    await page
      .getByRole("dialog")
      .getByRole("checkbox", { name: /Example LLM/ })
      .click();
    await expect(
      page.getByRole("button", { name: "Filter services" }),
    ).toContainText("1 selected");
    await expect(
      page
        .getByRole("dialog")
        .getByRole("button", { name: "Apply", exact: true }),
    ).toHaveCount(0);
    await page
      .getByRole("dialog")
      .getByRole("checkbox", { name: /Free service/ })
      .click();
    await expect(
      page.getByRole("button", { name: "Filter services" }),
    ).toContainText("2 selected");
    await expect(
      page.getByRole("button", {
        name: "Remove service: Free service",
        exact: true,
      }),
    ).toBeVisible();
    if (width === 1440) {
      await page
        .getByRole("dialog")
        .screenshot({
          path: "test-results/billing-services-picker-light.png",
          animations: "disabled",
        });
    }

    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Done", exact: true })
      .click();
    await page
      .getByRole("button", {
        name: "Remove service: Free service",
        exact: true,
      })
      .click();
    await expect(page.locator(".expandable-name > strong")).toHaveText([
      "Example LLM",
    ]);
    await page.reload();
    await expect(
      page.getByRole("button", { name: /Edit service filter:.*Example LLM/ }),
    ).toBeVisible();
    await page
      .getByRole("combobox", { name: "Time range", exact: true })
      .click();
    await page.getByRole("option", { name: "Last 24 hours" }).click();
    await expect(
      page.getByRole("button", {
        name: "Remove service: Example LLM",
        exact: true,
      }),
    ).toHaveCount(0);
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
