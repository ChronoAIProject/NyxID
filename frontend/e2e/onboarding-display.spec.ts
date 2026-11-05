import { expect, test, type Locator } from "@playwright/test";

async function staysInside(control: Locator, parent: Locator) {
  const box = await control.boundingBox(),
    outer = await parent.boundingBox();
  expect(box).not.toBeNull();
  expect(outer).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(outer!.x - 1);
  expect(box!.x + box!.width).toBeLessThanOrEqual(outer!.x + outer!.width + 1);
}

for (const theme of ["light", "dark"] as const)
  for (const width of [1440, 390])
    for (const scale of [0.5, 1, 1.5, 2]) {
      test(`onboarding roles, targets and wrapping (${theme}, ${width}, ${scale}×)`, async ({
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
        await page.context().grantPermissions(["local-network-access"]);
        await page.route("**/api/**", (route) => route.abort());
        await page.route("**/assistant?onboarding-display-fixture", (route) =>
          route.fulfill({
            contentType: "text/html",
            body: `<html><head><meta name="viewport" content="width=device-width, initial-scale=1"><script type="module" src="/@vite/client"></script><script type="module">
      import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
      window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => type => type; window.__vite_plugin_react_preamble_installed__ = true;
      </script></head><body><div id="root"></div><script type="module" src="/e2e/fixtures/onboarding-display.tsx"></script></body></html>`,
          }),
        );
        await page.goto("/assistant?onboarding-display-fixture");
        await expect(
          page.getByRole("heading", { name: "Welcome, Owner" }),
        ).toBeVisible();
        await page.evaluate(
          ({ scale, theme }) => window.onboardingFixture.set(scale, theme),
          { scale, theme },
        );
        await expect
          .poll(() =>
            page.evaluate(() =>
              parseFloat(getComputedStyle(document.documentElement).fontSize),
            ),
          )
          .toBeCloseTo(13.6 * scale, 2);
        const intro = page.getByRole("dialog");
        await page.getByRole("button", { name: "Let's get started" }).click();
        for (const target of [
          intro.getByRole("button", { name: "Close", exact: true }),
          ...(await intro.getByRole("button", { name: /Step \d of/ }).all()),
        ]) {
          const box = await target.boundingBox();
          expect(box!.width).toBeGreaterThanOrEqual(23.99);
          expect(box!.height).toBeGreaterThanOrEqual(23.99);
        }
        for (const target of await intro.getByRole("radio").all()) {
          await staysInside(target, intro);
          const style = await target.evaluate((e) => ({
            size: parseFloat(getComputedStyle(e).fontSize),
            weight: getComputedStyle(e).fontWeight,
          }));
          expect(style.size).toBeCloseTo(14 * 0.85 * scale, 2);
          expect(style.weight).toBe("500");
        }
        expect(
          await intro.evaluate((e) => e.scrollWidth <= e.clientWidth),
        ).toBe(true);
        await intro.getByRole("radio", { name: "My team" }).click();
        await intro.getByRole("checkbox", { name: "Email & inbox" }).click();
        await intro
          .getByRole("button", { name: "Continue", exact: true })
          .click();
        await expect(intro.getByRole("figure")).toBeVisible();
        const replay = intro.getByRole("figure");
        const replayHeight = await replay
          .locator('[class*="h-[18.75rem]"]')
          .evaluate((e) => e.getBoundingClientRect().height);
        expect(replayHeight).toBeCloseTo(300 * 0.85 * scale, 1);
        await intro
          .getByRole("button", { name: "Step 4 of 5: You stay in control" })
          .click();
        for (const target of await intro
          .getByRole("radiogroup", { name: "Access modes" })
          .getByRole("radio")
          .all())
          await staysInside(target, intro);
        await intro
          .getByRole("button", { name: "Step 5 of 5: First steps" })
          .click();
        const suggestion = intro.getByRole("button", {
          name: /Connect your inbox Let NyxBot/,
        });
        await staysInside(suggestion, intro);
        expect(
          await suggestion.evaluate((e) => e.scrollWidth <= e.clientWidth),
        ).toBe(true);
        expect(
          await intro.locator("[data-grouped-list]").evaluateAll(
            (groups) =>
              groups.flatMap((group) =>
                [...group.querySelectorAll("*")].filter((element) => {
                  const style = getComputedStyle(element);
                  return [
                    style.borderTopWidth,
                    style.borderRightWidth,
                    style.borderBottomWidth,
                    style.borderLeftWidth,
                  ].some((value) => parseFloat(value) > 0);
                }),
              ).length,
          ),
        ).toBe(0);
        await intro.getByRole("button", { name: "Show me around" }).click();
        const tour = page.getByRole("dialog", { name: "Ask NyxBot anything" });
        await expect(tour).toBeVisible();
        await expect
          .poll(async () => (await tour.boundingBox())!.x)
          .toBeGreaterThanOrEqual(0);
        for (const action of await tour.getByRole("button").all())
          await staysInside(action, tour);
        expect(await tour.evaluate((e) => e.scrollWidth <= e.clientWidth)).toBe(
          true,
        );
        await page.keyboard.press("Escape");
        await expect(tour).toHaveCount(0);
        expect(errors).toEqual([]);
        await context.close();
      });
    }
