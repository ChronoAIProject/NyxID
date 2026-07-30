import { expect, test } from "@playwright/test";
import {
  assistantHalo,
  composerInput,
  conversationRow,
  openAssistant,
  SCRIPTED_REPLY_START,
  SCRIPTED_TOOL_RESULT,
  SEEDED,
  sendMessage,
  stopButton,
  streamingDots,
  thread,
} from "./helpers";

/**
 * Flow 4 — Conversation switching.
 *
 * Switching away mid-stream must show the other conversation clean (no
 * leaked text, no leaked loading state, usable composer); switching back
 * must land in the live turn (or its completed result). Switching while a
 * send is in flight must keep the optimistic echo out of the other thread.
 */

test("switching away mid-stream leaks nothing; switching back lands in the finished turn", async ({
  page,
}) => {
  await openAssistant(page, { conversation: "conversation-stripe" });
  await expect(
    thread(page).getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toBeVisible();

  await sendMessage(page, "Check the payment retries as well.");
  // The turn is live in stripe: halo up, Stop offered.
  await expect(assistantHalo(page).first()).toBeVisible({ timeout: 2_000 });

  await conversationRow(page, SEEDED.github.title).click();

  // The other conversation is clean: its transcript, no leaked message,
  // no leaked loading state, composer immediately usable.
  await expect(
    thread(page).getByText("Rotate the deploy key for the web repository", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(
    thread(page).getByText("Check the payment retries as well."),
  ).toHaveCount(0);
  await expect(streamingDots(page)).toHaveCount(0);
  await expect(stopButton(page)).toHaveCount(0);
  await expect(composerInput(page)).toBeEnabled();

  // The background turn keeps running to completion; switching back shows
  // the whole exchange, with no error row.
  await conversationRow(page, SEEDED.stripe.title).click();
  await expect(
    thread(page).getByText("Check the payment retries as well."),
  ).toBeVisible();
  await expect(
    thread(page).getByText(SCRIPTED_TOOL_RESULT, { exact: false }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
  await expect(page.locator("[data-empty-turn-error]")).toHaveCount(0);
});

test("switching back WHILE the turn still streams resumes its live loading state", async ({
  page,
}) => {
  await openAssistant(page, { conversation: "conversation-stripe" });
  await expect(
    thread(page).getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toBeVisible();

  await sendMessage(page, "Re-check the two disputed charges.");
  await expect(assistantHalo(page).first()).toBeVisible({ timeout: 2_000 });

  // Away and straight back, inside the ~1.2 s scripted stream.
  await conversationRow(page, SEEDED.github.title).click();
  await expect(
    thread(page).getByText("Rotate the deploy key for the web repository", {
      exact: false,
    }),
  ).toBeVisible();
  await conversationRow(page, SEEDED.stripe.title).click();

  // Back in the live (or just-finished) turn: the reader's message is
  // there and the reply completes on screen.
  await expect(
    thread(page).getByText("Re-check the two disputed charges."),
  ).toBeVisible();
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
});

test("an in-flight send's optimistic echo never follows the reader into another chat", async ({
  page,
}) => {
  // Stretch the send round-trip so the pending window is observable.
  await openAssistant(page, {
    conversation: "conversation-stripe",
    faults: { historyDelayMs: 1_200 },
  });
  await expect(
    thread(page).getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toBeVisible({ timeout: 10_000 });

  await sendMessage(page, "File the dispute paperwork for me.");
  // Echo paints in the conversation it was sent to...
  await expect(
    thread(page).getByText("File the dispute paperwork for me."),
  ).toBeVisible({ timeout: 1_000 });

  // ...and stays out of the one the reader switches to mid-send.
  await conversationRow(page, SEEDED.github.title).click();
  await expect(
    thread(page).getByText("Rotate the deploy key for the web repository", {
      exact: false,
    }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(
    thread(page).getByText("File the dispute paperwork for me."),
  ).toHaveCount(0);

  // Back home: the message belongs here and the turn finishes.
  await conversationRow(page, SEEDED.stripe.title).click();
  await expect(
    thread(page).getByText("File the dispute paperwork for me."),
  ).toBeVisible({ timeout: 10_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 15_000 });
});
