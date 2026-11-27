import { expect, test, type Locator } from "@playwright/test";
import { sendMessage } from "./helpers";

test.use({ reducedMotion: "reduce" });

/** Compare painted geometry and colours, excluding per-instance SVG clip ids. */
async function appearance(svg: Locator) {
  return svg.evaluate((element) =>
    JSON.stringify(
      [...element.querySelectorAll("path,rect,circle,image")].map((part) =>
        [
          "d",
          "fill",
          "stroke",
          "stroke-width",
          "opacity",
          "href",
          "transform",
          "r",
          "rx",
        ].map((key) => part.getAttribute(key)),
      ),
    ),
  );
}

test("saves in the editor and updates the sidebar and open chat, including reset", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  await expect(
    page.getByRole("button", { name: "Create new agent", exact: true }),
  ).toBeVisible();
  const thread = await page.evaluate(() => {
    const state = JSON.parse(
      sessionStorage.getItem("nyxagent-http-fixture-v4")!,
    );
    const nyx = state.agents.find(
      (agent: { kind: string }) => agent.kind === "nyxbot",
    );
    return state.rows.find(
      ([, row]: [string, { agentId: string }]) => row.agentId === nyx.id,
    )[0] as string;
  });
  await page.goto(
    `/assistant?mock=1&nyxbot=1&avatars=1&c=${thread}&details=overview`,
  );
  const editor = page.getByRole("form", { name: "Avatar", exact: true });
  const edit = page.getByRole("button", { name: "Edit avatar", exact: true });
  const overview = page.locator('svg[aria-label="Saved avatar"]');
  await expect(overview).toBeVisible();
  await expect(editor).toHaveCount(0);
  const originalOverview = await appearance(overview);
  await edit.click();
  await expect(
    editor.getByRole("heading", { name: "Choose a face" }),
  ).toBeVisible();
  await expect(
    editor.getByText("Download avatar", { exact: true }),
  ).toHaveCount(0);
  // The open details dialog hides the underlying page from the accessibility tree.
  const sidebar = page.locator(
    'button[aria-label="NyxBot — your personal agent"] svg[data-agent-avatar]',
  );
  const header = page.locator(
    '[data-avatar-renderer]:not([role="dialog"] *) svg[data-agent-avatar]',
  );
  const details = page.locator(".agent-header-avatar svg[data-agent-avatar]");
  const surfaces = [sidebar, header, details];
  for (const svg of surfaces) await expect(svg).toHaveCount(1);
  const original = await Promise.all(surfaces.map(appearance));
  const saveBox = await editor
    .getByRole("button", { name: "Save avatar", exact: true })
    .boundingBox();
  const headBox = await editor
    .getByRole("radiogroup", { name: "Head", exact: true })
    .boundingBox();
  expect(saveBox!.y + saveBox!.height).toBeLessThan(headBox!.y);
  await expect(editor.getByText("In the app", { exact: true })).toHaveCount(0);
  await expect(editor.locator('[aria-label="Compact previews"]')).toHaveCount(
    0,
  );
  await expect(
    editor.getByRole("radio", { name: "Nyx", exact: true }),
  ).toHaveCount(1);
  await expect(
    editor.getByRole("radio", { name: "NyxID logo", exact: true }),
  ).toHaveCount(0);

  await editor.getByRole("radio", { name: "Wide", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save avatar", exact: true }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Discard avatar", exact: true }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("region", { name: "Unsaved changes", exact: true }),
  ).toHaveCount(0);
  await editor.getByRole("radio", { name: "Wink", exact: true }).click();
  await editor
    .getByRole("button", { name: "Next: Style", exact: true })
    .click();
  await editor.getByRole("radio", { name: "Mint", exact: true }).click();
  await editor.getByRole("radio", { name: "Dither", exact: true }).click();
  await expect(
    editor.getByText(
      "Small icons use a simpler version so the face stays clear.",
    ),
  ).toHaveCount(0);
  const review = page.getByRole("dialog", {
    name: "Review changes",
    exact: true,
  });
  const save = async () => {
    await editor
      .getByRole("button", { name: "Save avatar", exact: true })
      .click();
    await expect(review).toBeVisible();
    await review
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(review).toBeHidden();
    await expect(editor.getByRole("status")).toHaveText("Avatar saved.");
    await expect(
      editor.getByRole("button", { name: "Save avatar", exact: true }),
    ).toBeDisabled();
  };
  // A draft never changes the saved appearance on Overview.
  expect(await appearance(overview)).toBe(originalOverview);
  await save();
  await expect.poll(() => appearance(overview)).not.toBe(originalOverview);
  for (const [index, svg] of surfaces.entries()) {
    await expect.poll(() => appearance(svg)).not.toBe(original[index]);
  }
  const ditherAppearance = await appearance(overview);
  for (const svg of surfaces) {
    await expect(svg).toHaveAttribute("data-face-surface", "dither-ordered");
    await expect.poll(() => appearance(svg)).toBe(ditherAppearance);
  }
  const saved = await Promise.all(surfaces.map(appearance));
  await page
    .getByRole("button", { name: "Back to Overview", exact: true })
    .click();
  await expect(editor).toHaveCount(0);
  await expect(edit).toBeFocused();
  await expect(overview).toBeVisible();
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  const homeCard = page
    .locator("li")
    .filter({ has: page.getByText("Your personal agent", { exact: true }) })
    .locator("svg[data-agent-avatar]");
  await expect(homeCard).toHaveCount(1);
  await expect(homeCard).toHaveAttribute("data-face-surface", "dither-ordered");
  expect(await appearance(homeCard)).toBe(ditherAppearance);
  await page.goto(
    `/assistant?mock=1&nyxbot=1&avatars=1&c=${thread}&details=overview`,
  );
  await expect(editor).toHaveCount(0);
  await edit.click();
  await expect(
    editor.getByRole("radio", { name: "Wink", exact: true }),
  ).toHaveAttribute("aria-checked", "true");
  for (const [index, svg] of surfaces.entries()) {
    await expect.poll(() => appearance(svg)).toBe(saved[index]);
  }
  // The original Nyx head and a character texture must also reach the real chat.
  await editor.getByRole("radio", { name: "Nyx", exact: true }).click();
  await editor.getByRole("radio", { name: "Dots", exact: true }).click();
  await editor
    .getByRole("button", { name: "Next: Style", exact: true })
    .click();
  await editor.getByRole("radio", { name: "Graphite", exact: true }).click();
  await editor.getByRole("radio", { name: "ASCII", exact: true }).click();
  await editor
    .getByRole("combobox", { name: "Texture style", exact: true })
    .selectOption("ascii-braille");
  await save();
  const asciiAppearance = await appearance(overview);
  for (const svg of surfaces) {
    await expect(svg).toHaveAttribute("data-face-surface", "ascii-braille");
    await expect.poll(() => appearance(svg)).toBe(asciiAppearance);
  }
  await page
    .getByRole("button", { name: "Back to Overview", exact: true })
    .click();
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  await expect(homeCard).toHaveAttribute("data-face-surface", "ascii-braille");
  expect(await appearance(homeCard)).toBe(asciiAppearance);
  await page.goto(
    `/assistant?mock=1&nyxbot=1&avatars=1&c=${thread}&details=overview`,
  );
  await edit.click();
  await editor
    .getByRole("button", { name: "Reset to default", exact: true })
    .click();
  await save();
  for (const [index, svg] of surfaces.entries()) {
    await expect.poll(() => appearance(svg)).toBe(original[index]);
  }
  await expect.poll(() => appearance(overview)).toBe(originalOverview);
  await editor.locator("nav").scrollIntoViewIfNeeded();
  const preview = page.getByRole("img", {
    name: "Avatar preview",
    exact: true,
  });
  expect((await preview.boundingBox())?.width).toBe(112);
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    for (const step of ["Face", "Style", "Motion"]) {
      await editor.getByRole("button", { name: step, exact: true }).click();
      await expect(
        editor.getByRole("button", { name: "Save avatar", exact: true }),
      ).toBeInViewport();
      const dimensions = await editor.evaluate((element) => ({
        width: element.clientWidth,
        scroll: element.scrollWidth,
      }));
      expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.width + 1);
    }
  }
  // Confirm that returning through an animated discard dialog also restores focus.
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await editor.getByRole("button", { name: "Face", exact: true }).click();
  await editor.getByRole("radio", { name: "Wink", exact: true }).click();
  await page
    .getByRole("button", { name: "Back to Overview", exact: true })
    .click();
  const discard = page.getByRole("dialog", {
    name: "Discard unsaved changes?",
    exact: true,
  });
  await discard
    .getByRole("button", { name: "Keep editing", exact: true })
    .click();
  await expect(
    editor.getByRole("radio", { name: "Wink", exact: true }),
  ).toHaveAttribute("aria-checked", "true");
  await page
    .getByRole("button", { name: "Back to Overview", exact: true })
    .click();
  await discard.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(discard).toHaveCount(0);
  await expect(editor).toHaveCount(0);
  await expect(edit).toBeFocused();
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(await appearance(overview)).toBe(originalOverview);
  expect(errors).toEqual([]);
});

test("shows larger avatars and live textured motion on the actual home and chat pages", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  await page.bringToFront();
  await expect(
    page.getByRole("button", { name: "Create new agent", exact: true }),
  ).toBeVisible();
  const thread = await page.evaluate(() => {
    const state = JSON.parse(
      sessionStorage.getItem("nyxagent-http-fixture-v4")!,
    );
    const nyx = state.agents.find(
      (agent: { kind: string }) => agent.kind === "nyxbot",
    );
    return state.rows.find(
      ([, row]: [string, { agentId: string }]) => row.agentId === nyx.id,
    )[0] as string;
  });
  // Save the texture through the real editor, then inspect the actual app surfaces.
  await page.goto(
    `/assistant?mock=1&nyxbot=1&avatars=1&c=${thread}&details=overview`,
  );
  await page.getByRole("button", { name: "Edit avatar", exact: true }).click();
  const editor = page.getByRole("form", { name: "Avatar", exact: true });
  await editor
    .getByRole("button", { name: "Next: Style", exact: true })
    .click();
  await editor.getByRole("radio", { name: "Dither", exact: true }).click();
  await editor
    .getByRole("button", { name: "Save avatar", exact: true })
    .click();
  const review = page.getByRole("dialog", {
    name: "Review changes",
    exact: true,
  });
  await review
    .getByRole("button", { name: "Save changes", exact: true })
    .click();
  await expect(editor.getByRole("status")).toHaveText("Avatar saved.");
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  const hero = page.locator("header [data-avatar-renderer]");
  const card = page
    .locator("li")
    .filter({ has: page.getByText("Your personal agent", { exact: true }) })
    .locator("svg[data-agent-avatar]");
  const assertMotion = async (avatar: Locator) => {
    await expect(avatar).toBeInViewport();
    await expect(avatar.locator("svg[data-agent-avatar]")).toHaveAttribute(
      "data-face-surface",
      "dither-ordered",
    );
    await expect(avatar).toHaveAttribute("data-avatar-renderer", "rive");
    await expect(avatar.locator("svg g g").first()).toHaveAttribute("transform", /scale\(1\)/);
  };
  await expect(hero).toHaveCount(1);
  expect((await hero.boundingBox())!.width).toBe(72);
  expect((await card.boundingBox())!.width).toBe(56);
  await assertMotion(hero);
  await expect(card).toHaveAttribute("data-face-surface", "dither-ordered");
  await expect(card.locator("xpath=..").locator("[data-avatar-renderer]")).toHaveCount(0);
  await page.screenshot({ path: "../.context/avatar-spacing-home.png" });
  await page.goto(`/assistant?mock=1&nyxbot=1&avatars=1&c=${thread}`);
  const header = page.locator(
    '[data-chat-header-controls][data-compact="false"] [data-avatar-renderer]',
  );
  await expect(header).toBeInViewport();
  expect((await header.boundingBox())!.width).toBe(56);
  await assertMotion(header);
  // The response fixture emits a live turn, then settles; historical replies do not animate success.
  await sendMessage(page, "Check my connected services");
  await expect(header).toHaveAttribute(
    "data-avatar-state",
    /thinking|working|drafting/,
  );
  await assertMotion(header);
  await expect(header).toHaveAttribute("data-avatar-state", "idle", {
    timeout: 10_000,
  });
  await assertMotion(header);
  await page.screenshot({ path: "../.context/avatar-spacing-chat.png" });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(header.locator("svg g g").first()).toHaveAttribute(
    "transform",
    /scale\(1\)/,
  );
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    await expect(header).toBeInViewport();
    const bounds = await header.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(width);
  }
  expect(errors).toEqual([]);
});
