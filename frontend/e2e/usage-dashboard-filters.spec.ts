import { expect, test } from "@playwright/test";

for (const width of [1440, 390]) {
  test(`usage filters and model grouping work at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/admin/usage?mock=1&sample=operations");
    const filters = page.getByRole("group", { name: "Usage filters" });
    await expect(filters).toBeVisible();
    const tabs = page.getByRole("tablist", { name: "Usage view" });
    expect((await filters.boundingBox())!.y).toBeLessThan(
      (await tabs.boundingBox())!.y,
    );
    await filters.getByRole("combobox", { name: "Time range" }).click();
    await page
      .getByRole("option", { name: "Last 30 days", exact: true })
      .click();
    await filters
      .getByRole("button", { name: "Filter users", exact: true })
      .click();
    await page.getByRole("button", { name: /Alex Chen/ }).click();
    await page.getByRole("button", { name: "Apply", exact: true }).click();
    await filters
      .getByRole("button", {
        name: "Filter users / organizations",
        exact: true,
      })
      .click();
    await page
      .getByRole("button", { name: "Platform Engineering", exact: true })
      .click();
    await page.getByRole("button", { name: "Apply", exact: true }).click();
    const panel = page.getByRole("article", {
      name: "Tokens by model",
      exact: true,
    });
    await panel.scrollIntoViewIfNeeded();
    const table = panel.getByRole("table", { name: "Tokens by model data" });
    await expect(
      table.getByRole("columnheader", { name: "Model", exact: true }),
    ).toBeVisible();
    expect(await table.getByRole("row").count()).toBeGreaterThan(1);
    await expect(panel.locator(".recharts-surface")).toBeAttached();
    await expect(
      page.getByRole("status", { name: "Workspace save status" }),
    ).toContainText("Saved in this browser");
    await page.reload();
    await expect(
      filters.getByRole("combobox", { name: "Time range" }),
    ).toContainText("Last 30 days");
    await expect(
      filters.getByRole("button", { name: "Remove Users filter", exact: true }),
    ).toBeVisible();
    await expect(
      filters.getByRole("button", {
        name: "Remove Users / organizations filter",
        exact: true,
      }),
    ).toBeVisible();
    await panel.scrollIntoViewIfNeeded();
    await panel
      .getByRole("button", { name: "Configure Tokens by model", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Break down by", exact: true })
      .click();
    await page.getByRole("option", { name: "Service", exact: true }).click();
    await page.keyboard.press("Escape");
    await expect(
      table.getByRole("columnheader", { name: "Service", exact: true }),
    ).toBeVisible();
    await filters.scrollIntoViewIfNeeded();
    await filters.getByRole("combobox", { name: "Time range" }).click();
    await page
      .getByRole("option", { name: "Custom range", exact: true })
      .click();
    await expect(
      filters.getByLabel("Start (UTC)", { exact: true }),
    ).toBeVisible();
    await expect(
      filters.getByLabel("End (UTC)", { exact: true }),
    ).toBeVisible();
    expect(
      await page
        .locator("main")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await page.screenshot({ path: `test-results/usage-filters-${width}.png` });
    expect(errors).toEqual([]);
  });
}
