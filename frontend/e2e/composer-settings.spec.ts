import { expect, test } from "@playwright/test";
import { composerInput, openAssistant, sendMessage } from "./helpers";

for (const width of [1280, 390, 320]) {
  test(`combined composer settings stay in the bottom toolbar at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, {
      faults: { nyxagentEnabled: true, modelControlsEnabled: true },
    });
    await page.getByRole("button", { name: "Switch to dark mode" }).click();
    const input = composerInput(page);
    const trigger = page.getByRole("button", {
      name: "Model and permissions: Claude Sonnet 4.5, Auto",
    });
    await expect(trigger).toBeVisible();
    await input.fill("Review the latest changes\nand explain the next steps.");
    const attach = page.getByRole("button", { name: "Attach files" });
    const send = page.getByRole("button", {
      name: "Send message",
      exact: true,
    });
    const textBounds = await input.boundingBox();
    const controls = await Promise.all(
      [attach, trigger, send].map((control) => control.boundingBox()),
    );
    for (const bounds of controls) {
      expect(bounds!.y).toBeGreaterThanOrEqual(
        textBounds!.y + textBounds!.height,
      );
      expect(bounds!.x).toBeGreaterThanOrEqual(0);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      expect(Math.abs(bounds!.y - controls[0]!.y)).toBeLessThanOrEqual(3);
    }
    await trigger.click();
    const menu = page.getByRole("menu", { name: "Model and permissions" });
    await expect(menu).toBeVisible();
    await expect(menu.getByRole("menuitem")).toHaveCount(4);
    await expect(menu.getByRole("menuitem", { name: /Next message connection/ })).toBeVisible();
    await expect(
      menu.getByRole("menuitemcheckbox", { name: "Thinking mode" }),
    ).toHaveCount(0);
    await expect(page.getByRole("menuitemradio")).toHaveCount(0);
    await expect(
      menu.getByRole("menuitem", { name: /Reasoning/ }),
    ).toHaveAttribute("aria-disabled", "true");
    const menuBounds = await menu.boundingBox();
    expect(menuBounds!.x).toBeGreaterThanOrEqual(0);
    expect(menuBounds!.x + menuBounds!.width).toBeLessThanOrEqual(width);
    await page.screenshot({
      path: testInfo.outputPath(`composer-${width}.png`),
      animations: "disabled",
    });
    await menu.getByRole("menuitem", { name: /^Permissions/ }).click();
    const permissions = page.getByRole(width < 640 ? "group" : "menu", {
      name: /^Permissions/,
    });
    await expect(permissions).toBeVisible();
    await expect(
      permissions.getByRole("menuitemradio", { name: "Auto", exact: true }),
    ).toHaveAttribute("aria-checked", "true");
    await page.screenshot({
      path: testInfo.outputPath(`permissions-${width}.png`),
      animations: "disabled",
    });
    const permissionBounds = (await permissions.boundingBox())!;
    expect(permissionBounds.x).toBeGreaterThanOrEqual(0);
    expect(permissionBounds.x + permissionBounds.width).toBeLessThanOrEqual(
      width,
    );
    await permissions
      .getByRole("menuitemradio", { name: "Ask every time", exact: true })
      .click();
    await expect(
      page.getByRole("button", {
        name: "Model and permissions: Claude Sonnet 4.5, Ask every time",
      }),
    ).toBeVisible();
    await input.focus();
    await input.fill("");
  });
}

test("model choice applies to a new chat, changes an existing chat, and survives reload", async ({
  page,
}) => {
  await openAssistant(page, {
    faults: { nyxagentEnabled: true, modelControlsEnabled: true },
  });
  await page
    .getByRole("button", { name: /Model and permissions: Claude Sonnet 4.5/ })
    .click();
  await page.getByRole("menuitem", { name: /^Model / }).click();
  await page.getByRole("menuitemradio", { name: "GPT-5", exact: true }).click();
  await sendMessage(page, "Use the selected model");
  await expect(page).toHaveURL(/c=nyxa-/);
  const trigger = page.getByRole("button", {
    name: /Model and permissions: GPT-5/,
  });
  await expect(trigger).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Stop assistant turn" }),
  ).toHaveCount(0);
  await trigger.click();
  await page.getByRole("menuitem", { name: /^Model / }).click();
  await page
    .getByRole("menuitemradio", { name: "Claude Sonnet 4.5", exact: true })
    .click();
  await expect(
    page.getByRole("button", {
      name: /Model and permissions: Claude Sonnet 4.5/,
    }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("button", {
      name: /Model and permissions: Claude Sonnet 4.5/,
    }),
  ).toBeVisible();
});

test("narrow steering composer keeps settings, Stop, and Steer reachable", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 844 });
  await openAssistant(page, {
    faults: {
      nyxagentEnabled: true,
      modelControlsEnabled: true,
      progressStallMs: 60_000,
    },
  });
  await page.evaluate(() => {
    const original = globalThis.__nyxidAssistantHttpMock;
    if (!original) throw new Error("Missing chat fixture");
    globalThis.__nyxidAssistantHttpMock = async (request) => {
      if (request.endpoint.endsWith("/capabilities")) {
        return new Response(
          JSON.stringify({ steer: { max_chars: 32_000, max_per_turn: 20 } }),
          { headers: { "content-type": "application/json" } },
        );
      }
      const response = await original(request);
      if (
        response?.ok &&
        (request.init.method ?? "GET") === "GET" &&
        /\/conversations\/nyxa-[a-f0-9]{32}(?:\?|$)/.test(request.endpoint)
      ) {
        const history = await response.json();
        if (history.conversation?.active_turn)
          history.conversation.active_turn.steering_allowed = true;
        return new Response(JSON.stringify(history), {
          headers: { "content-type": "application/json" },
        });
      }
      return response;
    };
  });
  await sendMessage(page, "Keep working while I review");
  await expect(page).toHaveURL(/c=nyxa-/);
  await expect(composerInput(page)).toBeEnabled();
  await composerInput(page).fill("Focus on this detail\nand check the result");
  const trigger = page.getByRole("button", { name: /Model and permissions:/ });
  const controls = [
    page.getByRole("button", { name: "Attach files" }),
    trigger,
    page.getByRole("button", { name: "Stop assistant turn" }),
    page.getByRole("button", { name: "Send guidance" }),
  ];
  let previousEnd = 0;
  for (const control of controls) {
    const bounds = await control.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(previousEnd);
    previousEnd = bounds!.x + bounds!.width;
    expect(previousEnd).toBeLessThanOrEqual(320);
  }
  await trigger.click();
  await page.getByRole("menuitem", { name: /^Model / }).click();
  await expect(
    page.getByRole("menuitemradio", { name: "GPT-5", exact: true }),
  ).toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Stop assistant turn" }).click();
  await expect(
    page.getByRole("button", { name: "Stop assistant turn" }),
  ).toHaveCount(0);
});

for (const width of [1280, 320]) {
  test(`voice preview respects the feature flag at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, {
      faults: {
        nyxagentEnabled: true,
        modelControlsEnabled: true,
        voiceEnabled: false,
      },
    });
    const voice = page.getByRole("button", { name: "Open voice call" });
    await expect(voice).toHaveCount(0);

    await openAssistant(page, {
      faults: {
        nyxagentEnabled: true,
        modelControlsEnabled: true,
        voiceEnabled: true,
      },
    });
    await page.getByRole("button", { name: "Switch to dark mode" }).click();
    await expect(voice).toBeVisible();
    const trigger = page.getByRole("button", {
      name: /Model and permissions:/,
    });
    const controls = [
      page.getByRole("button", { name: "Attach files" }),
      trigger,
      voice,
    ];
    let previousEnd = 0;
    for (const control of controls) {
      const bounds = await control.boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(previousEnd);
      previousEnd = bounds!.x + bounds!.width;
      expect(previousEnd).toBeLessThanOrEqual(width);
    }
    const send = page.getByRole("button", {
      name: "Send message",
      exact: true,
    });
    await expect(send).toHaveCount(0);
    const editor = page.locator("[data-composer-editor]");
    const input = composerInput(page);
    await expect(editor).not.toHaveClass(/flex-col/);
    await expect
      .poll(async () => (await input.boundingBox())!.height)
      .toBeLessThanOrEqual(34);
    await input.fill("Hi");
    await expect(editor).not.toHaveClass(/flex-col/);
    await expect(send).toBeEnabled();
    await expect(voice).toHaveCount(0);
    await input.fill(
      "Review these changes and explain what needs to be done next. ".repeat(4),
    );
    await expect(editor).toHaveClass(/flex-col/);
    await input.fill("   ");
    await expect(editor).not.toHaveClass(/flex-col/);
    await expect(voice).toBeVisible();
    await expect(send).toHaveCount(0);
    await input.fill("");
    const composer = page.locator("[data-composer-input]");
    await composer.screenshot({
      path: testInfo.outputPath(`voice-composer-${width}.png`),
      animations: "disabled",
    });
    await trigger.click();
    const menu = page.getByRole("menu", { name: "Model and permissions" });
    await expect(menu).toBeVisible();
    await expect(menu).toHaveCSS("opacity", "1");
    const menuBounds = (await menu.boundingBox())!;
    const composerBounds = (await composer.boundingBox())!;
    const left = Math.min(menuBounds.x, composerBounds.x);
    const top = Math.min(menuBounds.y, composerBounds.y);
    await page.screenshot({
      path: testInfo.outputPath(`voice-menu-${width}.png`),
      animations: "disabled",
      clip: {
        x: left,
        y: top,
        width:
          Math.max(
            menuBounds.x + menuBounds.width,
            composerBounds.x + composerBounds.width,
          ) - left,
        height: composerBounds.y + composerBounds.height - top,
      },
    });
  });
}

for (const width of [1280, 320]) {
  test(`external Thinking toggle selects chat and research and persists at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, {
      faults: { nyxagentEnabled: true, voiceEnabled: true },
    });
    await page.getByRole("button", { name: "Switch to dark mode" }).click();
    const trigger = page.getByRole("button", {
      name: /Thinking and permissions:/,
    });
    await expect(trigger).toHaveText("Auto");
    await trigger.click();
    const menu = page.getByRole("menu", { name: "Thinking and permissions" });
    await expect(menu.getByRole("menuitem")).toHaveCount(2);
    await expect(menu.getByRole("menuitem", { name: /Next message connection/ })).toBeVisible();
    await expect(
      menu.getByRole("menuitem", { name: /^Permissions/ }),
    ).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: /^(Model|Reasoning)/ }),
    ).toHaveCount(0);
    const thinking = page.getByRole("menuitemcheckbox", {
      name: "Thinking mode",
    });
    await expect(thinking).toHaveAttribute("aria-checked", "false");
    await thinking.click();
    await expect(thinking).toHaveAttribute("aria-checked", "true");
    await page.screenshot({
      path: testInfo.outputPath(`external-thinking-${width}.png`),
      animations: "disabled",
    });
    await page.keyboard.press("Escape");
    await sendMessage(page, "Use thinking mode for this reply");
    await expect(page).toHaveURL(/c=nyxa-/);
    await expect(trigger).toHaveText("Auto, Thinking");
    await expect(
      page.getByRole("button", { name: "Stop assistant turn" }),
    ).toHaveCount(0);
    await page.reload();
    await expect(trigger).toHaveText("Auto, Thinking");
    await trigger.click();
    await thinking.click();
    await expect(thinking).toHaveAttribute("aria-checked", "false");
    await page.keyboard.press("Escape");
    await page.reload();
    await expect(trigger).toHaveText("Auto");
    await trigger.click();
    await page.getByRole("menuitem", { name: /^Permissions/ }).click();
    await page
      .getByRole("menuitemradio", { name: "Full access", exact: true })
      .click();
    await expect(trigger).toHaveText("Full access");
    await trigger.click();
    await thinking.click();
    await expect(thinking).toHaveAttribute("aria-checked", "true");
    await page.keyboard.press("Escape");
    await expect(trigger).toHaveText("Full access, Thinking");
  });
}

test("overflow stays scrollable after resize with controls directly below the text", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAssistant(page, { faults: { nyxagentEnabled: true, modelControlsEnabled: true } });
  const input = composerInput(page);
  await input.fill("Review these changes and explain what needs to be done next. ".repeat(3));
  await expect(input).toHaveCSS("overflow-y", "hidden");
  await page.setViewportSize({ width: 320, height: 900 });
  await expect(input).toHaveCSS("overflow-y", "scroll");
  await expect.poll(() => input.evaluate((element) => element.scrollHeight > element.clientHeight)).toBe(true);
  const scrollbar = page.locator("[data-composer-scrollbar]");
  await expect(scrollbar).toBeVisible();
  const track = (await scrollbar.boundingBox())!;
  await scrollbar.click({ position: { x: track.width / 2, y: track.height - 2 } });
  await expect.poll(() => input.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  const footer = page.locator("[data-composer-footer]");
  await expect.poll(async () => {
    const text = (await input.boundingBox())!;
    const controls = (await footer.boundingBox())!;
    return controls.y - (text.y + text.height);
  }).toBeLessThanOrEqual(3);
  await expect(page.getByRole("button", { name: "Send message", exact: true })).toBeVisible();
  await page.locator("[data-composer-input]").screenshot({ path: testInfo.outputPath("overflow-mobile.png"), animations: "disabled" });
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect(input).toHaveCSS("overflow-y", "hidden");
  await input.fill("Short draft");
  await expect(scrollbar).toHaveCount(0);
  await expect(page.locator("[data-composer-editor]")).not.toHaveClass(/flex-col/);
});

test("multiline composer follows drag height before the draft overflows", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAssistant(page, { faults: { nyxagentEnabled: true, voiceEnabled: true } });
  const input = composerInput(page);
  const draft = "Review the changes.\nExplain the tradeoffs.\nSuggest the next step.";
  await input.fill(draft);
  const draftBefore = await input.innerText();
  const handle = page.getByRole("separator", { name: "Resize composer" });
  await expect(handle).toBeVisible();
  await expect(input).toHaveCSS("overflow-y", "hidden");
  // Wait for the handle to be stable before reading coordinates for raw mouse
  // input; auto-growth can still be animating when the handle first appears.
  await handle.hover();
  const initial = (await input.boundingBox())!.height;
  const grip = (await handle.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2 - 160, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await input.boundingBox())!.height).toBeGreaterThan(initial + 140);
  const resized = (await input.boundingBox())!.height;
  expect(await input.innerText()).toBe(draftBefore);
  await handle.focus();
  await page.keyboard.press("ArrowDown");
  await expect.poll(async () => (await input.boundingBox())!.height).toBeLessThan(resized - 15);
  await handle.dblclick();
  await expect.poll(async () => Math.abs((await input.boundingBox())!.height - initial)).toBeLessThan(2);
  expect(await input.innerText()).toBe(draftBefore);
});

test("external composer resizes while short drafts remain compact", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await openAssistant(page, { faults: { nyxagentEnabled: true, voiceEnabled: true } });
  const input = composerInput(page);
  const draft = "Review the recent changes and explain the next steps. ".repeat(40);
  await input.fill(draft);
  const handle = page.getByRole("separator", { name: "Resize composer" });
  await expect(handle).toBeVisible();
  await handle.hover();
  const initialHeight = (await input.boundingBox())!.height;
  const grip = (await handle.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2 - 160, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await input.boundingBox())!.height).toBeGreaterThan(initialHeight + 120);
  const expandedHeight = (await input.boundingBox())!.height;
  await page.locator("[data-composer-input]").screenshot({ path: testInfo.outputPath("external-resized-composer.png"), animations: "disabled" });
  await input.fill("Hi");
  await expect(page.locator("[data-composer-editor]")).not.toHaveClass(/flex-col/);
  await expect(handle).toHaveCount(0);
  await input.fill(draft);
  await expect.poll(async () => Math.abs((await input.boundingBox())!.height - expandedHeight)).toBeLessThan(2);
  await handle.focus();
  await page.keyboard.press("ArrowDown");
  await expect.poll(async () => (await input.boundingBox())!.height).toBeLessThan(expandedHeight - 15);
  await page.setViewportSize({ width: 390, height: 500 });
  await expect.poll(async () => (await page.locator("[data-composer-surface]").boundingBox())!.height).toBeLessThanOrEqual(450);
  await handle.dblclick();
  await expect.poll(async () => (await input.boundingBox())!.height).toBeLessThan(120);
  await input.focus();
  await input.fill("");
  await expect(input).toHaveText("");
  await expect(page.getByRole("button", { name: "Open voice call" })).toBeVisible();
});

for (const width of [390, 1280]) {
  test(`composer grows to 90vh and overlays chat beyond its reserved band at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true, voiceEnabled: true } });
    await sendMessage(page, "List connected services");
    const reply = page.getByText("Your connected services are ready.", { exact: false });
    await expect(reply).toBeVisible();
    const input = composerInput(page);
    const draft = "Review the changes.\nExplain the tradeoffs.\nSuggest the next step.";
    await input.fill(draft);
    const originalText = await input.innerText();
    const handle = page.getByRole("separator", { name: "Resize composer" });
    const surface = page.locator("[data-composer-surface]");
    const band = page.locator("[data-composer-band]");
    await handle.hover();
    const initial = (await surface.boundingBox())!;
    const grip = (await handle.boundingBox())!;
    await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
    await page.mouse.down();
    await page.mouse.move(grip.x + grip.width / 2, 10, { steps: 20 });
    await page.mouse.up();
    await expect.poll(async () => Math.abs((await surface.boundingBox())!.height - 810)).toBeLessThan(2);
    await expect.poll(async () => (await band.boundingBox())!.height).toBe(450);
    const transcript = page.locator("main div.flex-1.overflow-y-auto > div").first();
    await expect(transcript).toHaveCSS("padding-bottom", "474px");
    const full = (await surface.boundingBox())!;
    const reserved = (await band.boundingBox())!;
    expect(full.y).toBeLessThan(reserved.y - 350);
    expect(Math.abs(full.y + full.height - initial.y - initial.height)).toBeLessThan(2);
    await expect(page.getByRole("button", { name: "Send message", exact: true })).toBeInViewport();
    expect(await input.innerText()).toBe(originalText);
    await page.screenshot({ path: testInfo.outputPath(`composer-overlay-${width}.png`), animations: "disabled" });
    const replyTop = (await reply.boundingBox())!.y;
    await handle.focus();
    await page.keyboard.press("ArrowDown");
    await expect.poll(async () => (await surface.boundingBox())!.height).toBeLessThan(790);
    await expect(transcript).toHaveCSS("padding-bottom", "474px");
    expect((await reply.boundingBox())!.y).toBe(replyTop);

    // Shrinking below the threshold returns to normal measured chat spacing.
    await handle.focus();
    await page.keyboard.press("Home");
    await expect.poll(async () => Math.abs((await band.boundingBox())!.height - (await surface.boundingBox())!.height)).toBeLessThan(2);
    await page.keyboard.press("End");
    await expect.poll(async () => Math.abs((await surface.boundingBox())!.height - 810)).toBeLessThan(2);

    // The software keyboard changes visualViewport without a window resize.
    await page.evaluate(() => {
      Object.defineProperties(window.visualViewport!, {
        height: { configurable: true, value: 400 },
        offsetTop: { configurable: true, value: 60 },
      });
      window.visualViewport!.dispatchEvent(new Event("resize"));
    });
    await expect.poll(async () => (await surface.boundingBox())!.height).toBeLessThanOrEqual(360);
    await expect.poll(async () => (await surface.boundingBox())!.y).toBeGreaterThanOrEqual(112);
    await expect.poll(async () => {
      const bounds = (await surface.boundingBox())!;
      return bounds.y + bounds.height;
    }).toBeLessThanOrEqual(461);
    await page.evaluate(() => {
      Reflect.deleteProperty(window.visualViewport!, "height");
      Reflect.deleteProperty(window.visualViewport!, "offsetTop");
      window.visualViewport!.dispatchEvent(new Event("resize"));
    });
    await expect.poll(async () => Math.abs((await surface.boundingBox())!.height - 810)).toBeLessThan(2);

    // Recalculate the whole surface, including toolbar and padding, on resize.
    await page.setViewportSize({ width, height: 400 });
    await expect.poll(async () => (await surface.boundingBox())!.height).toBeLessThanOrEqual(360);
    await expect(handle).toBeInViewport();
    await expect(page.getByRole("button", { name: "Send message", exact: true })).toBeInViewport();
    expect((await handle.boundingBox())!.y).toBeGreaterThanOrEqual(52);
    expect(await input.innerText()).toBe(originalText);
    await input.fill("A long draft remains scrollable.\n".repeat(100));
    await expect(input).toHaveCSS("overflow-y", "scroll");
    await input.evaluate((element) => { element.scrollTop = element.scrollHeight; });
    await expect.poll(() => input.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await handle.dblclick();
    await expect.poll(async () => (await input.boundingBox())!.height).toBeLessThan(120);
  });
}

for (const width of [1280, 320]) {
  test(`exact organization connection picker sends and resets at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await openAssistant(page, { faults: { nyxagentEnabled: true, modelControlsEnabled: true } });
    const sent: string[] = [];
    await page.exposeFunction("capturePolicyRequest", (body: string) => { sent.push(body); });
    await page.evaluate(() => {
      const original = globalThis.__nyxidAssistantHttpMock;
      if (!original) throw new Error("Missing HTTP fixture");
      const capture = (globalThis as typeof globalThis & {
        capturePolicyRequest: (body: string) => Promise<void>;
      }).capturePolicyRequest;
      globalThis.__nyxidAssistantHttpMock = async (request) => {
        if (request.init.method === "POST" && typeof request.init.body === "string") {
          await capture(request.init.body);
        }
        return original(request);
      };
    });
    const trigger = page.getByRole("button", { name: /Model and permissions:/ });
    await trigger.click();
    await page.getByRole("menuitem", { name: /Next message connection/ }).click();
    await page.getByRole("menuitemradio", { name: /ChronoAI.*Shared GPT-4o/ }).click();
    await expect(trigger).toHaveAccessibleName(/Connection: ChronoAI.*Shared GPT-4o/);
    const bounds = (await trigger.boundingBox())!;
    expect(bounds.x).toBeGreaterThanOrEqual(0);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
    await page.screenshot({ path: testInfo.outputPath("connection-selected.png") });
    await composerInput(page).fill("Use the selected organization account");
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await expect.poll(() => sent.some((body) => {
      const request: { text?: string } = JSON.parse(body);
      return request.text === "/service key-0008\nUse the selected organization account";
    })).toBe(true);
    await expect(page).toHaveURL(/c=nyxa-/);
    await expect(trigger).not.toHaveAccessibleName(/Connection:/);
    await expect(page.getByRole("button", { name: "Stop assistant turn" })).toHaveCount(0);
    await composerInput(page).fill("Use the saved order next time");
    await page.getByRole("button", { name: "Send message", exact: true }).click();
    await expect.poll(() => sent.some((body) => {
      const request: { text?: string } = JSON.parse(body);
      return request.text === "Use the saved order next time";
    })).toBe(true);
  });
}
