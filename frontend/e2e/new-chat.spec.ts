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
  thread,
} from "./helpers";

/**
 * A draft allocates nothing. Its first send stays unaddressed until
 * RUN_STARTED supplies the canonical actor id, without losing the optimistic
 * message, loading state, transcript, or sidebar row.
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
    thread(page).getByText(
      "Ask NyxID to help with services, access, and account operations.",
    ),
  ).toBeVisible();
  expect(new URL(page.url()).searchParams.has("c")).toBe(false);
  await expect(sidebarRows).toHaveCount(4);
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
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 2_000 });

  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
  await expect(echoedMessage).toHaveCount(1);
  await expect(sidebarRows).toHaveCount(5);

  const newRow = conversationRow(page, message);
  await expect(newRow).toHaveCount(1);
  await expect(newRow).toHaveCSS("font-weight", "500");
  await expect(emptyTurnError(page)).toHaveCount(0);

  // Leave enough time for query invalidation, loader exit, and any delayed
  // normalization to expose a regression that briefly returns to the draft.
  await page.waitForTimeout(2_500);
  await expect(echoedMessage).toHaveCount(1);
  await expect(
    thread(page).getByText(
      "Ask NyxID to help with services, access, and account operations.",
    ),
  ).toHaveCount(0);
  expect(await readTurnContinuity(page)).toEqual({
    sawMessage: true,
    sawLoading: true,
    sawReply: true,
    bouncedToEmptyState: false,
    loadingGapBeforeReply: false,
    loadingAfterReply: false,
    emptyAfterReply: false,
  });
});

test("a delayed canonical history read never replaces a finished transcript", async ({
  page,
}) => {
  const message = "Keep the finished answer visible through the id swap.";
  await openAssistant(page, {
    faults: { aliasOnFirstSend: true, historyDelayMs: 400 },
  });
  await observeTurnContinuity(page, message);

  await sendMessage(page, message);
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 3_000 });
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible({ timeout: 8_000 });
  await expect(composerInput(page)).toBeEnabled({ timeout: 10_000 });
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 5_000 });

  await page.waitForTimeout(1_000);
  await expect(
    thread(page).getByText(SCRIPTED_REPLY_START, { exact: false }),
  ).toBeVisible();
  const continuity = await readTurnContinuity(page);
  expect(continuity.sawReply).toBe(true);
  expect(continuity.loadingAfterReply).toBe(false);
  expect(continuity.emptyAfterReply).toBe(false);
});

test("reloading a fixture-only canonical id repairs naturally to New chat", async ({
  page,
}) => {
  const message = "Reload this pending placeholder.";
  await openAssistant(page, { faults: { aliasOnFirstSend: true } });
  await sendMessage(page, message);
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 2_000 });

  const reloadUrl = new URL(page.url());
  reloadUrl.searchParams.set("mock", "1");
  await page.evaluate((url) => {
    window.history.replaceState(window.history.state, "", url);
  }, reloadUrl.toString());
  await page.reload();

  await expect(
    page.locator("header").getByText("New chat", { exact: true }),
  ).toBeVisible({ timeout: 5_000 });
  await expect(
    thread(page).getByText(
      "Ask NyxID to help with services, access, and account operations.",
    ),
  ).toBeVisible();
  expect(new URL(page.url()).searchParams.has("c")).toBe(false);
  await page.waitForTimeout(1_000);
  await expect(
    page.getByText("Could not load earlier messages", { exact: false }),
  ).toHaveCount(0);
  await expect(
    page.getByText("no saved transcript", { exact: false }),
  ).toHaveCount(0);
  await expect(composerInput(page)).toBeEnabled();
});

test("deleting a canonical conversation drops c", async ({
  page,
}) => {
  const message = "Delete this pending alias.";
  await openAssistant(page, { faults: { aliasOnFirstSend: true } });
  await sendMessage(page, message);
  await expect(page).toHaveURL(/c=nyxid-chat-mock-/, { timeout: 2_000 });
  await expect(composerInput(page)).toHaveAttribute(
    "placeholder",
    "Message NyxID Assistant...",
    { timeout: 10_000 },
  );

  await conversationRow(page, message).hover();
  await page.getByRole("button", { name: `Options for ${message}` }).click();
  await page.getByRole("menuitem", { name: "Delete", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Delete chat?" });
  await dialog.getByRole("button", { name: "Delete", exact: true }).click();

  await expect(
    thread(page).getByText(
      "Ask NyxID to help with services, access, and account operations.",
    ),
  ).toBeVisible({ timeout: 5_000 });
  expect(new URL(page.url()).searchParams.has("c")).toBe(false);
  await expect(
    page.getByText("Could not load earlier messages", { exact: false }),
  ).toHaveCount(0);
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
