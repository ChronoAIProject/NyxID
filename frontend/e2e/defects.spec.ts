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
 * Regression specs from the chat-flow audit (docs/chat-flow-audit.md).
 *
 * Fault injection (silent turn, transcript latency) mirrors real transport
 * conditions the deterministic mock script cannot otherwise reach; the
 * mapping to live-transport behavior is argued in the audit report.
 */

test.describe("explicit conversation repair", () => {
  test("a confirmed missing deep link repairs to New chat", async ({
    page,
  }) => {
    await openAssistant(page, {
      conversation: "conversation-does-not-exist",
      faults: { historyErrorStatus: 404 },
    });

    await expect(
      page.locator("header").getByText("New chat", { exact: true }),
    ).toBeVisible({ timeout: 5_000 });
    await expect(
      thread(page).getByText("Start a new conversation"),
    ).toBeVisible();
    expect(new URL(page.url()).searchParams.has("c")).toBe(false);
  });

  test("a transient transcript failure retains the explicit address", async ({
    page,
  }) => {
    const conversationId = "conversation-temporarily-unavailable";
    await openAssistant(page, {
      conversation: conversationId,
      faults: { historyErrorStatus: 500 },
    });

    await expect(
      page
        .getByRole("status")
        .filter({ hasText: "Could not load earlier messages" }),
    ).toBeVisible({ timeout: 20_000 });
    expect(new URL(page.url()).searchParams.get("c")).toBe(conversationId);
    await expect(composerInput(page)).toBeEnabled();
  });
});

test.describe("NYX-1: a turn that never starts reaches a deadline", () => {
  test("a silent turn reports an error and frees the composer", async ({
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

    await expect(streamingDots(page).first()).toBeVisible({
      timeout: 3_000,
    });
    await expect(emptyTurnError(page)).toBeVisible({ timeout: 10_000 });
    await expect(streamingDots(page)).toHaveCount(0);
    await expect(stopButton(page)).toHaveCount(0);
    await expect(composerInput(page)).toBeEnabled();
  });

  test("a rejected retry cannot erase the live episode", async ({ page }) => {
    await openAssistant(page, {
      conversation: "conversation-github",
      faults: { sendSilent: true },
    });

    await sendMessage(page, "Verify the deploy key status.");
    await expect(streamingDots(page).first()).toBeVisible({
      timeout: 3_000,
    });

    await sendMessage(page, "Hello? Are you still there?");
    await expect(
      page.getByText("Message not sent", { exact: false }).first(),
    ).toBeVisible({ timeout: 5_000 });
    await expect(streamingDots(page).first()).toBeVisible({ timeout: 5_000 });
    await expect(emptyTurnError(page)).toHaveCount(0);
    await expect(stopButton(page)).toHaveCount(0);
    // The rejected text is restored so the reader can retry it once the
    // hanging turn reaches its deadline, rather than losing what they typed.
    await expect(composerInput(page)).toHaveValue(
      "Hello? Are you still there?",
    );
  });

  test("desired: a stream that never starts surfaces a way out within 10 s", async ({
    page,
  }) => {
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
    await expect(stopButton(page).or(emptyTurnError(page)).first()).toBeVisible(
      { timeout: 10_000 },
    );
  });
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

  test("desired: re-sending text that appears earlier in the chat still echoes instantly", async ({
    page,
  }) => {
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
    // The textarea cleared on Enter; the second copy must appear at once,
    // before the deliberately slow transcript projection lands.
    await expect(
      thread(page)
        .getByText(
          "Rotate the deploy key for the web repository and verify access.",
        )
        .nth(1),
    ).toBeVisible({ timeout: 1_000 });
  });
});

test.describe("NYX-5: approval episode cleanup", () => {
  test("the card settles without leaving an open episode", async ({ page }) => {
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

    await page.waitForTimeout(2_000);
    const episode = await readEpisode(page, "conversation-stripe");
    expect(episode === null || episode.open === false).toBe(true);

    // The chat still works afterwards — the next send replaces the pump.
    await sendMessage(page, "Thanks, confirm it posted.");
    await expect(
      thread(page).getByText("Thanks, confirm it posted."),
    ).toBeVisible({ timeout: 2_000 });
    await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
    await expect(sendButton(page)).toBeVisible();
  });

  test("desired: a settled approval leaves no open episode behind", async ({
    page,
  }) => {
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
  });
});
