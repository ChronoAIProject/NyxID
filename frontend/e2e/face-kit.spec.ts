import { test, expect } from "@playwright/test";
test("all voice colourways animate, pause live, and retain the custom face", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/e2e/fixtures/face-kit.html");
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
    await page.getByRole("button", { name: palette, exact: true }).click();
    await expect(page.locator("[data-ripple-frame]")).toHaveAttribute(
      "data-ripple-frame",
      new RegExp(`^${palette}:`),
    );
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
  await page.goto("/e2e/fixtures/face-kit.html");
  const result = await page.evaluate(async () => {
    const f = await import("/e2e/fixtures/face-kit.tsx");
    return f.comparePixels();
  });
  console.log(JSON.stringify(result));
  for (const row of result) {
    expect(row.width, row.surface).toBe(512);
    expect(row.height, row.surface).toBe(512);
    // Dense glyphs have many antialiased edges: mean error stays below 2% per channel.
    expect(row.mean / 255, row.surface).toBeLessThan(0.02);
    expect(row.interior, row.surface).toBeLessThanOrEqual(16);
  }
});
