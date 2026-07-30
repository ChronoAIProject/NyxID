import { expect, test } from "@playwright/test";
import {
  composerInput,
  conversationRow,
  openAssistant,
  SCRIPTED_REPLY_START,
  SEEDED,
  sendMessage,
  thread,
} from "./helpers";

/**
 * Flow 3 — New chat / old chat.
 *
 * "New chat" is navigation only: it allocates nothing, so the sidebar must
 * not grow and no conversation id may appear until the first send. That send
 * must paint the reader's message and the thinking state IMMEDIATELY (the
 * composer clears the textarea before awaiting — any gap reads as the app
 * dying), allocate lazily, and then behave like any other conversation.
 * Switching to an old chat and back must keep both transcripts intact.
 */

test("New chat allocates nothing until the first send, which paints instantly", async ({
  page,
}) => {
  await openAssistant(page);
  const sidebarRows = page
    .getByRole("navigation")
    .getByRole("button", { name: "Options for" });
  await expect(sidebarRows).toHaveCount(3);

  await page.getByRole("button", { name: "New chat" }).click();

  // Draft state: URL says draft, screen shows the empty state, and the
  // sidebar has NOT gained a row — nothing was provisioned.
  await expect(page).toHaveURL(/draft=true/);
  await expect(
    thread(page).getByText("Start a new conversation"),
  ).toBeVisible();
  await expect(sidebarRows).toHaveCount(3);

  await sendMessage(page, "Plan my week with my connected calendar.");

  // The reader's message appears immediately — the textarea already
  // cleared, so any blank gap here looks like data loss.
  await expect(
    thread(page).getByText("Plan my week with my connected calendar."),
  ).toBeVisible({ timeout: 1_000 });
  await expect(composerInput(page)).toHaveValue("");

  // Lazy allocation: the URL now addresses the created conversation.
  await expect(page).toHaveURL(/c=local-/, { timeout: 5_000 });

  // The turn streams and completes like any other conversation.
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });

  // The sidebar row exists now, titled from the first message (40-char cap).
  await expect(sidebarRows).toHaveCount(4);
  await expect(
    conversationRow(page, "Plan my week with my connected calendar."),
  ).toBeVisible();
});

test("switching from the new chat to an old one and back keeps both intact", async ({
  page,
}) => {
  await openAssistant(page);
  await page.getByRole("button", { name: "New chat" }).click();
  await expect(
    thread(page).getByText("Start a new conversation"),
  ).toBeVisible();

  await sendMessage(page, "Summarize my pending approvals.");
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });

  // Old chat: its own transcript, nothing from the new one.
  await conversationRow(page, SEEDED.weekly.title).click();
  await expect(
    thread(page).getByText("Prepare last week's NyxID usage report.", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(
    thread(page).getByText("Summarize my pending approvals."),
  ).toBeHidden();

  // Back to the new chat: transcript intact.
  await conversationRow(page, "Summarize my pending approvals.").click();
  await expect(
    thread(page).getByText("Summarize my pending approvals."),
  ).toBeVisible();
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible();
  await expect(
    thread(page).getByText("Prepare last week's NyxID usage report.", {
      exact: false,
    }),
  ).toBeHidden();
});
