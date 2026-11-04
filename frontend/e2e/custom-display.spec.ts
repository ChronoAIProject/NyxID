import { expect, test, type Locator, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

async function fixture(page: Page, path: string) {
  await page.context().grantPermissions(["local-network-access"]);
  await page.route("**/api/**", (route) => route.abort());
  await page.route(`**${path}?custom-display-fixture`, (route) =>
    route.fulfill({
      contentType: "text/html",
      body: `<html><head><meta name="viewport" content="width=device-width, initial-scale=1"><script type="module" src="/@vite/client"></script><script type="module">
    import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
    window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => type => type; window.__vite_plugin_react_preamble_installed__ = true;
    </script></head><body><div id="root"></div><script type="module" src="/e2e/fixtures/custom-display.tsx"></script></body></html>`,
    }),
  );
  await page.goto(`${path}?custom-display-fixture`);
  await expect(
    page.getByRole("button", { name: "View breakdown", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await page.addStyleTag({
    content:
      "*,*::before,*::after { animation:none!important; transition:none!important; }",
  });
}

async function role(
  target: Locator,
  token: number,
  weight: string,
  scale: number,
) {
  const style = await target.evaluate((e) => ({
    size: parseFloat(getComputedStyle(e).fontSize),
    weight: getComputedStyle(e).fontWeight,
  }));
  expect(style.size).toBeCloseTo(token * 0.85 * scale, 2);
  expect(style.weight).toBe(weight);
  return style;
}
async function targetBounds(
  target: Locator,
  container: Locator,
  width: number,
) {
  const box = (await target.boundingBox())!,
    outer = (await container.boundingBox())!;
  expect(box.width).toBeGreaterThanOrEqual(23.99);
  expect(box.height).toBeGreaterThanOrEqual(23.99);
  expect(box.x).toBeGreaterThanOrEqual(Math.max(0, outer.x) - 1);
  expect(box.x + box.width).toBeLessThanOrEqual(
    Math.min(width, outer.x + outer.width) + 1,
  );
  expect(await target.evaluate((e) => e.scrollWidth <= e.clientWidth + 1)).toBe(
    true,
  );
}

for (const theme of ["light", "dark"] as const)
  for (const width of [1440, 390])
    for (const scale of [0.5, 1, 1.5, 2]) {
      test(`custom roles, chart bounds and hit areas (${theme}, ${width}, ${scale}×)`, async ({
        browser,
      }) => {
        const context = await browser.newContext({
          viewport: { width, height: 1000 },
          hasTouch: width === 390,
          isMobile: width === 390,
          reducedMotion: "reduce",
        });
        const page = await context.newPage();
        const errors: string[] = [];
        page.on("pageerror", (error) => errors.push(error.message));
        await page.addInitScript(() => {
          const scope = window as Window & { displayErrors: string[] };
          scope.displayErrors = [];
          window.addEventListener("error", (event) => scope.displayErrors.push(event.message));
        });
        const readings: unknown[] = [];
        for (const path of ["/billing", "/assistant"]) {
          await fixture(page, path);
          await page.evaluate(
            ({ scale, theme }) =>
              window.customDisplayFixture.set(scale, theme, "billing"),
            { scale, theme },
          );
          await expect
            .poll(() =>
              page.evaluate(() =>
                parseFloat(getComputedStyle(document.documentElement).fontSize),
              ),
            )
            .toBeCloseTo(13.6 * scale, 2);
          const wallet = page.getByRole("region", { name: "Wallet fixture" });
          const breakdown = wallet.getByRole("button", {
            name: "View breakdown",
          });
          const action = await role(breakdown, 14, "500", scale);
          for (const target of [
            breakdown,
            wallet.getByRole("button", { name: "About wallet" }),
          ])
            await targetBounds(target, wallet, width);
          const analytics = page.getByRole("region", {
            name: "Analytics fixture",
          });
          // Panels intentionally defer rendering until their viewport sentinel
          // enters. Enlarged wallet/filter content can put this panel offscreen.
          await analytics.getByRole("article", { name: "Request traffic" }).scrollIntoViewIfNeeded();
          const metrics = analytics.locator(".font-display.text-28");
          await expect(metrics).toHaveCount(5);
          for (const metric of await metrics.all())
            await role(metric, 28, "600", scale);
          await targetBounds(
            analytics.getByRole("button", { name: /Token count breakdown/ }),
            analytics,
            width,
          );
          await targetBounds(
            analytics.getByRole("button", { name: "Add panel", exact: true }),
            analytics,
            width,
          );
          const analyticsTicks = analytics.locator(".recharts-xAxis-tick-labels .recharts-cartesian-axis-tick-value");
          await expect(analyticsTicks.first()).toBeVisible();
          const analyticsBox = (await analytics.boundingBox())!;
          let previousRight = analyticsBox.x;
          for (const tick of await analyticsTicks.all()) {
            await role(tick, 13, "400", scale);
            const box = (await tick.boundingBox())!;
            expect(box.x).toBeGreaterThanOrEqual(previousRight - 1);
            expect(box.x + box.width).toBeLessThanOrEqual(analyticsBox.x + analyticsBox.width + 1);
            previousRight = box.x + box.width;
          }
          const services = page.getByRole("region", {
            name: "Service fixture",
          });
          await targetBounds(
            services.getByRole("button", { name: "Aevatar", exact: true }),
            services,
            width,
          );
          const chart = page.getByRole("region", { name: "Quantity fixture" });
          for (const control of await chart.locator("button").all())
            await targetBounds(control, chart, width);
          expect(
            await chart.evaluate((e) => e.scrollWidth <= e.clientWidth + 1),
          ).toBe(true);
          const ticks = chart.locator(
            ".recharts-xAxis-tick-labels .recharts-cartesian-axis-tick-value",
          );
          await expect(ticks.first()).toBeVisible();
          await expect
            .poll(async () =>
              ticks.evaluateAll((elements) => {
                const boxes = elements
                  .map((e) => e.getBoundingClientRect())
                  .sort((a, b) => a.x - b.x);
                return boxes.every(
                  (box, i) => i === 0 || box.left >= boxes[i - 1]!.right + 1,
                );
              }),
            )
            .toBe(true);
          const chartBox = (await chart.boundingBox())!;
          for (const tick of await ticks.all()) {
            await role(tick, 13, "400", scale);
            const box = (await tick.boundingBox())!;
            expect(box.x).toBeGreaterThanOrEqual(chartBox.x - 1);
            expect(box.x + box.width).toBeLessThanOrEqual(
              chartBox.x + chartBox.width + 1,
            );
          }
          // Exercise chip rows with long labels and Clear filters after a selection.
          await chart
            .getByRole("button", { name: "Filter metrics", exact: true })
            .click();
          await page.getByRole("checkbox", { name: /input tokens/i }).click();
          await page.getByRole("button", { name: "Done", exact: true }).click();
          await chart
            .getByRole("button", { name: "Choose metrics", exact: true })
            .click();
          await page.getByRole("checkbox", { name: /input tokens/i }).click();
          await page.getByRole("button", { name: "Done", exact: true }).click();
          await targetBounds(
            chart.getByRole("button", { name: "Clear filters", exact: true }),
            chart,
            width,
          );
          if (
            process.env.NYX_REVIEW_SHOTS &&
            path === "/billing" &&
            (scale === 0.5 || scale === 2)
          ) {
            await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
            for (const [name, element] of [
              ["wallet", wallet],
              ["quantity", chart],
              ["analytics", analytics],
              ["services", services],
            ] as const) {
              await element.screenshot({
                path: join(
                  process.env.NYX_REVIEW_SHOTS,
                  `${theme}-${width}-${scale}-${name}.png`,
                ),
              });
            }
          }
          await page.evaluate(
            ({ scale, theme }) =>
              window.customDisplayFixture.set(scale, theme, "signin"),
            { scale, theme },
          );
          const providers = page.locator(
            ".nb-provider,.nb-device-login > button,.nb-auth-footer button",
          );
          await expect(providers).toHaveCount(6);
          for (const provider of await providers.all()) {
            await role(provider, 14, "500", scale);
            await targetBounds(
              provider,
              page.locator(".nyxbot-onboarding"),
              width,
            );
          }
          if (
            process.env.NYX_REVIEW_SHOTS &&
            path === "/billing" &&
            scale === 2
          )
            await page.screenshot({
              path: join(
                process.env.NYX_REVIEW_SHOTS,
                `${theme}-${width}-${scale}-signin.png`,
              ),
              fullPage: true,
            });
          expect(await page.evaluate(() => (window as Window & { displayErrors: string[] }).displayErrors)).toEqual([]);
          readings.push({ path, action, providers: await providers.count() });
        }
        await test.info().attach("custom-role-measurements.json", {
          body: JSON.stringify({ theme, width, scale, readings }),
          contentType: "application/json",
        });
        expect(errors).toEqual([]);
        await context.close();
      });
    }
