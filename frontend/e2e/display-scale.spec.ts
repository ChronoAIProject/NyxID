import { expect, test, type Page } from "@playwright/test";

const tokens = [9, 10, 11, 12, 13, 14, 15, 16, 17, 20, 22, 24, 28, 36];
const scales = [0.5, 0.85, 1, 1.5, 2];

async function fixture(page: Page, path: string) {
  // Real shared components, real Display controls/store/hook, and real portals.
  // Isolate data and navigation so this check also runs without backend/auth.
  await page.context().grantPermissions(["local-network-access"]);
  await page.route("**/api/**", route => route.abort());
  await page.route(`**${path}?display-fixture`, route => route.fulfill({
    contentType: "text/html",
    body: `<html><head><meta name="viewport" content="width=device-width, initial-scale=1"><script type="module" src="/@vite/client"></script><script type="module">
      import RefreshRuntime from '/@react-refresh';
      RefreshRuntime.injectIntoGlobalHook(window);
      window.$RefreshReg$ = () => {};
      window.$RefreshSig$ = () => type => type;
      window.__vite_plugin_react_preamble_installed__ = true;
      </script></head><body><div id="root"></div><script type="module" src="/e2e/fixtures/display-roles.tsx"></script></body></html>`,
  }));
  await page.goto(`${path}?display-fixture`);
  await expect(page.getByRole("textbox", { name: "Email", exact: true })).toBeVisible();
  await page.addStyleTag({ content: "*,*::before,*::after { animation:none!important; transition:none!important; }" });
}

async function metrics(page: Page, selector: string) {
  return page.locator(selector).evaluateAll(elements => elements.map(element => {
    const style = getComputedStyle(element), box = element.getBoundingClientRect();
    return { size: parseFloat(style.fontSize), weight: style.fontWeight, width: box.width, height: box.height };
  }));
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) for (const scale of scales) {
  test(`real roles, portals, scaling and touch targets (${theme}, ${width}, ${scale}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, hasTouch: width === 390, isMobile: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    const readings: unknown[] = [];
    for (const path of ["/settings", "/assistant/automations"]) {
      await fixture(page, path);
      expect(await page.evaluate(() => innerWidth)).toBe(width);
      {
        await page.evaluate(({ scale, theme }) => window.displayFixture.set(scale, theme), { scale, theme });
        await expect.poll(() => page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize))).toBeCloseTo(13.6 * scale, 3);
        const roles = {
          title: (await metrics(page, "main h2"))[0],
          cardTitle: (await metrics(page, '[data-role="card"] > :first-child'))[0],
          cardSubtitle: (await metrics(page, '[data-role="card"] > :last-child'))[0],
          body: (await metrics(page, '[data-role="body"]'))[0],
          meta: (await metrics(page, '[data-role="meta"]'))[0],
          label: (await metrics(page, 'form[aria-label="Profile form"] label'))[0],
          input: (await metrics(page, 'input[name="email"]'))[0],
          button: (await metrics(page, 'form[aria-label="Profile form"] button[type="submit"]'))[0],
        };
        for (const [role, n, weight] of [["title", width === 390 ? 22 : 28, "700"], ["cardTitle", 15, "600"], ["cardSubtitle", 13, "400"], ["body", 14, "400"], ["meta", 13, "400"]] as const) {
          expect(roles[role]?.size, role).toBeCloseTo(n * 0.85 * scale, 2);
          expect(roles[role]?.weight, role).toBe(weight);
        }
        expect(roles.label?.size).toBeCloseTo(14 * 0.85 * scale, 2);
        expect(roles.label?.weight).toBe("500");
        expect(roles.button?.size).toBeCloseTo(14 * 0.85 * scale, 2);
        expect(roles.button?.weight).toBe("500");
        expect(roles.input?.size).toBeCloseTo(width === 390 ? Math.max(16, 14 * 0.85 * scale) : 14 * 0.85 * scale, 2);
        expect(roles.input?.weight).toBe("400");
        expect(roles.button!.height).toBeGreaterThanOrEqual(23.99);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        const portals: unknown[] = [];
        for (const [kind, selector, n, weight] of [
          ["dialog", '[data-fixture-portal="dialog"] h2', 15, "600"],
          ["sheet", '[data-fixture-portal="sheet"] h2', 15, "600"],
          ["menu", '[data-fixture-portal="menu"] [role="menuitem"]', 14, "400"],
          ["select", '[data-fixture-portal="select"] [role="option"]', 14, "400"],
          ["tooltip", '[data-fixture-portal="tooltip"]', 13, "400"],
          ["toast", '[data-sonner-toast] [data-title]', 15, "500"],
        ] as const) {
          await page.evaluate(kind => window.displayFixture.portal(kind), kind);
          await expect(page.locator(selector).first()).toBeVisible();
          const reading = (await metrics(page, selector))[0]!;
          expect(reading.size, `${path} ${kind}`).toBeCloseTo(n * 0.85 * scale, 2);
          expect(reading.weight, `${path} ${kind}`).toBe(weight);
          portals.push({ kind, size: reading.size, weight: reading.weight });
          if (kind === "dialog" || kind === "sheet") {
            const close = page.locator(`[data-fixture-portal="${kind}"] button`).filter({ has: page.locator('span.sr-only') });
            const box = await close.boundingBox();
            expect(box?.width).toBeGreaterThanOrEqual(23.99);
            expect(box?.height).toBeGreaterThanOrEqual(23.99);
          }
          if (kind === "toast") for (const action of ["Action", "Cancel"]) {
            const button = page.locator('[data-sonner-toast]').getByRole("button", { name: action, exact: true });
            const style = await button.evaluate(e => ({ size: parseFloat(getComputedStyle(e).fontSize), weight: getComputedStyle(e).fontWeight }));
            expect(style.size).toBeCloseTo(14 * 0.85 * scale, 2);
            expect(style.weight).toBe("500");
          }
        }
        await page.evaluate(() => window.displayFixture.portal(""));
        readings.push({ scale, roles: Object.fromEntries(Object.entries(roles).map(([name, value]) => [name, { size: value!.size, weight: value!.weight }])), portals });
      }
    }
    expect(readings.slice(0, 1)).toEqual(readings.slice(1));
    await test.info().attach("role-measurements.json", { body: JSON.stringify({ theme, width, scale, readings }), contentType: "application/json" });
    expect(errors).toEqual([]);
    await context.close();
  });
}

test("compiled CSS defines each numeric token once as N/16rem, including unthemed defaults", async ({ page }) => {
  await fixture(page, "/settings");
  const definitions = await page.evaluate(() => {
    const result: Record<string, string[]> = {};
    function inspect(rules: CSSRuleList) {
      for (const rule of rules) {
        if ("style" in rule) for (const property of (rule as CSSStyleRule).style) if (/^--text-\d+$/.test(property)) (result[property] ??= []).push((rule as CSSStyleRule).style.getPropertyValue(property).trim());
        if ("cssRules" in rule) inspect((rule as CSSGroupingRule).cssRules);
      }
    }
    for (const sheet of document.styleSheets) inspect(sheet.cssRules);
    return result;
  });
  for (const n of tokens) expect(definitions[`--text-${n}`]).toEqual([`${n / 16}rem`]);
});

test("actual Display slider persists without migrating a second time", async ({ page }) => {
  await fixture(page, "/settings");
  const slider = page.getByRole("slider", { name: "Text size", exact: true });
  await slider.focus();
  await slider.press("Home");
  await slider.press("ArrowRight");
  await expect(slider).toHaveValue("0.51");
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("nyxid.theme")!).state.textScale)).toBe(0.51);
  await page.reload();
  await expect(page.getByRole("slider", { name: "Text size", exact: true })).toHaveValue("0.51");
  await expect.poll(() => page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize))).toBeCloseTo(6.936, 2);
});
