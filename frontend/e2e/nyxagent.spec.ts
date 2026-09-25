import { test, expect } from "@playwright/test";
import { openAssistant, sendMessage, stopButton, composerInput, conversationRow } from "./helpers";

const RESET_NOTICE =
  "Conversation context was reset; the assistant was given a recap of this chat.";

test("NyxAgent profiles, durable reload, connect link, rename and delete", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const welcome =
    "See your connected services, connect a new one, " +
    "set up a channel bot, or check approvals.";
  await expect(page.getByText(welcome)).toBeVisible();
  await page.getByRole("combobox", { name: "Profile" }).click();
  await page.getByRole("option", { name: "research", exact: true }).click();
  await sendMessage(page, "List connected services");
  await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
  const connect = page.getByRole("link", { name: "Connect GitHub" });
  await expect(connect).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Profile" })).toBeDisabled();
  await page.reload();
  await expect(connect).toHaveAttribute("rel", "noopener noreferrer");
  await conversationRow(page, "List connected services").hover();
  await page.getByRole("button", { name: "Options for List connected services" }).click();
  await page.getByRole("menuitem", { name: "Rename" }).click();
  await expect(page.getByRole("menu", { includeHidden: true })).toHaveCount(0);
  await page.getByRole("dialog").getByRole("textbox").fill("Services overview");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(conversationRow(page, "Services overview")).toBeVisible();
  await conversationRow(page, "Services overview").hover();
  await page.getByRole("button", { name: "Options for Services overview" }).click();
  await page.getByRole("menuitem", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("menu", { includeHidden: true })).toHaveCount(0);
  await page.getByRole("dialog").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(conversationRow(page, "Services overview")).toHaveCount(0);
});

test("Stop and reload retain one inline reset note between the failed reply and next user message", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 4000 } });
  await sendMessage(page, "Inspect my services");
  await expect(page).toHaveURL(/c=nyxa-/);
  await page.reload();
  await expect(stopButton(page)).toBeVisible();
  await stopButton(page).click();
  await expect(stopButton(page)).not.toBeVisible({ timeout: 6000 });
  const partial = page.getByText("Your connected services", { exact: true });
  const note = page.getByRole("note", { name: "Conversation context reset" });
  await expect(partial).toBeVisible();
  await expect(note).toHaveText(RESET_NOTICE);
  await expect(note).toHaveCount(1);
  await expect(composerInput(page)).toBeEnabled();
  await sendMessage(page, "Continue the inspection");
  await expect(note).toHaveCount(1);
  await expect(page.getByRole("link", { name: "Connect GitHub" })).toBeVisible({ timeout: 8000 });
  await page.reload();
  await expect(note).toHaveCount(1);
  // DOM order proves this is a transcript note, rather than a permanent top banner.
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const note = document.querySelector(
          '[role="note"][aria-label="Conversation context reset"]',
        );
        const leaves = [...document.querySelectorAll("p, div")].filter(
          (element) => element.childElementCount === 0,
        );
        const failed = leaves.find((element) => element.textContent === "Your connected services");
        const next = leaves.find((element) => element.textContent === "Continue the inspection");
        return Boolean(
          note &&
          failed &&
          next &&
          failed.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING &&
          note.compareDocumentPosition(next) & Node.DOCUMENT_POSITION_FOLLOWING,
        );
      }),
    )
    .toBe(true);
});

test("existing actor history remains readable with NyxAgent enabled", async ({ page }) => {
  await openAssistant(page, {
    conversation: "conversation-github",
    faults: { nyxagentEnabled: true },
  });
  await expect(composerInput(page)).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Profile" })).toHaveCount(0);
  await page.getByRole("button", { name: "New chat", exact: true }).first().click();
  await expect(page.getByRole("combobox", { name: "Profile" })).toBeVisible();
});

test("images a tool returns show under the reply during the turn and after reload", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true, progressStallMs: 4000 } });
  await sendMessage(page, "Show the lobby camera");
  const image = page.getByRole("img", { name: "Image from lobby-camera__snapshot" });
  // The polled live turn carries the attachment before the reply settles.
  await expect(image).toBeVisible({ timeout: 6000 });
  await expect(page.getByText("Here is the latest lobby snapshot.")).toBeVisible({
    timeout: 8000,
  });
  await expect(image).toHaveJSProperty("naturalWidth", 1);
  await page.reload();
  await expect(page.getByText("Here is the latest lobby snapshot.")).toBeVisible();
  await expect(image).toBeVisible();
  await expect(image).toHaveJSProperty("naturalWidth", 1);
});
