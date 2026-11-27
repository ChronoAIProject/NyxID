import { test, expect } from "@playwright/test";
import { openAvatarEditor } from "./avatar-editor-helpers";
test("all voice colourways animate in the real editor and stop for reduced motion", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const { editor } = await openAvatarEditor(page);
  await editor.getByRole("button", { name: "Motion", exact: true }).click();
  await page.bringToFront();
  for (const palette of [
    "nyx",
    "dusk",
    "aurora",
    "ember",
    "mint",
    "ocean",
    "neon",
    "midnight",
    "graphite",
    "sunrise",
    "lilac",
    "seafoam",
  ]) {
    await editor
      .getByRole("radiogroup", { name: "Voice colour", exact: true })
      .getByRole("radio", {
        name: palette[0]!.toUpperCase() + palette.slice(1),
        exact: true,
      })
      .click();
    await editor.locator("[data-voice-ripple]").scrollIntoViewIfNeeded();
    await expect(page.locator("[data-ripple-frame]")).toHaveAttribute(
      "data-ripple-frame",
      new RegExp(`^${palette}:`),
    );
    await expect(editor.locator("[data-voice-ripple]")).toHaveAttribute(
      "data-voice-ripple-moving",
      "true",
    );
    const frame = await page
      .locator("[data-ripple-frame]")
      .getAttribute("data-ripple-frame");
    await expect
      .poll(() =>
        page.locator("[data-ripple-frame]").getAttribute("data-ripple-frame"),
      )
      .not.toBe(frame);
    const colors = await page
      .locator("canvas[data-ripple-frame]")
      .evaluate((canvas: HTMLCanvasElement) => {
        const p = canvas.getContext("2d")!.getImageData(0, 0, 384, 128).data,
          s = new Set<string>();
        for (let i = 0; i < p.length; i += 4)
          s.add(`${p[i]},${p[i + 1]},${p[i + 2]},${p[i + 3]}`);
        return s.size;
      });
    expect(colors, palette).toBeGreaterThan(30);
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator("[data-voice-ripple]")).toHaveAttribute(
    "data-voice-ripple-moving",
    "false",
  );
  await expect(page.locator("[data-ripple-frame]")).toHaveAttribute(
    "data-ripple-frame",
    "seafoam:3",
  );
  expect(errors).toEqual([]);
});
test("actual Canvas and portable PNG agree across all surfaces within raster edge tolerances", async ({
  page,
}) => {
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  const result = await page.evaluate(async () => {
    const f = await import("/e2e/avatar-rendering-helpers.ts");
    return f.comparePixels();
  });
  for (const row of result) {
    expect(row.width, row.surface).toBe(512);
    expect(row.height, row.surface).toBe(512);
    // Dense glyphs have many antialiased edges: mean error stays below 2% per channel.
    expect(row.mean / 255, row.surface).toBeLessThan(0.02);
    expect(row.interior, row.surface).toBeLessThanOrEqual(16);
  }
});
