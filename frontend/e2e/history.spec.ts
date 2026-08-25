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
 * Bare assistant is always a fresh draft, even when history exists. Explicit
 * selection swaps transcripts; a slow transcript shows a loading state; a
 * failed/absent transcript is reported above a still-usable thread.
 */

test("opening the assistant lists history but leaves a bare route on New chat", async ({
  page,
}) => {
  await openAssistant(page);

  for (const conversation of Object.values(SEEDED)) {
    await expect(conversationRow(page, conversation.title)).toBeVisible();
  }

  expect(new URL(page.url()).searchParams.has("c")).toBe(false);
  await expect(
    page.locator("header").getByText("New chat", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Ask NyxID to help with services, access, and account operations.",
    ),
  ).toBeVisible();
  await expect(
    page.getByText("Pull yesterday's failed Stripe payments", {
      exact: false,
    }),
  ).toHaveCount(0);
  for (const conversation of Object.values(SEEDED)) {
    await expect(conversationRow(page, conversation.title)).toHaveCSS(
      "font-weight",
      "400",
    );
  }
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
  await openAssistant(page, {
    conversation: SEEDED.stripe.id,
    faults: { historyDelayMs: 1500 },
  });

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
  await openAssistant(page, {
    conversation: SEEDED.stripe.id,
    faults: { historyErrorStatus: 404 },
  });

  const notice = page
    .getByRole("status")
    .filter({ hasText: "no saved transcript" });
  await expect(notice).toBeVisible({ timeout: 1_000 });
  await expect(notice).toContainText("You can keep chatting");
  await expect(composerInput(page)).toBeEnabled();
});

test("a failing transcript read shows the error notice, not a dead screen", async ({
  page,
}) => {
  await openAssistant(page, {
    conversation: SEEDED.stripe.id,
    faults: { historyErrorStatus: 500 },
  });

  await expect(
    page
      .getByRole("status")
      .filter({ hasText: "Could not load earlier messages" }),
  ).toBeVisible({ timeout: 20_000 });
  await expect(composerInput(page)).toBeEnabled();
});
