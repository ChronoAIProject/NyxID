import { test, expect } from "@playwright/test";
test("initial body remains readable with JavaScript disabled", async ({
  browser,
}) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  for (const path of [
    "/",
    "/zh",
    "/security",
    "/zh/solutions/help-desk",
    "/docs/ai/guides/agent-isolation",
  ]) {
    await page.goto(`http://127.0.0.1:4188${path}`);
    await expect(page.locator("h1")).toBeVisible();
    await expect(page.locator("main")).not.toBeEmpty();
  }
  await context.close();
});
test("hydrates, keeps shell during a delayed route chunk and supports history and locale URLs", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.goto("/?code=INV&return_to=%2Fkeys&utm_source=test");
  await expect(page.locator("nav")).toBeVisible();
  await page.waitForFunction(() =>
    document.querySelector('nav a[href^="/security"]'),
  );
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await expect(page.locator('nav a[href^="/register"]')).toHaveAttribute(
    "href",
    /code=INV/,
  );
  const nav = await page.locator("#nav").elementHandle();
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let delayed = false;
  await page.route(/\/assets\/SecurityPage-[^/]+\.js/, async (route) => {
    delayed = true;
    await held;
    await route.continue();
  });
  await page.locator('nav a[href^="/security"]').click({ noWaitAfter: true });
  await page.waitForTimeout(250);
  await expect(page.locator("#nav")).toBeVisible();
  await expect(page.locator("#foot")).toBeAttached();
  expect(await nav!.evaluate((el) => el.isConnected)).toBe(true);
  release();
  await expect(page).toHaveURL(/\/security\?/);
  await expect(page.locator("h1")).toContainText("passwords");
  expect(delayed).toBe(true);
  await page.getByRole("link", { name: "中文", exact: true }).click();
  await expect(page).toHaveURL(/\/zh\/security\?/);
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  expect(new URL(page.url()).searchParams.get("return_to")).toBe("/keys");
  await page.goBack();
  await expect(page).toHaveURL(/\/security\?/);
  await page.goForward();
  await expect(page).toHaveURL(/\/zh\/security\?/);
  expect(errors.filter((e) => !/401|Failed to load resource/.test(e))).toEqual(
    [],
  );
});
test("contact preselection and disabled solution gate survive hydration", async ({
  page,
}) => {
  await page.goto("/zh/contact?topic=bundle&return_to=%2Fkeys");
  await expect(page.locator("#cTopic")).toHaveValue("bundle");
  await expect(
    page.locator("#cForm input,#cForm select,#cForm textarea"),
  ).toHaveCount(5);
  await expect(page.locator('[role="log"]')).toHaveCount(0);
  await page.goto("/solutions/help-desk");
  await expect(
    page.getByRole("button", { name: "Use this solution", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByText("Not available yet", { exact: true }),
  ).toBeVisible();
});

test("records a bounded lab resource sample", async ({ page }) => {
  await page.addInitScript(() => {
    const stats = { lcp: null as number | null, cls: 0 };
    (window as unknown as { publicLab: typeof stats }).publicLab = stats;
    if (
      PerformanceObserver.supportedEntryTypes.includes(
        "largest-contentful-paint",
      )
    ) {
      new PerformanceObserver((list) => {
        stats.lcp = list.getEntries().at(-1)?.startTime ?? null;
      }).observe({ type: "largest-contentful-paint", buffered: true });
    }
    if (PerformanceObserver.supportedEntryTypes.includes("layout-shift")) {
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          const shift = entry as PerformanceEntry & {
            hadRecentInput: boolean;
            value: number;
          };
          if (!shift.hadRecentInput) stats.cls += shift.value;
        }
      }).observe({ type: "layout-shift", buffered: true });
    }
  });
  await page.goto("/security");
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  const sample = await page.evaluate(() => {
    const resources = performance.getEntriesByType(
      "resource",
    ) as PerformanceResourceTiming[];
    return {
      kind: "lab",
      path: "/security",
      reducedMotion: true,
      fieldCwvStatus: "not-measured",
      resources: resources.length,
      transferBytes: resources.reduce((sum, r) => sum + r.transferSize, 0),
      decodedBytes: resources.reduce((sum, r) => sum + r.decodedBodySize, 0),
      ...(
        window as unknown as { publicLab: { lcp: number | null; cls: number } }
      ).publicLab,
      limitations:
        "Single CI browser at takeover; cross-origin transfer sizes may be unavailable. Not field CWV or an LCP/CLS pass assertion.",
    };
  });
  const { writeFile } = await import("node:fs/promises");
  await writeFile("dist/public-lab.json", JSON.stringify(sample, null, 2));
});

test("retains authenticated home redirects and Docs controls", async ({
  page,
  context,
}) => {
  await context.addCookies([
    {
      name: "fixture-auth",
      value: "1",
      url: "http://127.0.0.1:4188",
      httpOnly: true,
    },
  ]);
  for (const path of ["/", "/zh"]) {
    await page.goto(path);
    await expect(page).toHaveURL(/\/dashboard$/);
  }
  await page.goto("/docs/ai/guides/agent-isolation");
  await expect(
    page.getByRole("link", { name: "Dashboard", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Switch to (light|dark) mode/ }),
  ).toBeVisible();
  await expect(page.locator('aside a[href^="#"]')).not.toHaveCount(0);
});
