import { expect, test } from "@playwright/test";
import {
  composerInput,
  emptyTurnError,
  openAssistant,
  readEpisode,
  sendButton,
  sendMessage,
  stopButton,
  streamingDots,
  thread,
} from "./helpers";

/**
 * Defect specs — chat-flow audit (docs/chat-flow-audit.md).
 *
 * Convention:
 *  - `test.fail(...)`-annotated specs assert the DESIRED user-visible
 *    behavior and are expected to fail while the defect is open. Fixing the
 *    defect makes Playwright report "passed unexpectedly" — then remove the
 *    annotation and the spec becomes the regression test.
 *  - "current behavior" specs PASS and pin down exactly what a user sees
 *    today, so the defect is reproducible on demand.
 *
 * Fault injection (silent turn, transcript latency) mirrors real transport
 * conditions the deterministic mock script cannot otherwise reach; the
 * mapping to live-transport behavior is argued in the audit report.
 */

test.describe("NYX-1: a turn that never starts has no deadline, no Stop, no error", () => {
  test("current behavior: thinking dots forever over an idle-looking composer", async ({
    page,
  }) => {
    await openAssistant(page, {
      conversation: "conversation-github",
      faults: { sendSilent: true },
    });

    await sendMessage(page, "Rotate the key one more time.");
    await expect(
      thread(page).getByText("Rotate the key one more time."),
    ).toBeVisible({ timeout: 2_000 });

    // The chat says it is thinking...
    await expect(streamingDots(page).first()).toBeVisible({
      timeout: 3_000,
    });
    // ...but the turn was never announced, so there is no Stop to reach for
    // and the composer sits enabled as if nothing were happening.
    await expect(stopButton(page)).toHaveCount(0);
    await expect(composerInput(page)).toBeEnabled();

    // Ten seconds on: still dots, still no error, still no way out.
    await page.waitForTimeout(10_000);
    await expect(streamingDots(page).first()).toBeVisible();
    await expect(emptyTurnError(page)).toHaveCount(0);
    await expect(stopButton(page)).toHaveCount(0);
  });

  test("current behavior: retrying into the hang erases even the thinking dots", async ({
    page,
  }) => {
    await openAssistant(page, {
      conversation: "conversation-github",
      faults: { sendSilent: true },
    });

    await sendMessage(page, "Verify the deploy key status.");
    await expect(streamingDots(page).first()).toBeVisible({
      timeout: 3_000,
    });

    // The composer looks idle, so the reader naturally tries again. The
    // retry is rejected (active-turn guard) — and its cleanup NULLS the
    // live episode (NYX-2), so the thinking dots vanish too. The chat now
    // shows a sent message, no activity, no error, and a composer holding
    // the rejected text: it looks dead.
    await sendMessage(page, "Hello? Are you still there?");
    await expect(
      page.getByText("Message not sent", { exact: false }).first(),
    ).toBeVisible({ timeout: 5_000 });
    await expect(streamingDots(page)).toHaveCount(0, { timeout: 5_000 });
    await expect(emptyTurnError(page)).toHaveCount(0);
    await expect(stopButton(page)).toHaveCount(0);
    // The rejected text was restored — the reader's earlier message hangs
    // unanswered above a composer that claims nothing is running.
    await expect(composerInput(page)).toHaveValue(
      "Hello? Are you still there?",
    );
  });

  test.fail(
    "desired: a stream that never starts surfaces a way out within 10 s",
    async ({ page }) => {
      await openAssistant(page, {
        conversation: "conversation-github",
        faults: { sendSilent: true },
      });

      await sendMessage(page, "Rotate the key one more time.");
      await expect(streamingDots(page).first()).toBeVisible({
        timeout: 3_000,
      });

      // Within 10 s the reader must get EITHER a Stop button (the turn is
      // acknowledged as live) OR an error (the turn is acknowledged as
      // dead). Silence with an enabled composer is neither.
      await expect(
        stopButton(page).or(emptyTurnError(page)).first(),
      ).toBeVisible({ timeout: 10_000 });
    },
  );
});

test.describe("NYX-6: re-sending earlier text suppresses the optimistic echo", () => {
  test("control: novel text echoes instantly even on a slow transcript", async ({
    page,
  }) => {
    await openAssistant(page, {
      conversation: "conversation-github",
      faults: { historyDelayMs: 1_500 },
    });
    await expect(
      thread(page).getByText("Rotate the deploy key for the web repository", {
        exact: false,
      }),
    ).toBeVisible({ timeout: 10_000 });

    await sendMessage(page, "A message nobody has sent before.");
    await expect(
      thread(page).getByText("A message nobody has sent before."),
    ).toBeVisible({ timeout: 1_000 });
  });

  test.fail(
    "desired: re-sending text that appears earlier in the chat still echoes instantly",
    async ({ page }) => {
      await openAssistant(page, {
        conversation: "conversation-github",
        faults: { historyDelayMs: 1_500 },
      });
      // The transcript already contains this exact user message.
      await expect(
        thread(page).getByText(
          "Rotate the deploy key for the web repository and verify access.",
        ),
      ).toBeVisible({ timeout: 10_000 });

      await sendMessage(
        page,
        "Rotate the deploy key for the web repository and verify access.",
      );
      // The textarea cleared on Enter; the echo must appear at once. Today
      // the whole-transcript dedup (pages/assistant.tsx) mistakes the OLD
      // message for the new one's projection and shows nothing for the
      // whole transcript round-trip.
      await expect(
        thread(page)
          .getByText(
            "Rotate the deploy key for the web repository and verify access.",
          )
          .nth(1),
      ).toBeVisible({ timeout: 1_000 });
    },
  );
});

test.describe("NYX-5: deciding an approval opens an episode nothing closes", () => {
  test("current behavior: the card settles but the episode slot stays open forever", async ({
    page,
  }) => {
    await openAssistant(page, { conversation: "conversation-stripe" });
    await expect(
      thread(page).getByText("one write step needs your approval", {
        exact: false,
      }),
    ).toBeVisible();

    await thread(page)
      .getByRole("button", { name: "Approve and send" })
      .click();

    // The decision lands on screen — this part works.
    await expect(
      thread(page).getByText("Approved", { exact: false }).first(),
    ).toBeVisible({ timeout: 5_000 });

    // With the current seeded fixtures the leak is invisible (the thread's
    // tail is an assistant message, which suppresses the thinking row), so
    // pin the state itself: the episode opened by the decision's pump never
    // closes.
    await page.waitForTimeout(2_000);
    const episode = await readEpisode(page, "conversation-stripe");
    expect(episode).toMatchObject({ open: true, printed: false });

    // The chat still works afterwards — the next send replaces the pump.
    await sendMessage(page, "Thanks, confirm it posted.");
    await expect(
      thread(page).getByText("Thanks, confirm it posted."),
    ).toBeVisible({ timeout: 2_000 });
    await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
    await expect(sendButton(page)).toBeVisible();
  });

  test.fail(
    "desired: a settled approval leaves no open episode behind",
    async ({ page }) => {
      await openAssistant(page, { conversation: "conversation-stripe" });
      await thread(page)
        .getByRole("button", { name: "Approve and send" })
        .click();
      await expect(
        thread(page).getByText("Approved", { exact: false }).first(),
      ).toBeVisible({ timeout: 5_000 });

      await page.waitForTimeout(2_000);
      const episode = await readEpisode(page, "conversation-stripe");
      expect(episode === null || episode.open === false).toBe(true);
    },
  );
});
