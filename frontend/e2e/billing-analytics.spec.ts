import { expect, test, type Page } from "@playwright/test";
import type { WorkspaceResponse } from "../src/schemas/usage-analytics";

async function select(page: Page, name: string, choice: string) {
  await page.getByRole("combobox", { name, exact: true }).last().click();
  await page.getByRole("option", { name: choice, exact: true }).last().click();
}

test("all three analytics approaches are interactive and saved views survive reload", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/admin/usage?mock=1&sample=overview");
  await expect(page.getByText("Recommended", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("img", { name: /Spend over time/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Configure Service mix", exact: true })
    .click();
  await select(page, "Show", "Top 10 + Other");
  await select(page, "Chart type", "Bar");
  await select(page, "Data table", "Accordion");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByRole("img", {
      name: "Service mix: bar chart. Values available in the data table.",
      exact: true,
    }),
  ).toBeVisible();
  const serviceMix = page.getByRole("article", { name: "Service mix" });
  await expect(
    serviceMix.getByRole("table", { name: "Service mix data" }),
  ).toBeHidden();
  await serviceMix.getByText("Data table", { exact: true }).click();
  await expect(
    serviceMix.getByRole("table", { name: "Service mix data" }),
  ).toBeVisible();
  await serviceMix.getByText("Data table", { exact: true }).click();
  await expect(
    serviceMix.getByRole("table", { name: "Service mix data" }),
  ).toBeHidden();
  await page
    .getByRole("button", { name: "Configure Service mix", exact: true })
    .click();
  await select(page, "Data table", "Always open");
  await page.keyboard.press("Escape");
  await expect(
    serviceMix.getByRole("table", { name: "Service mix data" }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Overview GA inspired/ }).click();
  await expect(
    page.getByRole("img", { name: /Service mix: bar chart/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Filter billing accounts", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Platform Engineering", exact: true })
    .click();
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByRole("button", {
      name: "Remove Billing accounts filter",
      exact: true,
    }),
  ).toBeVisible();
  await select(page, "Time range", "Last 30 days");
  await page.getByRole("button", { name: /Saved views/ }).click();
  await page
    .getByRole("textbox", { name: "New saved view name" })
    .fill("Engineering monthly");
  await page
    .getByRole("button", { name: "Save as new view", exact: true })
    .click();
  await expect(
    page.getByRole("status", { name: "Workspace save status" }),
  ).toContainText("Saved in this browser");
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Engineering monthly", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Time range", exact: true }),
  ).toContainText("Last 30 days");
  await expect(
    page.getByRole("button", {
      name: "Remove Billing accounts filter",
      exact: true,
    }),
  ).toBeVisible();

  await page
    .getByRole("button", { name: /Operations Grafana inspired/ })
    .click();
  await page
    .getByRole("button", { name: "Configure Cost & traffic", exact: true })
    .click();
  await select(page, "Measure", "Requests");
  await expect(
    page.getByRole("combobox", { name: "Chart type", exact: true }),
  ).toContainText("Line");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(
    page.getByRole("img", { name: /Request traffic: line/ }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Configure Request traffic", exact: true })
    .click();
  await page.getByRole("button", { name: "Duplicate", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("heading", { name: "Request traffic copy", exact: true }),
  ).toBeVisible();

  await page.getByRole("button", { name: /Explorer PostHog inspired/ }).click();
  await select(page, "Measure", "Billed units");
  await select(page, "Metric unit", "images");
  await select(page, "Chart type", "Donut / pie");
  await expect(
    page.getByRole("img", { name: /Explore service usage: pie/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("table", { name: "Explore service usage data" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Use in Operations", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "My operations", exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

for (const layout of ["overview", "operations", "explorer"] as const) {
  test(`${layout} renders charts without horizontal overflow on mobile`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/admin/usage?mock=1&sample=${layout}`);
    await expect(
      page.getByRole("heading", { name: "Usage", exact: true }),
    ).toBeVisible();
    const firstPanel = page.getByRole("article").first();
    await firstPanel.scrollIntoViewIfNeeded();
    await expect(firstPanel.locator(".recharts-surface")).toBeAttached();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(
      await page
        .locator("main")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
  });
}

test("Operations adds and restores more than twelve panels and loads charts as they enter view", async ({
  page,
}) => {
  await page.goto("/admin/usage?mock=1&sample=operations");
  await expect(page.getByRole("article")).toHaveCount(6);
  for (let index = 0; index < 12; index++) {
    await page.getByRole("button", { name: "Add panel", exact: true }).click();
  }
  await expect(page.getByRole("article")).toHaveCount(18);
  await expect(
    page.getByRole("status", { name: "Workspace save status" }),
  ).toContainText("Saved in this browser");
  await page.reload();
  await expect(page.getByRole("article")).toHaveCount(18);
  await expect(
    page.getByRole("button", { name: "Add panel", exact: true }),
  ).toBeEnabled();
  expect(await page.locator(".recharts-surface").count()).toBeLessThan(18);
  const last = page.getByRole("article").last();
  await last.scrollIntoViewIfNeeded();
  await expect(last.locator(".recharts-surface")).toBeAttached();
  await expect(
    page.getByRole("button", { name: /Operations Grafana inspired/ }),
  ).toHaveAttribute("aria-pressed", "true");
});

test("Operations drag resizing persists and exposes twenty chart colors", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/admin/usage?mock=1&sample=operations");
  const panel = page.locator(".analytics-grid-item").first();
  const grip = page.getByRole("button", { name: "Resize Request traffic" });
  await expect(grip).toBeVisible();
  await grip.scrollIntoViewIfNeeded();
  await expect(grip).toHaveCSS("opacity", "0");
  const width = (await panel.boundingBox())!.width;
  const box = (await grip.boundingBox())!;
  await page.mouse.move(box.x - 12, box.y + box.height / 2);
  await expect(grip).toHaveCSS("opacity", "1");
  expect(
    await page.evaluate(
      ({ x, y }) =>
        document
          .elementFromPoint(x, y)
          ?.closest("button")
          ?.getAttribute("aria-label"),
      { x: box.x + box.width / 2, y: box.y + box.height / 2 },
    ),
  ).toBe("Resize Request traffic");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await expect(panel).toHaveAttribute("data-resizing", "true");
  await page.mouse.move(
    box.x + box.width / 2 + width + 12,
    box.y + box.height / 2 + 80,
    {
      steps: 8,
    },
  );
  await expect(panel).toHaveAttribute("data-span", "2");
  await page.mouse.up();
  await expect(panel.locator(".analytics-plot")).toHaveCSS("height", "280px");
  await expect(
    page.getByRole("status", { name: "Workspace save status" }),
  ).toContainText("Saved in this browser");
  await page.reload();
  await expect(page.locator(".analytics-grid-item").first()).toHaveAttribute(
    "data-span",
    "2",
  );
  await expect(
    page.locator(".analytics-grid-item").first().locator(".analytics-plot"),
  ).toHaveCSS("height", "280px");

  await page.getByRole("button", { name: "Configure Request traffic" }).click();
  await select(page, "Show", "Top 20 + Other");
  await page.evaluate(() => {
    document.documentElement.classList.remove("theme-light");
    document.documentElement.classList.add("theme-dark");
  });
  const colors = await page
    .locator(".analytics-chart")
    .first()
    .evaluate((chart) => {
      const style = getComputedStyle(chart);
      return Array.from({ length: 20 }, (_, index) =>
        style.getPropertyValue(`--analytics-series-${index + 1}`).trim(),
      );
    });
  expect(colors.every(Boolean)).toBe(true);
  expect(new Set(colors).size).toBe(20);
  const appliedColors = await page
    .locator(".analytics-grid-item")
    .first()
    .locator(".analytics-chart span.rounded-full")
    .evaluateAll((dots) =>
      dots.map((dot) => getComputedStyle(dot).backgroundColor),
    );
  expect(new Set(appliedColors).size).toBeGreaterThan(1);
  const lightColors = await page
    .locator(".analytics-chart")
    .first()
    .evaluate((chart) => {
      document.documentElement.classList.remove("theme-dark");
      document.documentElement.classList.add("theme-light");
      const style = getComputedStyle(chart);
      return Array.from({ length: 20 }, (_, index) =>
        style.getPropertyValue(`--analytics-series-${index + 1}`).trim(),
      );
    });
  expect(new Set(lightColors).size).toBe(20);
  expect(lightColors).not.toEqual(colors);
});

test("panel handles and options show action tooltips", async ({ page }) => {
  await page.goto("/admin/usage?mock=1&sample=operations");
  const drag = page.getByRole("button", { name: "Drag Request traffic" });
  await drag.scrollIntoViewIfNeeded();
  await expect(drag).toHaveCSS("opacity", "0");
  const dragBox = (await drag.boundingBox())!;
  await page.mouse.move(dragBox.x - 12, dragBox.y + dragBox.height / 2);
  await expect(drag).toHaveCSS("opacity", "1");
  await drag.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Drag to reorder");
  await page.mouse.move(0, 0, { steps: 10 });
  await expect(page.getByRole("tooltip")).toHaveCount(0);

  const resize = page.getByRole("button", { name: "Resize Request traffic" });
  await resize.scrollIntoViewIfNeeded();
  const resizeBox = (await resize.boundingBox())!;
  await page.mouse.move(resizeBox.x - 12, resizeBox.y + resizeBox.height / 2);
  await expect(resize).toHaveCSS("opacity", "1");
  await expect(resize.locator("svg")).toHaveClass(/lucide-move-diagonal-2/);
  await resize.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Drag to resize");
  await page.mouse.move(0, 0, { steps: 10 });
  await expect(page.getByRole("tooltip")).toHaveCount(0);

  const options = page.getByRole("button", {
    name: "Configure Request traffic",
  });
  await options.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Panel options");
  await options.click();
  await expect(
    page.getByRole("combobox", { name: "Panel width" }),
  ).toBeVisible();
});

for (const choice of ["Use saved view", "Restore recovered draft"] as const) {
  test(`an incomplete recovered draft keeps charts visible until ${choice}`, async ({
    page,
  }) => {
    const key = "nyxid:usage-workspace:v1:demo:design-review-operations";
    await page.goto("/admin/usage?mock=1&sample=operations");
    await select(page, "Time range", "Last 30 days");
    await expect(
      page.getByRole("status", { name: "Workspace save status" }),
    ).toContainText("Saved in this browser");
    await select(page, "Time range", "Last 7 days");
    await expect(
      page.getByRole("status", { name: "Workspace save status" }),
    ).toContainText("Saved in this browser");
    const raw = await page.evaluate((key) => {
      const saved = JSON.parse(
        localStorage.getItem(`${key}:saved`)!,
      ) as WorkspaceResponse;
      if (!saved.config) throw new Error("Expected a saved workspace");
      saved.config.draft.filters = {
        period: "custom",
        from: null,
        to: null,
        services: ["llm-cohere", "aws-cost-explorer"],
        actors: [],
        owners: [],
      };
      saved.revision -= 1;
      const raw = JSON.stringify(saved);
      sessionStorage.setItem(key, raw);
      localStorage.setItem(key, raw);
      return raw;
    }, key);

    for (let reload = 0; reload < 2; reload++) {
      await page.reload();
      await expect(
        page.getByRole("status", { name: "Recovered workspace" }),
      ).toBeVisible();
      await expect(
        page.getByRole("combobox", { name: "Time range", exact: true }),
      ).toContainText("Last 7 days");
      await expect(
        page.getByRole("button", { name: "Add panel", exact: true }),
      ).toBeDisabled();
      await expect(
        page.getByText("Choose a valid time range to query usage.", {
          exact: true,
        }),
      ).toHaveCount(0);
      await expect(page.getByText("Save failed", { exact: true })).toHaveCount(
        0,
      );
      await expect(page.getByRole("article")).toHaveCount(6);
      for (const panel of await page.getByRole("article").all()) {
        await panel.scrollIntoViewIfNeeded();
        await expect(panel.locator(".recharts-surface")).toBeAttached();
      }
      expect(
        await page.evaluate(
          (key) => [sessionStorage.getItem(key), localStorage.getItem(key)],
          key,
        ),
      ).toEqual([raw, raw]);
    }

    await page.getByRole("button", { name: choice, exact: true }).click();
    await expect(
      page.getByRole("status", { name: "Recovered workspace" }),
    ).toHaveCount(0);
    if (choice === "Restore recovered draft") {
      await expect(
        page.getByRole("combobox", { name: "Time range", exact: true }),
      ).toContainText("Custom");
      await expect(
        page
          .getByText("Choose a valid time range to query usage.", {
            exact: true,
          })
          .first(),
      ).toBeVisible();
      await page
        .getByRole("button", { name: "Reset range", exact: true })
        .click();
    }
    await expect(
      page.getByRole("button", { name: "Add panel", exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByRole("status", { name: "Workspace save status" }),
    ).toContainText("Saved in this browser");
    expect(
      await page.evaluate(
        (key) => [sessionStorage.getItem(key), localStorage.getItem(key)],
        key,
      ),
    ).toEqual([null, null]);
    await page.reload();
    await expect(
      page.getByRole("status", { name: "Recovered workspace" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("combobox", { name: "Time range", exact: true }),
    ).toContainText("Last 7 days");
  });
}

test("the live workspace renders API values and keeps the time range on one line", async ({
  page,
}) => {
  await page.route("**/api/v1/admin/usage/workspace", async (route) => {
    const saved =
      route.request().method() === "PUT"
        ? route.request().postDataJSON()
        : null;
    await route.fulfill({
      json: { revision: saved ? 1 : 0, config: saved?.config ?? null },
    });
  });
  await page.route("**/api/v1/admin/usage/analytics?**", async (route) => {
    const measure = new URL(route.request().url()).searchParams.get("measure");
    const total =
      measure === "requests"
        ? 41
        : measure === "total_tokens"
          ? 8200
          : 12_000_000;
    const point = {
      bucket: "2026-09-23T00:00:00Z",
      value: total,
      requests: 41,
      unknown_cost_events: 0,
    };
    await route.fulfill({
      json: {
        window: {
          from: "2026-09-23T00:00:00Z",
          to: "2026-09-24T00:00:00Z",
          period: "24h",
        },
        freshness: {
          rolled_up_through: "2026-09-23T00:00:00Z",
          tail_rows: 0,
          validated: true,
        },
        granularity: "hour",
        unit:
          measure === "requests"
            ? "requests"
            : measure === "total_tokens"
              ? "tokens"
              : "microcredits",
        total,
        totals: {
          requests: 41,
          events: 41,
          quantities: {},
          prompt_tokens: 8000,
          completion_tokens: 200,
          cached_tokens: 0,
          cache_creation_tokens: 0,
          total_tokens: 8200,
          gross_cost_micros: 12_000_000,
          wallet_cost_micros: 12_000_000,
          grant_cost_micros: 0,
          allowance_cost_micros: 0,
          exact_cost_events: 41,
          legacy_cost_events: 0,
          unknown_cost_events: 0,
          unique_users: 1,
          unique_services: 1,
        },
        points: [point],
        series: [
          { label: "API response service", is_other: false, points: [point] },
        ],
        slices: [
          {
            id: null,
            label: "API response service",
            value: total,
            unknown_cost_events: 0,
            is_other: false,
          },
        ],
      },
    });
  });
  await page.goto("/admin/usage?mock=1");
  await expect(
    page.getByText("Automated test fixture.", { exact: true }),
  ).toHaveCount(0);
  const traffic = page.getByRole("article", {
    name: "Request traffic",
    exact: true,
  });
  await traffic.scrollIntoViewIfNeeded();
  await expect(traffic.getByText("41", { exact: true }).first()).toBeVisible();
  await expect(
    traffic.getByText("API response service", { exact: true }).first(),
  ).toBeVisible();
  const label = page.locator("label", { hasText: /^Time range$/ });
  const select = page.getByRole("combobox", {
    name: "Time range",
    exact: true,
  });
  const labelBounds = await label.boundingBox();
  const selectBounds = await select.boundingBox();
  expect(labelBounds).not.toBeNull();
  expect(selectBounds).not.toBeNull();
  expect(
    Math.abs(
      labelBounds!.y +
        labelBounds!.height / 2 -
        selectBounds!.y -
        selectBounds!.height / 2,
    ),
  ).toBeLessThan(2);
  await page.route("**/api/v1/admin/usage/analytics?**", (route) =>
    route.fulfill({ status: 503, json: { error: "Usage unavailable" } }),
  );
  await page.getByRole("button", { name: "Refresh", exact: true }).click();
  await traffic.scrollIntoViewIfNeeded();
  await expect(traffic.getByRole("button", { name: /retry/i })).toBeVisible();
});

test("filter selection is staged separately from the audit-style applied pills", async ({
  page,
}) => {
  await page.goto("/admin/usage?mock=1&sample=operations");
  const pills = page.getByLabel("Applied filters", { exact: true });
  await page
    .getByRole("button", { name: "Filter services", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Anthropic anthropic", exact: false })
    .click();
  await expect(pills).toHaveCount(0);
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(pills).toHaveCount(0);
  await page
    .getByRole("button", { name: "Filter services", exact: true })
    .click();
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "Anthropic anthropic" }),
  ).toHaveAttribute("aria-pressed", "false");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Anthropic anthropic" })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "OpenAI openai" })
    .click();
  await page.getByRole("button", { name: "Apply", exact: true }).click();
  await expect(pills).toContainText("Anthropic, OpenAI");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await pills.getByRole("button", { name: /^Edit Services filter/ }).click();
  await page
    .getByRole("textbox", { name: "Search services", exact: true })
    .fill("Gemini");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Gemini gemini" })
    .click();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(pills).toContainText("Anthropic, OpenAI");
  await expect(
    page.getByRole("status", { name: "Workspace save status" }),
  ).toContainText("Saved in this browser");
  await page.reload();
  await expect(pills).toContainText("Anthropic, OpenAI");
  await pills
    .getByRole("button", { name: "Remove Services filter", exact: true })
    .click();
  await expect(pills).toHaveCount(0);
});

test("panels drag, reorder by keyboard, and persist their three-column sizes and intervals", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1100 });
  await page.goto("/admin/usage?mock=1&sample=operations");
  await expect(page.getByRole("article")).toHaveCount(6);
  const titles = () =>
    page
      .getByRole("article")
      .evaluateAll((elements) =>
        elements.map((element) => element.getAttribute("aria-label")),
      );
  const original = await titles();
  const from = await page
    .getByRole("button", { name: "Drag Request traffic", exact: true })
    .boundingBox();
  const to = await page
    .getByRole("button", { name: "Drag Service spend", exact: true })
    .boundingBox();
  expect(from).not.toBeNull();
  expect(to).not.toBeNull();
  await page.mouse.move(from!.x + from!.width / 2, from!.y + from!.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    from!.x + from!.width / 2 + 10,
    from!.y + from!.height / 2,
    { steps: 3 },
  );
  await page.mouse.move(to!.x + to!.width / 2, to!.y + to!.height / 2, {
    steps: 15,
  });
  await page.mouse.up();
  await expect
    .poll(titles)
    .toEqual([original[1], original[2], original[0], ...original.slice(3)]);
  const handle = page.getByRole("button", {
    name: "Drag Cost & traffic",
    exact: true,
  });
  await handle.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Space");
  await expect
    .poll(titles)
    .toEqual([original[2], original[1], original[0], ...original.slice(3)]);
  await page
    .getByRole("button", { name: "Configure Service spend", exact: true })
    .click();
  await select(page, "Panel width", "2 columns");
  await select(page, "Chart height", "Tall");
  await select(page, "Time interval", "Weekly");
  await page.keyboard.press("Escape");
  const panel = page.getByRole("article", {
    name: "Service spend",
    exact: true,
  });
  await expect(panel.getByText("Weekly · UTC", { exact: true })).toBeVisible();
  await expect(panel.locator(".analytics-plot")).toHaveCSS("height", "380px");
  const width = (await panel.boundingBox())!.width;
  const gridWidth = (await page
    .locator(".analytics-operations-grid")
    .boundingBox())!.width;
  expect(width / gridWidth).toBeGreaterThan(0.64);
  expect(width / gridWidth).toBeLessThan(0.68);
  await expect(
    page.getByRole("status", { name: "Workspace save status" }),
  ).toContainText("Saved in this browser");
  const arranged = await titles();
  await page.reload();
  await expect.poll(titles).toEqual(arranged);
  await expect(panel.locator(".analytics-plot")).toHaveCSS("height", "380px");
  await expect(panel.getByText("Weekly · UTC", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Configure Service spend", exact: true })
    .click();
  await select(page, "Panel width", "3 columns · full width");
  await select(page, "Time interval", "Monthly");
  await select(page, "Measure", "Input tokens");
  await page.keyboard.press("Escape");
  await expect(panel.getByText("Monthly · UTC", { exact: true })).toBeVisible();
  expect((await panel.boundingBox())!.width).toBeCloseTo(gridWidth, 0);
  await page.setViewportSize({ width: 1900, height: 1200 });
  expect(
    await page
      .locator(".analytics-operations-grid")
      .evaluate(
        (element) =>
          getComputedStyle(element).gridTemplateColumns.split(" ").length,
      ),
  ).toBe(3);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page
      .locator("main")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  expect(
    await page
      .locator(".analytics-operations-grid")
      .evaluate(
        (element) =>
          getComputedStyle(element).gridTemplateColumns.split(" ").length,
      ),
  ).toBe(1);
});

test("mixed-width panels keep stable drop targets and Escape cancels reordering", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1800 });
  await page.goto("/admin/usage?mock=1&sample=operations");
  await expect(page.getByRole("article")).toHaveCount(6);
  const titles = () =>
    page
      .getByRole("article")
      .evaluateAll((elements) =>
        elements.map((element) => element.getAttribute("aria-label")),
      );
  const original = await titles();
  for (const [title, width] of [
    ["Request traffic", "2 columns"],
    ["Service spend", "3 columns · full width"],
  ]) {
    await page
      .getByRole("button", { name: `Configure ${title}`, exact: true })
      .click();
    await select(page, "Panel width", width!);
    await page.keyboard.press("Escape");
  }
  // Mixed spans must keep the DOM order stable while a panel is held. Escape
  // cancels without committing a partial preview.
  const sourceHandle = page.getByRole("button", {
    name: "Drag Cost & traffic",
    exact: true,
  });
  await sourceHandle.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Escape");
  await expect.poll(titles).toEqual(original);
  const handle = page.getByRole("button", {
    name: "Drag Service spend",
    exact: true,
  });
  await handle.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Escape");
  await expect.poll(titles).toEqual(original);
  await expect(
    page.locator('.analytics-grid-item[data-drop-target="true"]'),
  ).toHaveCount(0);
  await expect(
    page.getByRole("status", { name: "Workspace save status" }),
  ).toContainText("Saved in this browser");
  await page.reload();
  await expect.poll(titles).toEqual(original);
});

test("chart tooltip values and axis labels have readable contrast in both themes", async ({
  page,
}) => {
  await page.goto("/admin/usage?mock=1&sample=operations");
  for (const theme of ["dark", "light"] as const) {
    const switcher = page.getByRole("button", {
      name: `Switch to ${theme} mode`,
      exact: true,
    });
    if (await switcher.count()) await switcher.click();
    const panel = page.getByRole("article", {
      name: "Top billing accounts",
      exact: true,
    });
    await panel.scrollIntoViewIfNeeded();
    const bar = panel.locator(".recharts-bar-rectangle").first();
    await expect(bar).toBeVisible();
    await bar.hover();
    const tooltip = panel.locator(".recharts-tooltip-wrapper");
    await expect(tooltip).toBeVisible();
    const ratios = await panel.evaluate((element) => {
      function luminance(color: string) {
        const rgb = color
          .match(/[\d.]+/g)!
          .slice(0, 3)
          .map(Number)
          .map((n) => n / 255)
          .map((n) =>
            n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4,
          );
        return rgb[0]! * 0.2126 + rgb[1]! * 0.7152 + rgb[2]! * 0.0722;
      }
      function contrast(a: string, b: string) {
        const pair = [luminance(a), luminance(b)].sort((a, b) => a - b);
        return (pair[1]! + 0.05) / (pair[0]! + 0.05);
      }
      const tooltip = element.querySelector(".recharts-default-tooltip")!;
      const value = element.querySelector(".recharts-tooltip-item-value")!;
      const tick = element.querySelector(
        ".recharts-cartesian-axis-tick-value",
      )!;
      return {
        tooltip: contrast(
          getComputedStyle(value).color,
          getComputedStyle(tooltip).backgroundColor,
        ),
        tick: contrast(
          getComputedStyle(tick).fill,
          getComputedStyle(element).backgroundColor,
        ),
      };
    });
    expect(ratios.tooltip).toBeGreaterThanOrEqual(4.5);
    expect(ratios.tick).toBeGreaterThanOrEqual(4.5);
    await expect(tooltip).not.toHaveText(/undefined|NaN/);
  }
});

for (const chart of ["bar", "line", "combo", "pie"] as const) {
  test(`${chart} keeps exact credit labels and tooltips with twenty colors and resizing`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1600, height: 1000 });
    let workspace: WorkspaceResponse = {
      revision: 0,
      config: {
        version: 1,
        saved_views: [],
        draft: {
          id: "00000000-0000-4000-8000-000000000101",
          name: "Exact accounting",
          layout: "operations",
          filters: {
            period: "24h",
            from: null,
            to: null,
            services: [],
            actors: [],
            owners: [],
          },
          panels: [
            {
              id: "00000000-0000-4000-8000-000000000102",
              title: "Exact credits",
              chart,
              measure: "cost",
              metric: "requests",
              breakdown: "service",
              top: 20,
              wide: false,
              span: 1,
              height: "compact",
            },
          ],
        },
      },
    };
    await page.route("**/api/v1/admin/usage/workspace", async (route) => {
      if (route.request().method() === "PUT") {
        workspace = {
          revision: workspace.revision + 1,
          config: route.request().postDataJSON().config,
        };
      }
      await route.fulfill({ json: workspace });
    });
    const points = Array.from({ length: 21 }, (_, index) => ({
      bucket: "2026-09-23T00:00:00Z",
      // Deliberately stale compatibility amounts must never override exact fields.
      value: 1,
      exact_value:
        index === 0
          ? "9007199254.740993"
          : index === 1
            ? null
            : "0.000000000001",
      requests: 1,
      unknown_cost_events: index === 1 ? 1 : 0,
    }));
    await page.route("**/api/v1/admin/usage/analytics?**", (route) =>
      route.fulfill({
        json: {
          window: {
            from: "2026-09-23T00:00:00Z",
            to: "2026-09-24T00:00:00Z",
            period: "24h",
          },
          freshness: {
            rolled_up_through: "2026-09-23T00:00:00Z",
            tail_rows: 0,
            validated: true,
          },
          granularity: "hour",
          unit: "microcredits",
          total: 21,
          exact_total: null,
          totals: {
            requests: 21,
            events: 21,
            quantities: {},
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            total_tokens: 0,
            gross_cost_micros: 21,
            wallet_cost_micros: 21,
            grant_cost_micros: 0,
            allowance_cost_micros: 0,
            gross_cost: null,
            wallet_cost: null,
            grant_cost: "0",
            allowance_cost: "0",
            exact_cost_events: 20,
            legacy_cost_events: 0,
            unknown_cost_events: 1,
            unique_users: 1,
            unique_services: 21,
          },
          points: [
            {
              ...points[0],
              value: 21,
              exact_value: null,
              requests: 21,
              unknown_cost_events: 1,
            },
          ],
          series: points.map((point, index) => ({
            label: `Service ${index}`,
            is_other: index === 20,
            points: [point],
          })),
          slices: points.map((point, index) => ({
            ...point,
            id: null,
            label: `Service ${index}`,
            is_other: index === 20,
          })),
        },
      }),
    );
    await page.goto("/admin/usage?mock=1");
    const panel = page.getByRole("article", {
      name: "Exact credits",
      exact: true,
    });
    await expect(
      panel.locator('[title="Service 0: 9,007,199,254.740993 credits"]'),
    ).toBeVisible();
    await expect(
      panel.locator('[title="Service 1: Unknown credits"]'),
    ).toBeVisible();
    await expect(
      panel.locator('[title="Service 2: <0.000001 credits"]'),
    ).toBeVisible();
    const colors = await panel
      .locator(".analytics-chart span.rounded-full")
      .evaluateAll((dots) =>
        dots.map((dot) => getComputedStyle(dot).backgroundColor),
      );
    expect(new Set(colors).size).toBe(21);
    const target = panel
      .locator(
        chart === "pie"
          ? ".recharts-pie-sector"
          : chart === "line"
            ? ".recharts-area-dot"
            : ".recharts-bar-rectangle",
      )
      .first();
    if (chart === "pie") {
      // Find a painted point: the donut center and the padding gaps are empty.
      const point = await target.evaluate((sector) => {
        const box = sector.getBoundingClientRect();
        for (const dx of [0.04, 0.2, 0.5, 0.8, 0.96]) {
          for (const dy of [0.04, 0.2, 0.5, 0.8, 0.96]) {
            const x = box.x + box.width * dx,
              y = box.y + box.height * dy;
            if (sector.contains(document.elementFromPoint(x, y)))
              return { x, y };
          }
        }
        throw new Error("The first donut slice has no painted hover target");
      });
      await page.mouse.move(point.x, point.y);
    } else {
      await target.hover();
    }
    await expect(panel.locator(".recharts-tooltip-wrapper")).toContainText(
      "9,007,199,254.740993 credits",
    );
    await page.mouse.move(0, 0);
    const resize = page.getByRole("button", { name: "Resize Exact credits" });
    await resize.focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowDown");
    await expect(panel.locator(".analytics-plot")).toHaveCSS("height", "280px");
    await expect(
      page.getByRole("status", { name: "Workspace save status" }),
    ).toHaveText("All changes saved");
    await page.reload();
    await expect(page.locator(".analytics-grid-item")).toHaveAttribute(
      "data-span",
      "2",
    );
    await expect(panel.locator(".analytics-plot")).toHaveCSS("height", "280px");
    await expect(
      panel.locator('[title="Service 0: 9,007,199,254.740993 credits"]'),
    ).toBeVisible();
    await expect(panel.getByRole("table")).toContainText(
      "9,007,199,254.740993",
    );
    await expect(panel.getByRole("table")).toContainText("Unknown");
    await expect(panel.getByRole("table")).toContainText("<0.000001");
  });
}
