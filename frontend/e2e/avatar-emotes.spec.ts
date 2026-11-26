import { expect, test } from "@playwright/test";

test("Rive performs every emote in one pixel style with pause and replay", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/avatar-emotes.html");
  await page.bringToFront();
  await expect(
    page.getByRole("heading", { name: "Emote preview", exact: true }),
  ).toBeVisible();
  await expect(page.locator("[data-emote-card]")).toHaveCount(12);
  await page.getByRole("checkbox", { name: "Use the same avatar" }).check();
  await expect(
    page.getByRole("combobox", { name: "Texture", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.locator('[data-emote-card] [data-avatar-style="pixel"]'),
  ).toHaveCount(12);
  const masks = new Set<string>();
  for (const state of [
    "idle",
    "thinking",
    "drafting",
    "working",
    "waiting",
    "approval",
    "retry",
    "stopping",
    "failure",
    "listening",
    "speaking",
    "success",
  ]) {
    const card = page.locator(`[data-emote-card="${state}"]`);
    await card.scrollIntoViewIfNeeded();
    const avatar = card.locator("[data-avatar-renderer]");
    await expect(avatar).toHaveAttribute("data-avatar-renderer", "rive");
    await expect(avatar).toHaveAttribute("data-avatar-state", state);
    await expect(avatar.locator("svg g g").first()).toHaveAttribute(
      "transform",
      /scale\(1\)/,
    );
    // A settled pose differs across the emotes while the outer face stays still.
    await expect
      .poll(async () =>
        avatar.locator("[data-avatar-eye-mask]").getAttribute("d"),
      )
      .not.toBeNull();
    await page.waitForTimeout(300);
    masks.add(
      (await avatar.locator("[data-avatar-eye-mask]").getAttribute("d"))!,
    );
  }
  expect(masks.size).toBeGreaterThanOrEqual(8);
  // Real rig transforms must drive each prop assembly, including its child motion.
  for (const [state, root, moving] of [
    ["drafting", "Keyboard", "TapLeft"],
    ["working", "Tool", "Wrench"],
    ["waiting", "Clock", "Minute"],
    ["thinking", "Thought", "ThoughtA"],
  ]) {
    const card = page.locator(`[data-emote-card="${state}"]`);
    await card.scrollIntoViewIfNeeded();
    await expect(card.locator(`[data-rive-prop="${root}"]`)).toHaveAttribute(
      "transform",
      /scale\(1 1\)/,
    );
    const node = card.locator(`[data-rive-prop="${moving}"]`);
    const output = card.locator("canvas[data-avatar-pixel]");
    const image = await output.evaluate((canvas: HTMLCanvasElement) =>
      canvas.toDataURL(),
    );
    const pose = await node.getAttribute("transform");
    await expect
      .poll(
        () =>
          output.evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL()),
        { timeout: 5000 },
      )
      .not.toBe(image);
    await expect
      .poll(() => node.getAttribute("transform"), { timeout: 5000 })
      .not.toBe(pose);
  }
  await page
    .getByRole("combobox", { name: "Preview size", exact: true })
    .selectOption("56");
  await expect(
    page.locator(
      '[data-emote-card] [data-avatar-props][data-prop-detail="compact"]',
    ),
  ).toHaveCount(12);
  expect(
    (
      await page
        .locator('[data-emote-card="drafting"] [data-avatar-renderer]')
        .boundingBox()
    )?.width,
  ).toBe(56);
  await page
    .getByRole("combobox", { name: "Preview size", exact: true })
    .selectOption("112");
  await expect(
    page.locator(
      '[data-emote-card] [data-avatar-props][data-prop-detail="full"]',
    ),
  ).toHaveCount(12);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(page.locator('[data-avatar-motion="natural"]')).toHaveCount(0);
  const canvases = page.locator("[data-emote-card] canvas[data-avatar-pixel]");
  await expect(canvases).toHaveCount(12);
  const pixels = await canvases.evaluateAll((elements) =>
    elements.map((element) => {
      const canvas = element as HTMLCanvasElement;
      const values = canvas
        .getContext("2d")!
        .getImageData(0, 0, canvas.width, canvas.height).data;
      const colours = new Set<string>();
      let softAlpha = 0;
      for (let i = 0; i < values.length; i += 4) {
        if (values[i + 3] !== 0 && values[i + 3] !== 255) softAlpha++;
        if (values[i + 3])
          colours.add(`${values[i]}:${values[i + 1]}:${values[i + 2]}`);
      }
      return {
        grid: canvas.width,
        colours: colours.size,
        softAlpha,
        rendering: getComputedStyle(canvas).imageRendering,
        opacity: getComputedStyle(canvas).opacity,
      };
    }),
  );
  for (const pixel of pixels) {
    expect(pixel.grid).toBe(56);
    expect(pixel.colours).toBeGreaterThan(2);
    expect(pixel.colours).toBeLessThanOrEqual(8);
    expect(pixel.softAlpha).toBe(0);
    expect(pixel.rendering).toBe("pixelated");
    expect(pixel.opacity).toBe("1");
  }
  await page.getByRole("button", { name: "Play", exact: true }).click();
  const typing = page.locator('[data-emote-card="drafting"]');
  await typing.scrollIntoViewIfNeeded();
  await expect(typing.locator("[data-avatar-renderer]")).toHaveAttribute(
    "data-avatar-renderer",
    "rive",
  );
  await page
    .getByRole("button", { name: "Randomise avatars", exact: true })
    .click();
  await expect(page.locator("[data-emote-card]")).toHaveCount(12);
  await page.getByRole("button", { name: "Replay all", exact: true }).click();
  await page.getByRole("button", { name: "Light theme", exact: true }).click();
  await expect(page.locator("html")).toHaveClass("theme-light");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(page.locator('[data-avatar-motion="natural"]')).toHaveCount(0);
  const clock = page.locator(
    '[data-emote-card="waiting"] [data-rive-prop="Minute"]',
  );
  const stillClock = await clock.getAttribute("transform");
  await page.waitForTimeout(150);
  expect(await clock.getAttribute("transform")).toBe(stillClock);
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(width);
  }
  expect(errors).toEqual([]);
});
