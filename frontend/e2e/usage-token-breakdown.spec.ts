import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { usageFixture, usageStats } from "../src/test/admin-usage-fixture";
import { newView } from "../src/lib/usage-analytics";

async function mockUsage(page: Page, withImages = true) {
  await mockDashboard(page);
  await page.route("**/api/v1/services", (route) =>
    route.fulfill({ json: { services: [] } }),
  );
  await page.route("**/api/v1/admin/usage/identities?**", (route) =>
    route.fulfill({ json: { options: [], total: 0 } }),
  );
  await page.route("**/api/v1/admin/assistant/feedback?**", (route) =>
    route.fulfill({ json: { counts: [], recent: [], next_cursor: null } }),
  );
  const totals = usageStats({
    prompt_tokens: 4_600_000_000,
    completion_tokens: 1_500_000_000,
    total_tokens: 6_100_000_000,
    cached_tokens: 300_000_000,
    cache_creation_tokens: 0,
    image_input_tokens: withImages ? 900_000_000 : 0,
    image_output_tokens: withImages ? 1_000_000_000 : 0,
  });
  await page.route("**/api/v1/admin/usage/workspace", (route) =>
    route.fulfill({
      json: {
        revision: 1,
        config: { version: 1, draft: newView(), saved_views: [] },
      },
    }),
  );
  await page.route("**/api/v1/admin/usage/analytics?**", (route) => {
    const params = new URL(route.request().url()).searchParams;
    const measure = params.get("measure")!;
    const label =
      params.get("breakdown") === "model" ? "gpt-6-luna" : "Image service";
    const value = measure.endsWith("_tokens")
      ? totals[measure as keyof typeof totals]
      : measure === "requests"
        ? totals.requests
        : totals.gross_cost_micros;
    const point = {
      bucket: "2026-10-06T00:00:00Z",
      value,
      requests: totals.requests,
      unknown_cost_events: 0,
    };
    const responseTotals = { ...totals } as Record<string, unknown>;
    if (!withImages) {
      delete responseTotals.image_input_tokens;
      delete responseTotals.image_output_tokens;
    }
    return route.fulfill({
      json: {
        window: {
          from: "2026-10-06T00:00:00Z",
          to: "2026-10-07T00:00:00Z",
          period: "24h",
        },
        freshness: {
          rolled_up_through: "2026-10-07T00:00:00Z",
          tail_rows: 0,
          validated: true,
        },
        measure,
        metric: "tokens",
        unit: measure.endsWith("_tokens")
          ? "tokens"
          : measure === "requests"
            ? "requests"
            : "microcredits",
        breakdown: "service",
        granularity: "hour",
        total: value,
        totals: responseTotals,
        points: [point],
        series: [
          {
            id: "service",
            label,
            is_other: false,
            value,
            points: [point],
          },
        ],
        slices: [
          {
            id: "service",
            label,
            value,
            requests: totals.requests,
            unknown_cost_events: 0,
            is_other: false,
          },
        ],
      },
    });
  });
}

for (const theme of ["light", "dark"] as const) {
  test(`image token subsets stay within Total and the hover breakdown fits ${theme} mode`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await mockUsage(page);
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto("/admin/usage");
    await page.evaluate((theme) => {
      document.documentElement.classList.remove("theme-light", "theme-dark");
      document.documentElement.classList.add(`theme-${theme}`);
    }, theme);
    const value = page.getByRole("button", { name: /Token count breakdown/ });
    await expect(value).toHaveText("6.1B");
    await value.hover();
    const tooltip = page.getByRole("tooltip");
    await expect(tooltip).toContainText(
      "4,600,000,000 + 1,500,000,000 = 6,100,000,000",
    );
    await expect(tooltip).toContainText("Image input900,000,000");
    await expect(tooltip).toContainText("Image output1,000,000,000");
    await expect(tooltip.locator("..")).toHaveCSS("opacity", "1");
    await page.screenshot({
      path: `test-results/usage-token-breakdown-${theme}.png`,
    });
    await page.keyboard.press("Escape");
    await page.getByRole("button", { name: "Summary token types" }).click();
    await page.getByRole("checkbox", { name: "Cache-read tokens" }).click();
    await page.keyboard.press("Escape");
    await expect(value).toHaveText("6.1B");
    expect(errors).toEqual([]);
  });
}

test.describe("mobile token breakdown", () => {
  test.use({ isMobile: true, hasTouch: true });
  for (const width of [390, 320]) {
    test(`token breakdown opens by tap at ${width}px and preserves older backend totals`, async ({
      page,
    }) => {
      await mockUsage(page, false);
      await page.setViewportSize({ width, height: 844 });
      await page.goto("/admin/usage");
      const value = page.getByRole("button", { name: /Token count breakdown/ });
      await expect(value).toHaveText("6.1B");
      await value.tap();
      const tooltip = page.getByRole("tooltip");
      await expect(tooltip).toContainText(
        "No separate image or voice breakdown was recorded",
      );
      await expect(tooltip).not.toContainText("Image input");
      await expect(tooltip.locator("..")).toHaveCSS("opacity", "1");
      const bounds = (await tooltip.boundingBox())!;
      expect(bounds.x).toBeGreaterThanOrEqual(0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(width);
      await page.screenshot({
        path: `test-results/usage-token-breakdown-mobile-${width}.png`,
      });
    });
  }
});

test("top-level filters scope dashboard, model charts, and usage records together", async ({
  page,
}) => {
  await mockUsage(page);
  const listRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname.endsWith("/admin/usage"))
      listRequests.push(request.url());
  });
  const actor = "11111111-1111-4111-8111-111111111111";
  const owner = "22222222-2222-4222-8222-222222222222";
  await page.route("**/api/v1/services", (route) =>
    route.fulfill({
      json: {
        services: [
          {
            id: "chrono-llm-public",
            name: "Chrono LLM (public)",
            slug: "chrono-llm-public",
          },
        ],
      },
    }),
  );
  await page.route("**/api/v1/admin/usage/identities?**", (route) => {
    const params = new URL(route.request().url()).searchParams;
    const options = [
      { id: actor, label: "Alice" },
      { id: owner, label: "Research org" },
    ].filter((option) =>
      params.has("ids")
        ? params.get("ids")!.split(",").includes(option.id)
        : option.id === (params.get("kind") === "actors" ? actor : owner),
    );
    return route.fulfill({ json: { options, total: options.length } });
  });
  await page.route("**/api/v1/admin/usage?**", (route) =>
    route.fulfill({ json: usageFixture() }),
  );
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/admin/usage");
  const filters = page.getByRole("group", { name: "Usage filters" });
  await expect(filters).toBeVisible();
  const filterBounds = (await filters.boundingBox())!;
  const tabsBounds = (await page
    .getByRole("tablist", { name: "Usage view" })
    .boundingBox())!;
  expect(filterBounds.y + filterBounds.height).toBeLessThan(tabsBounds.y);
  for (const [button, option] of [
    ["Filter users", "Alice"],
    ["Filter users / organizations", "Research org"],
    ["Filter services", "Chrono LLM (public)"],
  ]) {
    await page.getByRole("button", { name: button, exact: true }).click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: option })
      .click();
    await page.getByRole("button", { name: "Apply", exact: true }).click();
  }
  await page.getByRole("combobox", { name: "Time range" }).click();
  const analytics = page.waitForRequest((request) => {
    const url = new URL(request.url());
    return (
      url.pathname.endsWith("/usage/analytics") &&
      url.searchParams.get("period") === "30d"
    );
  });
  await page.getByRole("option", { name: "Last 30 days" }).click();
  const scope = (url: string) => {
    const params = new URL(url).searchParams;
    expect(params.get("actors")).toBe(actor);
    expect(params.get("owners")).toBe(owner);
    expect(params.get("services")).toBe("chrono-llm-public");
    expect(params.get("period")).toBe("30d");
  };
  scope((await analytics).url());
  await page
    .getByRole("combobox", { name: "Break down Request traffic by" })
    .click();
  const regrouped = page.waitForRequest((request) => {
    const params = new URL(request.url()).searchParams;
    return (
      new URL(request.url()).pathname.endsWith("/usage/analytics") &&
      params.get("measure") === "requests" &&
      params.get("breakdown") === "model"
    );
  });
  await page.getByRole("option", { name: "Model", exact: true }).click();
  scope((await regrouped).url());
  const modelPanel = page.getByRole("article", { name: "Tokens by model" });
  await modelPanel.scrollIntoViewIfNeeded();
  await expect(
    modelPanel.getByRole("combobox", { name: "Break down Tokens by model by" }),
  ).toHaveText("Model");
  await expect(modelPanel.getByRole("table")).toContainText("gpt-6-luna");
  await expect(
    modelPanel.getByRole("columnheader", { name: "Model", exact: true }),
  ).toBeVisible();
  // Model identifiers must never be applied as billing-owner UUID filters.
  await expect(
    modelPanel.getByRole("button", { name: "gpt-6-luna", exact: true }),
  ).toHaveCount(0);
  await page.screenshot({ path: "test-results/usage-model-breakdown.png" });
  await page.getByRole("tab", { name: "List", exact: true }).click();
  // The dashboard preloads list data, so switching tabs can reuse that request.
  await expect
    .poll(() =>
      listRequests.some(
        (url) => new URL(url).searchParams.get("period") === "30d",
      ),
    )
    .toBe(true);
  scope(
    listRequests.findLast(
      (url) => new URL(url).searchParams.get("period") === "30d",
    )!,
  );
  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  await expect(
    filters.getByRole("combobox", { name: "Time range" }),
  ).toHaveText("Last 30 days");
  await page.setViewportSize({ width: 390, height: 844 });
  await filters.scrollIntoViewIfNeeded();
  await expect(filters).toBeVisible();
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(390);
  await page.screenshot({ path: "test-results/usage-filters-mobile.png" });
});
