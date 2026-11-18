import { expect, test } from "@playwright/test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { openAssistant } from "./helpers";
import { installVoiceFixture } from "./voice-fixture";
import { expectActionTextContained, expectInternalContainment } from "./containment";

for (const theme of ["light", "dark"] as const) for (const width of [1440, 390]) for (const scale of [0.5, 1, 1.5, 2]) for (const density of [1, 1.4]) {
  test(`voice settings own input roles and stay internally contained (${theme}, ${width}, text ${scale}×, spacing ${density}×)`, async ({ browser }) => {
    const context = await browser.newContext({ viewport: { width, height: 1000 }, isMobile: width === 390, hasTouch: width === 390, reducedMotion: "reduce" });
    const page = await context.newPage(), errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(({ scale, density, theme }) => {
      localStorage.setItem("nyxid.theme", JSON.stringify({ state: { textScale: scale, density, mode: theme, motion: "reduce" }, version: 2 }));
      localStorage.setItem("nyxid.telemetry_consent.v3.test-user", JSON.stringify({ enabled: false, asked: true }));
    }, { scale, density, theme });
    const calls = await installVoiceFixture(page);
    await openAssistant(page, { faults: { nyxagentEnabled: true, voiceEnabled: true, modelControlsEnabled: true } });
    await page.getByRole("button", { name: /Thinking and permissions:|Model and permissions:/ }).click();
    await page.getByRole("menuitem", { name: "Voice settings", exact: true }).click();
    const panel = page.getByRole("region", { name: "Voice settings", exact: true });
    await expect(panel).toBeVisible();
    const selects = panel.getByRole("combobox");
    await expect(selects).toHaveCount(3);
    const readings = await selects.evaluateAll(elements => elements.map(element => {
      const style = getComputedStyle(element);
      const box = element.getBoundingClientRect(), icon = element.parentElement!.querySelector("svg")!.getBoundingClientRect();
      return { name: element.getAttribute("aria-label"), size: parseFloat(style.fontSize), weight: style.fontWeight, lineHeight: parseFloat(style.lineHeight), width: element.clientWidth, scrollWidth: element.scrollWidth, iconGap: icon.left - (box.right - parseFloat(style.paddingRight)) };
    }));
    for (const control of readings) {
      expect(control.size, control.name!).toBeCloseTo(Math.max(width === 390 ? 16 : 0, 14 * 0.85 * scale), 2);
      expect(control.weight, control.name!).toBe("400");
      expect(control.lineHeight, control.name!).toBeCloseTo(control.size * 1.5, 2);
      expect(control.scrollWidth, control.name!).toBeLessThanOrEqual(control.width + 1);
      expect(control.iconGap, control.name!).toBeGreaterThanOrEqual(8 * density - 1);
    }
    await expectInternalContainment(panel);
    await expectInternalContainment(page.locator('[role="dialog"]:visible'));
    await panel.getByRole("combobox", { name: "Microphone mode", exact: true }).selectOption("push_to_talk");
    const bar = panel.locator('.mt-4.flex.flex-wrap.gap-2').last();
    await expect(bar).toBeVisible();
    await expectActionTextContained(bar);
    expect(calls.starts).toBe(0);
    expect(calls.dictations).toBe(0);
    expect(errors).toEqual([]);
    const evidence = JSON.stringify({ theme, width, scale, density, readings }, null, 2);
    await test.info().attach("voice-input-roles.json", { body: evidence, contentType: "application/json" });
    if (process.env.NYX_REVIEW_SHOTS) {
      await mkdir(process.env.NYX_REVIEW_SHOTS, { recursive: true });
      await writeFile(join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-${density}-voice.json`), evidence);
      if (density === 1.4 && [1, 2].includes(scale)) {
        await page.screenshot({ animations: "disabled", path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-voice.png`) });
        await selects.first().evaluate(element => element.closest("label")!.scrollIntoView({ block: "start", behavior: "instant" }));
        await page.screenshot({ animations: "disabled", path: join(process.env.NYX_REVIEW_SHOTS, `${theme}-${width}-${scale}-voice-controls.png`) });
      }
    }
    await context.close();
  });
}
