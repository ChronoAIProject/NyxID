import { expect, test } from "@playwright/test";
import {
  composerInput,
  conversationRow,
  emptyTurnError,
  observeTurnContinuity,
  openAssistant,
  readTurnContinuity,
  SCRIPTED_REPLY_START,
  SEEDED,
  sendMessage,
  stopButton,
  thread,
} from "./helpers";

/**
 * A draft allocates nothing. Its first send keeps the placeholder address for
 * the whole live turn, then settles onto the canonical conversation without
 * losing the optimistic message, loading state, transcript, or sidebar row.
 */

test("a first send stays continuous through the canonical id transition", async ({
  page,
}) => {
  const message = "Plan my week with my connected calendar.";
  await openAssistant(page, { faults: { aliasOnFirstSend: true } });
  const sidebarRows = page
    .getByRole("navigation")
    .getByRole("button", { name: "Options for" });

  await expect(
    page.locator("header").getByText("New chat", { exact: true }),
  ).toBeVisible();
  await expect(
    thread(page).getByText("Start a new conversation"),
  ).toBeVisible();
  expect(new URL(page.url()).searchParams.has("c")).toBe(false);
  await expect(sidebarRows).toHaveCount(3);
  for (const conversation of Object.values(SEEDED)) {
    await expect(conversationRow(page, conversation.title)).toHaveCSS(
      "font-weight",
      "400",
    );
  }

  await observeTurnContinuity(page, message);
  await sendMessage(page, message);

  const echoedMessage = thread(page).getByText(message, { exact: true });
  await expect(echoedMessage).toHaveCount(1, { timeout: 1_000 });
  await expect(composerInput(page)).toHaveValue("");
  await expect(page).toHaveURL(/c=local-pending-/, { timeout: 2_000 });
  await expect(stopButton(page)).toBeVisible({ timeout: 1_000 });

  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 5_000 });
  await expect(echoedMessage).toHaveCount(1);
  await expect(sidebarRows).toHaveCount(4);

  const newRow = conversationRow(page, message);
  await expect(newRow).toHaveCount(1);
  await expect(newRow).toHaveCSS("font-weight", "500");
  await expect(emptyTurnError(page)).toHaveCount(0);

  // Leave enough time for query invalidation, loader exit, and any delayed
  // normalization to expose a regression that briefly returns to the draft.
  await page.waitForTimeout(2_500);
  await expect(echoedMessage).toHaveCount(1);
  await expect(thread(page).getByText("Start a new conversation")).toHaveCount(
    0,
  );
  expect(await readTurnContinuity(page)).toEqual({
    sawMessage: true,
    sawLoading: true,
    bouncedToEmptyState: false,
    loadingGapBeforeReply: false,
  });
});

test("switching away from a canonicalized new chat and back keeps both transcripts", async ({
  page,
}) => {
  const message = "Summarize my pending approvals.";
  await openAssistant(page, { faults: { aliasOnFirstSend: true } });

  await sendMessage(page, message);
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 5_000 });

  await conversationRow(page, SEEDED.weekly.title).click();
  await expect(
    thread(page).getByText("Prepare last week's NyxID usage report.", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(thread(page).getByText(message, { exact: true })).toHaveCount(0);

  await conversationRow(page, message).click();
  await expect(thread(page).getByText(message, { exact: true })).toHaveCount(1);
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible();
  await expect(
    thread(page).getByText("Prepare last week's NyxID usage report.", {
      exact: false,
    }),
  ).toHaveCount(0);
});
