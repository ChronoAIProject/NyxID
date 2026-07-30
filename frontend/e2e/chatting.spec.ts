import { expect, test } from "@playwright/test";
import {
  assistantHalo,
  composerInput,
  emptyTurnError,
  openAssistant,
  SCRIPTED_REPLY_START,
  SCRIPTED_TOOL_RESULT,
  sendMessage,
  stopButton,
  streamingDots,
} from "./helpers";

/**
 * Flow 2 — Chatting.
 *
 * The contract, in order: the sent message appears immediately; a thinking
 * state (halo + bouncing dots) holds the floor until the first streamed
 * content; the dots are REPLACED by the answer; the turn completes with the
 * full answer and its tool ledger on screen; the red empty-turn error never
 * appears on a turn that answered.
 */

test("a sent message echoes immediately and the reply streams to completion", async ({
  page,
}) => {
  await openAssistant(page, { conversation: "conversation-github" });

  await sendMessage(page, "Verify the new deploy key once more.");

  // While the turn runs: composer locked behind Stop, halo on. (Asserted
  // first — the scripted turn only lives ~1.2 s, so the live-state checks
  // must not queue behind slower ones.)
  await expect(stopButton(page)).toBeVisible({ timeout: 1_000 });
  await expect(composerInput(page)).toBeDisabled();
  await expect(assistantHalo(page).first()).toBeVisible();

  // The reader's message is on screen at once — not after the round-trip.
  await expect(
    page.getByText("Verify the new deploy key once more."),
  ).toBeVisible({ timeout: 1_000 });

  // The streamed answer arrives and completes, tool ledger included.
  await expect(
    page.getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(
    page.getByText(SCRIPTED_TOOL_RESULT, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });

  // Turn over: dots gone, composer usable again, halo fades out.
  await expect(streamingDots(page)).toHaveCount(0, { timeout: 5_000 });
  await expect(composerInput(page)).toBeEnabled();
  await expect(assistantHalo(page)).toHaveCount(0, { timeout: 5_000 });

  // A turn that answered is never called an error.
  await expect(emptyTurnError(page)).toHaveCount(0);
});

test("thinking dots hold the floor before the first content, then are replaced by it", async ({
  page,
}) => {
  // Delay transcript projection so the pre-content window is long enough to
  // assert deterministically (the scripted turn otherwise prints within
  // ~300 ms, faster than assertion polling can reliably observe — and under
  // parallel-worker load the whole window can pass before the first poll).
  await openAssistant(page, {
    conversation: "conversation-github",
    faults: { historyDelayMs: 2_500 },
  });
  await expect(
    page.getByText("Rotate the deploy key for the web repository", {
      exact: false,
    }),
  ).toBeVisible({ timeout: 10_000 });

  await sendMessage(page, "Run the access verification again.");

  // Pre-content: dots present and announced to assistive tech.
  await expect(streamingDots(page).first()).toBeVisible({ timeout: 5_000 });
  await expect(
    page.getByRole("status", { name: /Assistant is (thinking|answering)/ }),
  ).toBeVisible();

  // The dots are replaced by streamed text — never both, never neither, and
  // no error in between.
  await expect(
    page.getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 10_000 });
  await expect(streamingDots(page)).toHaveCount(0, { timeout: 5_000 });
  await expect(emptyTurnError(page)).toHaveCount(0);
});

test("Stop ends the turn quietly: no red error for a reader-cancelled turn", async ({
  page,
}) => {
  await openAssistant(page, { conversation: "conversation-github" });

  await sendMessage(page, "Draft a long rotation postmortem.");
  await expect(stopButton(page)).toBeVisible({ timeout: 2_000 });
  await stopButton(page).click();

  // The composer hands back control...
  await expect(composerInput(page)).toBeEnabled({ timeout: 5_000 });
  await expect(stopButton(page)).toHaveCount(0);
  // ...and pressing Stop is the reader's decision, not an error.
  await expect(emptyTurnError(page)).toHaveCount(0);
  await expect(streamingDots(page)).toHaveCount(0, { timeout: 5_000 });

  // The chat is still alive: a follow-up send works.
  await sendMessage(page, "Short version instead, please.");
  await expect(
    page.getByText("Short version instead, please."),
  ).toBeVisible({ timeout: 2_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
});
