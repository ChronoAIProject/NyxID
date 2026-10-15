import { expect, test } from "@playwright/test";
import { composerInput, conversationRow, openAssistant, SEEDED } from "./helpers";

async function attach(page: import("@playwright/test").Page) {
  await page
    .getByLabel("Choose attachments")
    .setInputFiles({
      name: "Design notes.md",
      mimeType: "text/markdown",
      buffer: Buffer.from("# Design notes\nKeep it compact."),
    });
  await expect(
    page
      .getByRole("list", { name: "Attachments", exact: true })
      .getByRole("status"),
  ).toHaveText("Ready");
}

test("attachment pills support selection, atomic deletion, undo, copy and send", async ({
  page,
  context,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await attach(page);
  const input = composerInput(page);
  await input.fill("Review @fi");
  await page.getByRole("option", { name: /file-1/ }).click();
  const pill = input.locator('[data-mention-handle="file-1"]');
  await expect(pill).toHaveText("file-1");
  await input.press("Backspace"); // trailing space
  await input.press("Backspace"); // one whole reference
  await expect(pill).toHaveCount(0);
  await expect(input).toHaveText("Review ");
  await input.press("ControlOrMeta+z");
  await expect(pill).toBeVisible();
  await input.press("ControlOrMeta+Shift+z");
  await expect(pill).toHaveCount(0);
  await input.press("ControlOrMeta+z");
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await input.press("ControlOrMeta+a");
  await input.press("ControlOrMeta+c");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toContain(
    "Review @file-1",
  );
  await input.press("ArrowRight");
  await input.press("Enter");
  await expect(page.getByRole("main")).toContainText(
    'Review file "Design notes.md"',
  );
});

test("pasted text becomes a pill, and removing its upload clears undo references", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const input = composerInput(page);
  await input.evaluate((element) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", "A long research note.\n".repeat(10));
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
  await expect(
    input.locator('[data-composer-mention="pasted-text"]'),
  ).toHaveText("text-1");
  await page.getByRole("button", { name: "Remove pasted-text-1.txt" }).click();
  await expect(input.locator("[data-composer-mention]")).toHaveCount(0);
  await input.press("ControlOrMeta+z");
  await expect(input).not.toContainText("text-1");
  await expect(
    page.getByRole("button", { name: "Send message", exact: true }),
  ).toBeDisabled();
});

test("plain paste preserves line breaks and ignores HTML formatting", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const input = composerInput(page);
  await input.focus();
  await input.evaluate((element) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", "First line\nSecond line");
    clipboardData.setData(
      "text/html",
      '<h1>Different text</h1><img src="bad" onerror="alert(1)">',
    );
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        clipboardData,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
  await expect(input).toHaveText("First lineSecond line");
  await expect(input.locator("br")).toHaveCount(1);
  await expect(input.locator("h1,img")).toHaveCount(0);
  await input.press("Shift+Enter");
  await input.pressSequentially("Third line");
  await input.press("Enter");
  await expect(
    page
      .getByText("First line\nSecond line\nThird line", { exact: true })
      .first(),
  ).toBeVisible();
});


test("draft restoration keeps undo history isolated between chats", async ({ page }) => {
  await openAssistant(page);
  await conversationRow(page, SEEDED.stripe.title).click();
  const input = composerInput(page);
  await input.fill("Draft for Stripe");
  await conversationRow(page, SEEDED.github.title).click();
  await expect(input).toHaveText("");
  await input.fill("Draft for GitHub");
  await input.press("ControlOrMeta+z");
  await expect(input).toHaveText("");
  await conversationRow(page, SEEDED.stripe.title).click();
  await expect(input).toHaveText("Draft for Stripe");
  await input.press("ControlOrMeta+z");
  await expect(input).toHaveText("Draft for Stripe");
});

test("Enter during composition does not submit an unfinished message", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const input = composerInput(page);
  await input.fill("你好");
  await input.evaluate((element) => {
    element.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    element.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true }));
  });
  await expect(page).not.toHaveURL(/c=nyxa-/);
  await expect(input).toHaveText("你好");
  await input.evaluate((element) => element.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true, data: "你好" })));
  await page.getByRole("button", { name: "Send message", exact: true }).click();
  await expect(page).toHaveURL(/c=nyxa-/);
  await expect(page.getByRole("main")).toContainText("你好");
});
