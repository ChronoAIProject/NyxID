import { expect, test } from "@playwright/test";
import { openAssistant, composerInput } from "./helpers";
import { installVoiceFixture } from "./voice-fixture";
const faults = {
  nyxagentEnabled: true,
  voiceEnabled: true,
  modelControlsEnabled: true,
};
async function setup(page: import("@playwright/test").Page) {
  await page.getByRole("button", { name: "Configure dictation" }).click();
  await expect(
    page.getByRole("button", { name: "Start dictation" }),
  ).toBeEnabled();
  await expect(
    page.getByText(
      /0.01 credits per input audio second \+ 0.02 credits per start/,
    ),
  ).toBeVisible();
}
for (const width of [320, 390, 1280])
  test(`all composer controls fit at ${width}px with a usable editor`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await installVoiceFixture(page);
    await openAssistant(page, { faults });
    await composerInput(page).fill("A draft");
    const composer = await page.locator("[data-composer-input]").boundingBox();
    expect(composer).not.toBeNull();
    const controls = await page
      .locator("[data-composer-input] button:visible")
      .all();
    const boxes = [];
    for (const control of controls) {
      const b = (await control.boundingBox())!;
      expect(b.x).toBeGreaterThanOrEqual(composer!.x);
      expect(b.x + b.width).toBeLessThanOrEqual(
        Math.min(width, composer!.x + composer!.width),
      );
      boxes.push(b);
    }
    for (let a = 0; a < boxes.length; a++)
      for (let b = a + 1; b < boxes.length; b++) {
        const x = boxes[a]!,
          y = boxes[b]!;
        expect(
          x.x + x.width <= y.x + 0.5 ||
            y.x + y.width <= x.x + 0.5 ||
            x.y + x.height <= y.y + 0.5 ||
            y.y + y.height <= x.y + 0.5,
        ).toBe(true);
      }
    expect((await composerInput(page).boundingBox())!.width).toBeGreaterThan(
      120,
    );
  });
test("dictation preserves rich selected draft and attachments; confirm focuses editor and one undo restores", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page);
  await openAssistant(page, { faults });
  await page
    .getByLabel("Choose attachments")
    .setInputFiles({
      name: "notes.md",
      mimeType: "text/markdown",
      buffer: Buffer.from("notes"),
    });
  await expect(
    page
      .getByRole("list", { name: "Attachments", exact: true })
      .getByRole("status"),
  ).toHaveText("Ready");
  const input = composerInput(page);
  await input.fill("Review @fi");
  await page.getByRole("option", { name: /file-1/ }).click();
  await input.press("End");
  await input.press("ControlOrMeta+ArrowRight");
  await input.press("End");
  await page.keyboard.type("old world");
  await input.evaluate((element) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    let node: Node | null;
    while ((node = walker.nextNode())) {
      const i = node.textContent?.indexOf("old") ?? -1;
      if (i >= 0) {
        const range = document.createRange();
        range.setStart(node, i);
        range.setEnd(node, i + 3);
        const s = window.getSelection()!;
        s.removeAllRanges();
        s.addRange(range);
        element.dispatchEvent(new Event("selectionchange", { bubbles: true }));
        break;
      }
    }
  });
  await setup(page);
  await page.getByRole("button", { name: "Start dictation" }).click();
  await expect(page.getByLabel("Dictation review")).toContainText(
    "first second interim",
  );
  await page.getByRole("button", { name: "Stop dictation for review" }).click();
  await page.getByRole("button", { name: "Insert dictation" }).click();
  await expect(input).toBeFocused();
  await expect(input).toContainText("first second world");
  await expect(input.locator('[data-mention-handle="file-1"]')).toBeVisible();
  await expect(
    page.getByRole("list", { name: "Attachments", exact: true }),
  ).toBeVisible();
  await input.press("ControlOrMeta+z");
  await expect(input).toContainText("old world");
  expect(calls.dictations).toBe(1);
  expect(calls.starts).toBe(0);
});
test("permission cancellation fences late grants and escape restores selection", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page, "pending");
  await openAssistant(page, { faults });
  const input = composerInput(page);
  await input.fill("Keep this draft");
  await input.press("ControlOrMeta+a");
  await setup(page);
  await page.getByRole("button", { name: "Start dictation" }).click();
  await expect(
    page.getByText("Allow microphone access in your browser…"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(input).toBeFocused();
  await expect(input).toHaveText("Keep this draft");
  await page.evaluate(() => {
    (
      window as unknown as { __composerVoiceFixture: { grant: () => void } }
    ).__composerVoiceFixture.grant();
  });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __composerVoiceFixture: { stopped: number } })
            .__composerVoiceFixture.stopped,
      ),
    )
    .toBe(1);
  expect(calls.dictations).toBe(0);
});
test("integrated active voice uses real fixture levels, keeps one session and provider fragment delivery", async ({
  page,
}) => {
  const calls = await installVoiceFixture(page);
  await openAssistant(page, { faults });
  await page.getByRole("button", { name: "Open voice call" }).click();
  await page.getByLabel("Microphone mode").selectOption("automatic");
  await page.getByRole("button", { name: "Start voice" }).click();
  await expect(
    page.getByRole("button", { name: "End call", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("A real event-shaped live caption"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Open voice call" }).click();
  expect(calls.starts).toBe(1);
  expect(calls.ends).toBe(0);
  await expect
    .poll(() =>
      page
        .locator(".composer-audio-surface")
        .evaluate((element) =>
          Number(
            getComputedStyle(element).getPropertyValue(
              "--composer-audio-level",
            ),
          ),
        ),
    )
    .toBeGreaterThan(0);
  expect(
    await page
      .locator(".composer-audio-surface")
      .evaluate(
        (element) => getComputedStyle(element, "::before").backgroundImage,
      ),
  ).not.toBe("none");
  await expect(page.locator('[data-voice-delivery="played"]')).toContainText(
    "Played.",
  );
  await expect(page.locator('[data-voice-delivery="pending"]')).toContainText(
    "Pending.",
  );
  await page.getByRole("button", { name: "End call", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Open voice call" }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect.poll(() => calls.ends).toBe(1);
});
