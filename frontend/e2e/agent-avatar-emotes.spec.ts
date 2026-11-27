import { expect, test } from "@playwright/test";
import { openAvatarEditor } from "./avatar-editor-helpers";

test("the real assistant editor renders every pixel emote and animates its activity props", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const { editor, avatar } = await openAvatarEditor(page);
  await page.bringToFront();
  await editor.getByRole("button", { name: "Motion", exact: true }).click();
  const expression = editor.getByRole("combobox", {
    name: "Preview expression",
    exact: true,
  });
  for (const id of [
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
    await expression.selectOption(id);
    await editor.evaluate((element) =>
      element.closest("[data-compact-scroll]")!.scrollTo({ top: 0 }),
    );
    await expect(avatar).toHaveCSS("width", "112px");
    await expect(avatar).toBeInViewport();
    await expect(avatar).toHaveAttribute("data-avatar-renderer", "rive");
    await expect(avatar).toHaveAttribute("data-avatar-state", id);
    await expect(avatar).toHaveAttribute("data-avatar-style", "pixel");
    await expect(avatar.locator("svg g g").first()).toHaveAttribute(
      "transform",
      /scale\(1\)/,
    );
    const pixels = await avatar
      .locator("canvas[data-avatar-pixel]")
      .evaluate((canvas: HTMLCanvasElement) => {
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
        };
      });
    expect(pixels.grid).toBe(56);
    expect(pixels.colours).toBeGreaterThan(2);
    expect(pixels.colours).toBeLessThanOrEqual(8);
    expect(pixels.softAlpha).toBe(0);
    expect(pixels.rendering).toBe("pixelated");
  }
  // Props carry process meaning independently of the eyes.
  for (const [state, root, moving] of [
    ["drafting", "Keyboard", "TapLeft"],
    ["working", "Tool", "Wrench"],
    ["waiting", "Clock", "Minute"],
    ["thinking", "Thought", "ThoughtA"],
  ]) {
    await expression.selectOption(state!);
    await editor.evaluate((element) =>
      element.closest("[data-compact-scroll]")!.scrollTo({ top: 0 }),
    );
    await expect(avatar).toHaveCSS("width", "112px");
    await expect(avatar.locator(`[data-rive-prop="${root}"]`)).toHaveAttribute(
      "transform",
      /scale\(1 1\)/,
    );
    const node = avatar.locator(`[data-rive-prop="${moving}"]`);
    const output = avatar.locator("canvas[data-avatar-pixel]");
    const image = await output.evaluate((canvas: HTMLCanvasElement) =>
      canvas.toDataURL(),
    );
    const pose = await node.getAttribute("transform");
    await expect
      .poll(() =>
        output.evaluate((canvas: HTMLCanvasElement) => canvas.toDataURL()),
      )
      .not.toBe(image);
    await expect.poll(() => node.getAttribute("transform")).not.toBe(pose);
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  const thought = avatar.locator('[data-rive-prop="ThoughtA"]');
  await expect(avatar).not.toHaveAttribute("data-avatar-motion", "natural");
  const still = await thought.getAttribute("transform");
  await page.waitForTimeout(150);
  expect(await thought.getAttribute("transform")).toBe(still);
  expect(errors).toEqual([]);
});
