import { expect, test, type Page } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";
import { usageStats } from "../src/test/admin-usage-fixture";
import { newView } from "../src/lib/usage-analytics";

async function mockUsage(page: Page, withImages = true) {
  await mockDashboard(page);
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
    const measure = new URL(route.request().url()).searchParams.get("measure")!;
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
            label: "Image service",
            is_other: false,
            value,
            points: [point],
          },
        ],
        slices: [
          {
            id: "service",
            label: "Image service",
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
