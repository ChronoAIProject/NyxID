import { expect, test } from "@playwright/test";
import {
  composerInput,
  conversationRow,
  openAssistant,
  SEEDED,
} from "./helpers";

/**
 * Flow 1 — Getting history.
 *
 * Opening the assistant must land the reader in a real conversation with its
 * transcript on screen; switching must swap transcripts; a slow transcript
 * shows a loading state; a failed/absent transcript is REPORTED above a
 * still-usable thread, never a dead screen.
 */

test("opening the assistant lists all chats and shows the newest transcript", async ({
  page,
}) => {
  await openAssistant(page);

  for (const conversation of Object.values(SEEDED)) {
    await expect(conversationRow(page, conversation.title)).toBeVisible();
  }

  // Newest conversation auto-selected; its transcript is on screen.
  await expect(page).toHaveURL(/c=conversation-stripe/);
  await expect(
    page.getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(
    page.getByText("23 failed payments", { exact: false }).first(),
  ).toBeVisible();

  // The composer is ready — no active turn in a loaded conversation.
  await expect(composerInput(page)).toBeEnabled();
});

test("selecting another chat swaps in that transcript and nothing leaks", async ({
  page,
}) => {
  await openAssistant(page);
  await conversationRow(page, SEEDED.github.title).click();

  await expect(page).toHaveURL(/c=conversation-github/);
  await expect(
    page.getByText("Rotate the deploy key for the web repository", {
      exact: false,
    }),
  ).toBeVisible();
  // The previous transcript is gone.
  await expect(
    page.getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toBeHidden();
});

test("a slow transcript read shows a loading state, then the conversation", async ({
  page,
}) => {
  await openAssistant(page, { faults: { historyDelayMs: 1500 } });

  await expect(page.getByText("Loading conversation...")).toBeVisible();
  await expect(
    page.getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText("Loading conversation...")).toBeHidden();
});

test("a 404 transcript shows the no-transcript-yet notice above a usable composer", async ({
  page,
}) => {
  await openAssistant(page, { faults: { historyErrorStatus: 404 } });

  // The query retries 3 times before surfacing; allow for the backoff.
  const notice = page
    .getByRole("status")
    .filter({ hasText: "no saved transcript" });
  await expect(notice).toBeVisible({ timeout: 20_000 });
  await expect(notice).toContainText("You can keep chatting");
  await expect(composerInput(page)).toBeEnabled();
});

test("a failing transcript read shows the error notice, not a dead screen", async ({
  page,
}) => {
  await openAssistant(page, { faults: { historyErrorStatus: 500 } });

  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "Could not load earlier messages" }),
  ).toBeVisible({ timeout: 20_000 });
  await expect(composerInput(page)).toBeEnabled();
});
