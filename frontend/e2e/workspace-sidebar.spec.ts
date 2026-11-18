import { expect, test, type Locator, type Page } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { openAssistant } from "./helpers";

async function revealNavigation(page: Page, assistant: boolean) {
  // URL commitment precedes onResolved, which dismisses the mobile drawer.
  // Reopen only after that event rather than racing it with a second click.
  await expect.poll(() => page.evaluate(() => {
    const app = (window as { __nyxApp?: { router: { state: { status: string; resolvedLocation?: { href: string }; location: { href: string } } } } }).__nyxApp;
    return !!app && app.router.state.status === "idle" && app.router.state.resolvedLocation?.href === app.router.state.location.href;
  })).toBe(true);
  const toggle = page.getByRole("button", { name: assistant ? "Open chats" : "Open menu", exact: true });
  if (page.viewportSize()!.width < 768) await toggle.click();
}
async function installWorkspaceFixtures(page: Page) {
  await page.route("**/api/v1/triggers", route => route.fulfill({ json: { triggers: [] } }));
  await page.route("**/api/v1/machines/updates", route => route.fulfill({ json: [] }));
  await page.route("**/api/v1/remote-machines", route => route.fulfill({ json: { machines: [], next_cursor: null } }));
  await page.route("**/api/v1/service-pools", route => route.fulfill({ json: { pools: [] } }));
  await page.route("**/api/v1/service-preferences", route => route.fulfill({ status: 404, json: { message: "No saved ordering" } }));
}
async function enablePreviews(page: Page) {
  await page.evaluate(() => {
    const { authStore } = (window as unknown as { __nyxApp: { authStore: typeof import("../src/stores/auth-store").useAuthStore } }).__nyxApp;
    const user = authStore.getState().user!;
    authStore.setState({ user: { ...user, capabilities: { ...user.capabilities, enabled_features: [...(user.capabilities?.enabled_features ?? []), "assistant:workspace-preview"] } } });
  });
}
async function alignment(rows: Locator) {
  return rows.evaluateAll(rows => rows.map(row => {
    const icon = row.querySelector(":scope > svg")!.getBoundingClientRect();
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    let label: Text | undefined;
    while (walker.nextNode()) if (walker.currentNode.textContent?.trim()) { label = walker.currentNode as Text; break; }
    const range = document.createRange();
    range.selectNodeContents(label!);
    return { name: label!.textContent!.trim(), iconX: icon.x, labelX: range.getBoundingClientRect().x, iconWidth: icon.width };
  }));
}
async function restFrame(glyph: Locator) {
  return glyph.evaluate(element => element.innerHTML.replace(/nav-glyph-roof-[\w-]+/g, "roof"));
}

for (const theme of ["light", "dark"] as const) {
  for (const mode of ["expanded", "rail", "mobile"] as const) test(`workspace glyph activation survives navigation (${theme}, ${mode})`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width: mode === "mobile" ? 390 : 1440, height: 1000 }, hasTouch: mode === "mobile", isMobile: mode === "mobile", reducedMotion: "no-preference" });
    const page = await context.newPage(), errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await installWorkspaceFixtures(page);
    await page.addInitScript(theme => { localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode: theme, motion: "system" }, version: 2 })); }, theme);
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    if (mode === "rail") await page.getByRole("button", { name: "Hide sidebar", exact: true }).click();
    await revealNavigation(page, true);
    await page.clock.install();
    const scope = mode === "rail" ? page.getByRole("navigation", { name: "Workspace and agents", exact: true }) : page.locator('aside[data-tour="sidebar"]:visible');
    for (const name of ["plugins", "approvals", "automations", "machines", "home"]) {
      const glyph = scope.locator(`svg[data-nav-glyph="${name}"]`);
      const row = glyph.locator("..");
      const rest = await restFrame(glyph);
      await row.hover();
      await row.focus();
      await expect(glyph).not.toHaveAttribute("data-playing");
      expect(await restFrame(glyph)).toBe(rest);
      if (mode === "mobile") await row.tap(); else await row.click();
      await expect(page).toHaveURL(new RegExp(`/assistant${name === "home" ? "(?:\\?|$)" : `/${name}(?:\\?|$)`}`));
      if (mode === "mobile") await revealNavigation(page, true);
      await expect(glyph).toHaveAttribute("data-playing", "true");
      await page.clock.runFor(400);
      const animations = await glyph.evaluate(element => element.getAnimations({ subtree: true }).map(animation => animation.effect!.getTiming().iterations));
      for (const iterations of animations) expect(iterations).toBe(1);
      if (process.env.NYX_REVIEW_SHOTS) {
        await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
        await page.screenshot({ path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${mode}-${name}-activated.png`) });
      }
      await page.clock.fastForward(5000);
      await expect(glyph).not.toHaveAttribute("data-playing");
      expect(await restFrame(glyph)).toBe(rest);
      // Real native Enter and Space activation; another activation restarts.
      for (const key of ["Enter", "Space"]) {
        await row.press(key);
        if (mode === "mobile") await revealNavigation(page, true);
        await expect(glyph).toHaveAttribute("data-playing", "true");
      }
      await page.clock.fastForward(5000);
      await expect(glyph).not.toHaveAttribute("data-playing");
      expect(await restFrame(glyph)).toBe(rest);
    }
    expect(errors).toEqual([]);
    await context.close();
  });

  test(`reduced motion never starts workspace glyphs (${theme})`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
    const page = await context.newPage();
    await installWorkspaceFixtures(page);
    await page.addInitScript(theme => { localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode: theme, motion: "system" }, version: 2 })); }, theme);
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    const row = page.locator('aside:visible [href="/assistant/plugins"]');
    await row.click();
    await expect(page.locator('svg[data-nav-glyph="plugins"]:visible')).not.toHaveAttribute("data-playing");
    await context.close();
  });

  for (const width of [1440, 390]) for (const scale of [0.5, 1, 1.5, 2]) for (const density of [1, 1.4]) test(`all workspace nav rows align (${theme}, ${width}, text ${scale}×, spacing ${density}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage();
    await installWorkspaceFixtures(page);
    await page.addInitScript(({ theme, scale, density }) => { localStorage.setItem("nyxid.theme", JSON.stringify({ state: { mode: theme, textScale: scale, density, motion: "reduce" }, version: 2 })); }, { theme, scale, density });
    await openAssistant(page, { faults: { nyxagentEnabled: true } });
    await enablePreviews(page);
    await revealNavigation(page, true);
    const rows = page.locator('.shell-nav-stack:visible > .shell-nav-item, nav[aria-label="Agents and conversations"]:visible > .shell-nav-item');
    await expect(rows.filter({ hasText: "Artifacts" })).toBeVisible();
    await expect(rows.filter({ hasText: "Activity" })).toBeVisible();
    await expect(rows.filter({ hasText: "Create new agent" })).toBeVisible();
    const assistant = await alignment(rows);
    for (const name of ["Home", "Automations", "Machines", "Plugins", "Approvals", "Artifacts", "Activity", "Create new agent"]) expect(assistant.some(row => row.name === name)).toBe(true);
    for (const row of assistant) {
      expect(row.iconX, row.name).toBeCloseTo(assistant[0]!.iconX, 1);
      expect(row.labelX, row.name).toBeCloseTo(assistant[0]!.labelX, 1);
      expect(row.iconWidth, row.name).toBeCloseTo(assistant[0]!.iconWidth, 1);
    }
    if (process.env.NYX_REVIEW_SHOTS && [1, 2].includes(scale) && density === 1.4) await page.screenshot({ path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-workspace-alignment.png`) });
    await page.goto("/keys?mock=1");
    await revealNavigation(page, false);
    const studioRows = page.locator('.shell-nav-stack:visible').first().locator(':scope > .shell-nav-item');
    const studio = await alignment(studioRows);
    expect(studio.length).toBeGreaterThan(1);
    for (const row of studio) {
      expect(row.iconX, row.name).toBeCloseTo(studio[0]!.iconX, 1);
      expect(row.labelX, row.name).toBeCloseTo(studio[0]!.labelX, 1);
      expect(row.iconWidth, row.name).toBeCloseTo(studio[0]!.iconWidth, 1);
    }
    if (process.env.NYX_REVIEW_SHOTS) await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${density}-workspace-alignment.json`), JSON.stringify({ assistant, studio }, null, 2));
    await context.close();
  });
}
