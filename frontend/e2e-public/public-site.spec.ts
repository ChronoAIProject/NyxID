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
  await expect(page.locator('nav a[href^="/register"]')).toHaveAttribute("href", /code=INV/);
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
