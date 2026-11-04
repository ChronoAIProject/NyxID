import { expect, test, type Page } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expectReadableToast } from "./contrast";
import type { QueryClient } from "@tanstack/react-query";

const properties = ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "columnGap", "rowGap", "minHeight", "minWidth", "lineHeight", "borderTopLeftRadius", "borderTopRightRadius", "borderBottomLeftRadius", "borderBottomRightRadius", "fontSize", "fontWeight"] as const;
const fontResponses = new Map<string, Promise<{ body: Buffer; contentType: string }>>();

async function readings(page: Page, selector: string) {
  return page.locator(selector).evaluateAll((elements, properties) => Object.fromEntries(elements.map((element, i) => {
    const style = getComputedStyle(element);
    const icons = [...element.querySelectorAll("svg")].map(icon => { const s = getComputedStyle(icon); return { width: s.width, height: s.height }; });
    return [element.getAttribute("data-parity") ?? `${element.tagName}-${i}`, { ...Object.fromEntries(properties.map(key => [key, style[key as keyof CSSStyleDeclaration]])), height: style.height, icons }];
  })), [...properties]);
}

async function settleLayout(page: Page) {
  await page.evaluate(async () => {
    await Promise.all([400, 500, 600, 700].map(weight => document.fonts.load(`${weight} 16px "Mona Sans"`)));
    await document.fonts.ready;
    // Flush the font/layout commit and reduced-motion transition frame before
    // comparing geometry, without depending on wall-clock animation delays.
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  });
}

async function openRoute(page: Page, path: string, scale: number, density: number, theme: "light" | "dark", mount = true) {
  // Each pair must use identical font bytes. Page routing disables the HTTP
  // cache; refetching optional Google fonts for every navigation can compare
  // a loaded webfont with a fallback and change flex-row wrapping under load.
  await page.route(/^https:\/\/fonts\.(googleapis|gstatic)\.com\//, async route => {
    const url = route.request().url();
    if (!fontResponses.has(url)) fontResponses.set(url, (async () => {
      const response = await route.fetch();
      expect(response.status(), url).toBe(200);
      return { body: await response.body(), contentType: response.headers()["content-type"]! };
    })());
    const { body, contentType } = await fontResponses.get(url)!;
    await route.fulfill({ body, contentType, headers: { "access-control-allow-origin": "*" } });
  });
  await page.addInitScript(({ scale, density, theme }) => {
    localStorage.setItem("nyxid.theme", JSON.stringify({ state: { textScale: scale, density, mode: theme, motion: "reduce" }, version: 2 }));
    localStorage.setItem("nyxid.telemetry_consent.v3.test-user", JSON.stringify({ enabled: false, asked: true }));
  }, { scale, density, theme });
  await page.goto(`${path}?mock=1`);
  await expect(page.locator(".shell-page-content")).toBeVisible();
  if (!mount) {
    await settleLayout(page);
    return;
  }
  await page.evaluate(async () => {
    const module = await import("/e2e/fixtures/mount-treatment.ts");
    module.mountTreatment();
  });
  await expect(page.locator('[data-parity="input"]')).toBeVisible();
  await expect(page.locator('[data-parity="hidden-icon"]')).toBeHidden();
  await settleLayout(page);
}

async function navSiblingGaps(page: Page, density: number) {
  const stacks = page.locator(".shell-nav-stack:visible");
  await expect(stacks.first()).toBeVisible();
  const gaps = await stacks.evaluateAll(stacks => stacks.flatMap(stack => {
    const rows = [...stack.children].filter(child => child.matches(".shell-nav-item, .ui-icon-button"));
    return rows.slice(1).filter((row, i) => rows[i]!.nextElementSibling === row).map(row => ({
      previous: row.previousElementSibling!.textContent,
      next: row.textContent || row.getAttribute("aria-label"),
      gap: row.getBoundingClientRect().top - row.previousElementSibling!.getBoundingClientRect().bottom,
    }));
  }));
  expect(gaps.length).toBeGreaterThan(0);
  for (const pair of gaps) expect(pair.gap, JSON.stringify(pair)).toBeCloseTo(2 * density, 1);
  return gaps;
}

async function pluginCardBounds(page: Page) {
  const grids = await page.locator(".assistant-plugin-grid:visible").evaluateAll(grids => grids.map(grid => {
    const cards = [...grid.querySelectorAll<HTMLElement>("[data-plugin-card]")];
    return cards.map(card => {
      const outer = card.getBoundingClientRect();
      const children = [...card.children, ...card.querySelectorAll("button,a")].map(child => {
        const box = child.getBoundingClientRect();
        return { text: child.textContent?.trim(), left: box.left - outer.left, right: outer.right - box.right, top: box.top - outer.top, bottom: outer.bottom - box.bottom };
      });
      const overlaps = cards.filter(other => other !== card).filter(other => {
        const b = other.getBoundingClientRect();
        return outer.left < b.right && outer.right > b.left && outer.top < b.bottom && outer.bottom > b.top;
      }).length;
      return { name: card.querySelector("[data-service-card-title]")?.textContent, height: outer.height, children, overlaps };
    });
  }));
  expect(grids.flat().length).toBeGreaterThan(1);
  for (const cards of grids) for (const card of cards) {
    expect(card.overlaps, String(card.name)).toBe(0);
    expect(card.height, String(card.name)).toBeCloseTo(cards[0]!.height, 1);
    for (const child of card.children) for (const edge of ["left", "right", "top", "bottom"] as const) {
      expect(child[edge], JSON.stringify({ card: card.name, child, edge })).toBeGreaterThanOrEqual(-1);
    }
  }
  return grids;
}

async function fieldRole(page: Page, selector: string, text: string) {
  const field = page.locator(`${selector}:visible`).filter({ hasText: text }).first();
  await expect(field).toBeVisible();
  return field.evaluate(element => {
    const style = getComputedStyle(element);
    return { size: style.fontSize, weight: style.fontWeight, lineHeight: style.lineHeight, tracking: style.letterSpacing };
  });
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) for (const scale of [0.5, 1, 1.5, 2]) for (const density of [1, 1.4]) {
  test(`populated approval pending and History field parity (${theme}, ${width}, text ${scale}×, spacing ${density}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/api/**", route => route.fulfill({ json: {} }));
    const evidence = [];
    for (const path of ["/approvals/history", "/assistant/approvals"]) {
      const assistant = path.startsWith("/assistant");
      await openRoute(page, path, scale, density, theme, false);
      const pending = {
        identity: await fieldRole(page, assistant ? '.assistant-workspace-approval > h3' : '[data-approval-field="identity"]', "Lark"),
        action: await fieldRole(page, assistant ? '.assistant-workspace-approval > section > div:nth-child(2) > p:first-child' : '[data-approval-field="action"]', "Post the drafted summary"),
      };
      if (assistant) await page.getByRole("tab", { name: "History", exact: true }).click();
      const history = {
        identity: await fieldRole(page, '[data-approval-field="identity"]', "OpenAI"),
        action: await fieldRole(page, '[data-approval-field="action"]', "Generate chat completion with gpt-4o"),
      };
      for (const fields of [pending, history]) {
        expect(parseFloat(fields.identity.size)).toBeCloseTo(14 * 0.85 * scale, 2);
        expect(fields.identity.weight).toBe("500");
        expect(parseFloat(fields.action.size)).toBeCloseTo(14 * 0.85 * scale, 2);
        expect(fields.action.weight).toBe("400");
      }
      evidence.push({ pending, history });
      if (process.env.NYX_REVIEW_SHOTS && density === 1.4 && [1, 2].includes(scale)) {
        await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
        await page.locator('[data-approval-field="identity"]:visible').filter({ hasText: "OpenAI" }).scrollIntoViewIfNeeded();
        await page.screenshot({ animations: "disabled", path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${assistant ? "assistant" : "studio"}-history.png`) });
      }
    }
    expect(evidence[1]).toEqual(evidence[0]);
    expect(errors).toEqual([]);
    const metrics = JSON.stringify({ theme, width, scale, density, evidence }, null, 2);
    await test.info().attach("populated-history.json", { body: metrics, contentType: "application/json" });
    if (process.env.NYX_REVIEW_SHOTS) await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${density}-history.json`), metrics);
    await context.close();
  });

  test(`equivalent automation labels and machine summaries (${theme}, ${width}, text ${scale}×, spacing ${density}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/api/**", route => route.fulfill({ json: {} }));
    await page.route("**/api/v1/machines/updates", route => route.fulfill({ json: [] }));
    const evidence = [];
    for (const [studio, assistant, selector, label, token, weight] of [
      ["/triggers", "/assistant/automations", "[data-trigger-title]", "Shared trigger", 14, "500"],
      ["/nodes", "/assistant/machines", "[data-machine-card-title]", "Shared machine", 15, "600"],
    ] as const) {
      const pair = [];
      for (const path of [studio, assistant]) {
        await openRoute(page, path, scale, density, theme, false);
        await page.evaluate(async () => {
          const client = (window as Window & { __nyxQueryClient: QueryClient }).__nyxQueryClient;
          await client.cancelQueries({ queryKey: ["nodes"] });
          await client.cancelQueries({ queryKey: ["triggers"] });
          const { getMockResponse } = await import("/src/lib/mock-data.ts");
          const { nodes } = getMockResponse("/nodes") as import("../src/types/nodes").NodeListResponse;
          if (nodes?.length) client.setQueryData(["nodes"], [{ ...nodes[0], name: "Shared machine", machine: {
            version: 1, shell: true, files: true, computer: false, browser: false, os: "linux", arch: "x86_64", roots: ["/workspace"], computer_mode: "standard", cua_version: null, computer_tools: [], computer_ready: false, browser_isolated: false, saved_login_ready: false,
          } }]);
          client.setQueryData(["triggers", { orgId: null }], { triggers: [{
            id: "11111111-1111-4111-8111-111111111111", user_id: "test-user", label: "Shared trigger", source: "webhook", status: "active", user_service_id: null, verification: { mode: "token", location: "bearer" }, delivery: { type: "notification" }, inbound_url: "https://example.test/trigger", created_at: "2026-11-05T00:00:00Z", updated_at: "2026-11-05T00:00:00Z",
          }] });
        });
        const role = await fieldRole(page, selector, label);
        expect(parseFloat(role.size)).toBeCloseTo(token * 0.85 * scale, 2);
        expect(role.weight).toBe(weight);
        pair.push(role);
      }
      expect(pair[1]).toEqual(pair[0]);
      evidence.push({ studio, assistant, pair });
    }
    expect(errors).toEqual([]);
    await test.info().attach("equivalent-list-roles.json", { body: JSON.stringify({ theme, width, scale, density, evidence }, null, 2), contentType: "application/json" });
    await context.close();
  });
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) {
  test(`normal and typed toast text meets contrast in each shell (${theme}, ${width})`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, reducedMotion: "reduce" });
    const page = await context.newPage();
    await page.route("**/api/**", route => route.fulfill({ json: {} }));
    const evidence = [];
    for (const path of ["/approvals/history", "/assistant/approvals"]) {
      await openRoute(page, path, 2, 1.4, theme);
      for (const kind of ["toast", "toast:success", "toast:error", "toast:info", "toast:warning"]) {
        await page.evaluate(kind => window.treatmentFixture.portal(kind), kind);
        evidence.push({ path, kind, colors: await expectReadableToast(page) });
        if (process.env.NYX_REVIEW_SHOTS && kind === "toast") {
          await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
          await page.screenshot({ animations: "disabled", path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${path.startsWith("/assistant") ? "assistant" : "studio"}-toast.png`) });
        }
        await page.evaluate(() => window.treatmentFixture.portal(""));
        await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
      }
    }
    await test.info().attach("toast-contrast.json", { body: JSON.stringify(evidence, null, 2), contentType: "application/json" });
    if (process.env.NYX_REVIEW_SHOTS) await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-toast-contrast.json`), JSON.stringify(evidence, null, 2));
    await context.close();
  });
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) for (const scale of [0.5, 1, 1.5, 2]) for (const density of [1, 1.4]) {
  test(`real service headers, card bounds and nav stacks (${theme}, ${width}, text ${scale}×, spacing ${density}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/api/**", route => route.fulfill({ json: {} }));
    const headers = [];
    const evidence = [];
    for (const path of ["/keys", "/assistant/plugins"]) {
      const assistant = path.startsWith("/assistant");
      await openRoute(page, path, scale, density, theme, false);
      const service = page.locator("[data-service-card-header]").filter({ has: page.getByRole("heading", { name: "GitHub", exact: true }) });
      await expect(service).toBeVisible();
      const header = await service.evaluate(element => {
        const title = element.querySelector("[data-service-card-title]")!;
        const text = title.querySelector("button") ?? title;
        const subtitle = element.querySelector("[data-service-card-subtitle]")!;
        const headerBox = element.getBoundingClientRect(), titleBox = title.getBoundingClientRect(), subtitleBox = subtitle.getBoundingClientRect();
        const icon = element.querySelector("[data-service-card-icon]")!;
        const style = getComputedStyle(text), box = icon.getBoundingClientRect(), iconStyle = getComputedStyle(icon);
        const range = document.createRange();
        range.selectNodeContents(text);
        const glyph = range.getBoundingClientRect();
        return { fontSize: style.fontSize, fontWeight: style.fontWeight, tracking: style.letterSpacing, titleHeight: titleBox.height, titleOffset: titleBox.top - headerBox.top, textOffset: glyph.top - titleBox.top, subtitleOffset: subtitleBox.top - headerBox.top, subtitleHeight: subtitleBox.height, icon: { width: box.width, height: box.height, radius: iconStyle.borderRadius, padding: iconStyle.padding, background: iconStyle.backgroundColor, border: iconStyle.border } };
      });
      expect(header.fontSize).toBe(`${15 * 0.85 * scale}px`);
      expect(header.fontWeight).toBe("600");
      expect(header.icon.width).toBeCloseTo(40 * density, 1);
      expect(header.icon.height).toBeCloseTo(40 * density, 1);
      if (!assistant) {
        const target = (await service.getByRole("button", { name: "GitHub", exact: true }).boundingBox())!;
        expect(target.height).toBeGreaterThanOrEqual(24);
        expect(target.width).toBeGreaterThanOrEqual(24);
      }
      headers.push(header);
      const cards = assistant ? await pluginCardBounds(page) : null;
      if (process.env.NYX_REVIEW_SHOTS && density === 1.4 && (scale === 0.5 || scale === 1)) {
        await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
        await service.evaluate(element => element.scrollIntoView({ block: "center", behavior: "instant" }));
        await page.screenshot({ animations: "disabled", path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${assistant ? "plugins" : "keys"}.png`) });
      }
      if (width === 390) await page.getByRole("button", { name: assistant ? "Open chats" : "Open menu", exact: true }).click();
      const nav = await navSiblingGaps(page, density);
      let collapsed;
      if (width === 1440) {
        if (assistant) await page.getByRole("button", { name: "Hide sidebar", exact: true }).click();
        else await page.evaluate(async () => (await import("/src/stores/theme-store.ts")).useThemeStore.getState().setSidebarMode("collapsed"));
        collapsed = await navSiblingGaps(page, density);
      }
      evidence.push({ path, header, cards, nav, collapsed });
    }
    expect(headers[1]).toEqual(headers[0]);
    expect(errors).toEqual([]);
    const metrics = JSON.stringify({ theme, width, scale, density, evidence }, null, 2);
    await test.info().attach("real-service-parity.json", { body: metrics, contentType: "application/json" });
    if (process.env.NYX_REVIEW_SHOTS) await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${density}-service-parity.json`), metrics);
    await context.close();
  });
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) for (const scale of [0.5, 1, 1.5, 2]) for (const density of [1, 1.4]) {
  test(`shared box treatment (${theme}, ${width}, text ${scale}×, spacing ${density}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => { window.addEventListener("error", event => { (window as Window & { treatmentErrors?: string[] }).treatmentErrors ??= []; (window as Window & { treatmentErrors: string[] }).treatmentErrors.push(event.message); }); });
    // Fixture mode serves local data; never send writes to a backend.
    await page.route("**/api/**", route => route.fulfill({ json: {} }));
    const shells: Record<string, unknown>[] = [];
    for (const path of ["/approvals/history", "/assistant/approvals"]) {
      await openRoute(page, path, scale, density, theme);
      if (!path.startsWith("/assistant")) {
        const toolbar = page.locator('header:has([aria-label="User menu"])');
        await expect.poll(() => toolbar.evaluate(header => {
          const outer = header.getBoundingClientRect();
          return [...header.querySelectorAll("button,a")].flatMap(button => {
            const box = button.getBoundingClientRect();
            return !box.width || !box.height || (box.top >= outer.top && box.bottom <= outer.bottom && box.left >= outer.left && box.right <= outer.right) ? [] : [{ label: button.getAttribute("aria-label") ?? button.textContent, box: box.toJSON(), header: outer.toJSON() }];
          });
        })).toEqual([]);
      }
      const sample: Record<string, unknown> = { controls: await readings(page, "[data-treatment-fixture] [data-parity]") };
      if (process.env.NYX_REVIEW_SHOTS) {
        const layout = await page.locator("[data-treatment-fixture]").evaluate(element => ({
          width: element.getBoundingClientRect().width,
          parentWidth: element.parentElement!.getBoundingClientRect().width,
          fontFamily: getComputedStyle(element).fontFamily,
          fonts: [...document.fonts].map(font => ({ family: font.family, weight: font.weight, status: font.status })),
          controls: [...element.querySelectorAll("[data-parity]")].map(control => ({ role: control.getAttribute("data-parity"), width: control.getBoundingClientRect().width, height: control.getBoundingClientRect().height })),
        }));
        await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
        await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${density}-${path.startsWith("/assistant") ? "assistant" : "studio"}-fixture-layout.json`), JSON.stringify(layout, null, 2));
      }
      sample.header = await readings(page, "[data-treatment-fixture] [data-page-header], [data-treatment-fixture] [data-page-heading], [data-treatment-fixture] [data-page-heading] :is(h1,h2), [data-treatment-fixture] [data-page-heading] > p, [data-treatment-fixture] [data-title-actions]");
      // Title tag names differ intentionally; compare the roles by stable position.
      sample.header = Object.values(sample.header as object);
      sample.group = await readings(page, "[data-treatment-fixture] section[aria-labelledby] > h3, [data-treatment-fixture] [data-grouped-footer]");
      sample.page = await page.locator(".shell-page-content").evaluate(el => { const s = getComputedStyle(el); return { padding: s.padding, gap: s.gap }; });
      for (const kind of ["dialog", "sheet", "menu", "popover", "select", "tooltip", "toast"]) {
        await page.evaluate(kind => window.treatmentFixture.portal(kind), kind);
        const selector = kind === "toast" ? "[data-sonner-toast]" : `[data-parity="${kind === "select" ? "select-content" : kind}"]`;
        await expect(page.locator(selector).first()).toBeVisible();
        if (kind === "toast") await expectReadableToast(page);
        sample[kind] = Object.values(await readings(page, kind === "toast" ? "[data-sonner-toast], [data-sonner-toast] :is([data-title],[data-description],[data-button])" : `${selector}, ${selector} [data-parity], ${selector} [data-icon-button], ${selector} [data-select-indicator]`));
        if (process.env.NYX_TREATMENT_SHOTS && density === 1 && (scale === 1 || scale === 2) && ["dialog", "menu", "toast"].includes(kind)) {
          await mkdir(process.env.NYX_TREATMENT_SHOTS, { recursive: true });
          await page.screenshot({ animations: "disabled", path: join(process.env.NYX_TREATMENT_SHOTS, `${theme}-${width}-${scale}-${path.startsWith("/assistant") ? "assistant" : "studio"}-${kind}.png`) });
        }
        await page.evaluate(() => window.treatmentFixture.portal(""));
        await expect(page.locator(selector).first()).toBeHidden();
        await expect.poll(() => page.evaluate(() => getComputedStyle(document.body).pointerEvents)).not.toBe("none");
      }
      await page.evaluate(() => window.treatmentFixture.portal(""));
      const assistant = path.startsWith("/assistant");
      if (width === 390) await page.getByRole("button", { name: assistant ? "Open chats" : "Open menu", exact: true }).click();
      const nav = page.locator(assistant ? '.shell-nav-item[href="/assistant/plugins"]' : '.shell-nav-item[href="/dashboard"]').filter({ visible: true }).first();
      await expect(nav).toBeVisible();
      sample.nav = Object.values(await readings(page, assistant ? '.shell-nav-item[href="/assistant/plugins"]:visible' : '.shell-nav-item[href="/dashboard"]:visible'))[0];
      sample.groupLabel = Object.values(await readings(page, '.shell-group-label:visible'))[0];
      if (width === 390) await page.getByRole("button", { name: assistant ? "Close chats" : "Close menu", exact: true }).last().click();
      if (width === 390 && !assistant) await expect(page.getByRole("button", { name: "Close menu", exact: true })).toHaveCount(0);
      if (width === 390 && assistant) await page.getByRole("button", { name: "Open chats", exact: true }).click();
      await page.getByRole("button", { name: assistant ? "Account menu" : "User menu", exact: true }).filter({ visible: true }).click();
      sample.account = Object.values(await readings(page, '[role="menu"] .shell-user-name, [role="menu"] .shell-user-email'));
      await page.keyboard.press("Escape");
      if (width === 390 && assistant) await page.getByRole("button", { name: "Close chats", exact: true }).last().click();
      if (process.env.NYX_TREATMENT_SHOTS && density === 1 && (scale === 1 || scale === 2)) {
        await page.locator('[data-treatment-fixture]').scrollIntoViewIfNeeded();
        await page.screenshot({ animations: "disabled", path: join(process.env.NYX_TREATMENT_SHOTS, `${theme}-${width}-${scale}-${path.startsWith("/assistant") ? "assistant" : "studio"}-controls.png`) });
      }
      expect(await page.evaluate(() => (window as Window & { treatmentErrors?: string[] }).treatmentErrors ?? [])).toEqual([]);
      shells.push(sample);
    }
    expect(shells[1]).toEqual(shells[0]);
    expect(errors).toEqual([]);
    const evidence = { theme, width, scale, density, shells };
    await test.info().attach("box-parity.json", { body: JSON.stringify(evidence, null, 2), contentType: "application/json" });
    if (process.env.NYX_TREATMENT_SHOTS) await writeFile(join(process.env.NYX_TREATMENT_SHOTS, `${theme}-${width}-${scale}-${density}-metrics.json`), JSON.stringify(evidence, null, 2));
    await context.close();
  });
}

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) {
  test(`equivalent page spacing (${theme}, ${width})`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    await page.route("**/api/**", route => route.fulfill({ json: {} }));
    await page.addInitScript(theme => localStorage.setItem("nyxid.theme", JSON.stringify({ state: { textScale: 1, density: 1, mode: theme }, version: 2 })), theme);
    const pages = [];
    for (const path of ["/approvals/history", "/assistant/approvals", "/keys", "/assistant/plugins"]) {
      await page.goto(`${path}?mock=1`);
      await expect(page.locator('[data-page-header]')).toBeVisible();
      const metrics = await page.locator('[data-page-header]').evaluate(header => {
        const s = getComputedStyle(header), heading = getComputedStyle(header.querySelector('[data-page-heading]')!), content = getComputedStyle(document.querySelector('.shell-page-content')!), stack = getComputedStyle(header.closest('.shell-page-stack')!);
        const grid = document.querySelector('.shell-card-grid');
        return { cardGap: grid ? getComputedStyle(grid).gap : null, headerColumnGap: s.columnGap, headerRowGap: s.rowGap, headingGap: heading.gap, pagePadding: content.padding, sectionGap: stack.gap };
      });
      pages.push({ path, metrics });
      if (process.env.NYX_TREATMENT_SHOTS) await page.screenshot({ animations: "disabled", path: join(process.env.NYX_TREATMENT_SHOTS, `${theme}-${width}-${path.replaceAll('/', '_')}-page.png`) });
    }
    for (const page of pages) {
      if (page.path === "/keys" || page.path === "/assistant/plugins") expect(page.metrics.cardGap).toBe("16px");
      expect({ ...page.metrics, cardGap: null }).toEqual({ ...pages[0]!.metrics, cardGap: null });
    }
    await test.info().attach("page-spacing.json", { body: JSON.stringify(pages, null, 2), contentType: "application/json" });
    if (process.env.NYX_TREATMENT_SHOTS) await writeFile(join(process.env.NYX_TREATMENT_SHOTS, `${theme}-${width}-page-spacing.json`), JSON.stringify(pages, null, 2));
    await context.close();
  });
}
