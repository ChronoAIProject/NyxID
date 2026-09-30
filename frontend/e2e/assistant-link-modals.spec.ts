import { test, expect, type Page } from "@playwright/test";
import { openAssistant, sendMessage } from "./helpers";

async function openDemo(page: Page, nyxagentEnabled: boolean) {
  await openAssistant(page, { faults: { nyxagentEnabled } });
  if (!nyxagentEnabled) {
    await page
      .getByRole("button", { name: "New chat", exact: true })
      .first()
      .click();
  }
}

test("the manual NyxBot demo URL works without test initialization and survives reload", async ({
  page,
}) => {
  await page.goto("/assistant?mock=1&nyxbot=1");
  await expect(
    page.getByRole("button", { name: "New agent", exact: true }).first(),
  ).toBeVisible();
  await sendMessage(page, "connect to my github");
  const link = page.getByRole("link", { name: "Connect GitHub", exact: true });
  await expect(link).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole("button", { name: "New agent", exact: true }).first(),
  ).toBeVisible();
  await link.click();
  const dialog = page.getByRole("dialog", { name: "Connect a service" });
  await expect(
    dialog.getByLabel("Personal access token", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Approve & connect" }),
  ).toBeDisabled();
});

for (const nyxagentEnabled of [false, true]) {
  test.describe(
    nyxagentEnabled ? "NyxBot setup modals" : "Actor setup modals",
    () => {
      test("connector stays pending through idle and dismissal, and completes only on submit", async ({
        page,
      }, testInfo) => {
        await openDemo(page, nyxagentEnabled);
        await sendMessage(page, "connect to my github");
        const link = page
          .getByRole("link", { name: "Connect GitHub", exact: true })
          .last();
        await expect(link).toBeVisible();
        await expect(
          page.getByText("Approved and sent", { exact: true }),
        ).toHaveCount(0);
        await expect(page.getByRole("dialog")).toHaveCount(0);
        const chatUrl = page.url();
        const firstHref = await link.getAttribute("href");
        await link.click();
        const dialog = page.getByRole("dialog", { name: "Connect a service" });
        const token = dialog.getByLabel("Personal access token", {
          exact: true,
        });
        const submit = dialog.getByRole("button", {
          name: "Approve & connect",
        });
        await expect(token).toBeVisible();
        await expect(submit).toBeDisabled();
        // Longer than the old auto-completion timer: opening must never approve.
        await page.waitForTimeout(4000);
        await expect(submit).toBeDisabled();
        await expect(
          page.getByText("Connection completed", { exact: true }),
        ).toHaveCount(0);
        await dialog
          .getByRole("button", { name: "Close", exact: true })
          .click();
        await expect(dialog).toHaveCount(0);
        await link.click();
        await expect(token).toHaveValue("");
        await token.fill("demo-github-token");
        await expect(submit).toBeEnabled();
        await page.waitForTimeout(4000);
        await expect(submit).toBeEnabled();
        await expect(
          dialog.getByText("Connection completed", { exact: true }),
        ).toHaveCount(0);
        await page.screenshot({
          path: testInfo.outputPath("connector-pending.png"),
        });
        await submit.click();
        await expect(
          dialog.getByText("Connection completed", { exact: true }),
        ).toBeVisible();
        await expect(page).toHaveURL(chatUrl);
        expect(page.context().pages()).toHaveLength(1);
        await dialog
          .getByRole("button", { name: "Close", exact: true })
          .click();
        // A second request receives a fresh pending link, including after success.
        await sendMessage(page, "connect to my github");
        await expect.poll(() => link.getAttribute("href")).not.toBe(firstHref);
        await link.click();
        await expect(token).toHaveValue("");
        await expect(submit).toBeDisabled();
      });

      test("declining a connector stays cancelled when reopened", async ({
        page,
      }) => {
        await openDemo(page, nyxagentEnabled);
        await sendMessage(page, "connect to my github");
        const link = page.getByRole("link", {
          name: "Connect GitHub",
          exact: true,
        });
        await link.click();
        const chatUrl = page.url();
        const dialog = page.getByRole("dialog", { name: "Connect a service" });
        await dialog
          .getByRole("button", { name: "Decline", exact: true })
          .click();
        await expect(
          dialog.getByRole("heading", { name: "Connection cancelled" }),
        ).toBeVisible();
        await dialog
          .getByRole("button", { name: "Close", exact: true })
          .click();
        await link.click();
        await expect(
          dialog.getByRole("heading", { name: "Connection cancelled" }),
        ).toBeVisible();
        await expect(
          dialog.getByText("Connection completed", { exact: true }),
        ).toHaveCount(0);
        await expect(page).toHaveURL(chatUrl);
        expect(page.context().pages()).toHaveLength(1);
      });

      test("channel setup waits for input and Add Bot; cancellation does not create it", async ({
        page,
      }, testInfo) => {
        await openDemo(page, nyxagentEnabled);
        await sendMessage(page, "set up a telegram bot");
        const link = page.getByRole("link", {
          name: "Telegram bot setup",
          exact: true,
        });
        await expect(link).toBeVisible();
        const chatUrl = page.url();
        // The former three-second fixture must not claim a bot before setup.
        await page.waitForTimeout(4000);
        await expect(
          page.getByText(/Your Telegram bot @helper_bot is linked/),
        ).toHaveCount(0);
        await expect(
          page.getByText("Approved and sent", { exact: true }),
        ).toHaveCount(0);
        await link.click();
        const dialog = page.getByRole("dialog");
        const token = dialog.getByLabel("Bot token", { exact: true });
        const submit = dialog.getByRole("button", {
          name: "Add Bot",
          exact: true,
        });
        await expect(token).toBeVisible();
        await expect(submit).toBeDisabled();
        await dialog
          .getByRole("button", { name: "Cancel", exact: true })
          .click();
        await expect(dialog).toHaveCount(0);
        await link.click();
        await expect(token).toHaveValue("");
        await token.fill("demo-bot-token");
        await expect(submit).toBeEnabled();
        await page.waitForTimeout(4000);
        await expect(submit).toBeEnabled();
        await expect(
          page.getByText(/Your Telegram bot @helper_bot is linked/),
        ).toHaveCount(0);
        await page.screenshot({
          path: testInfo.outputPath("channel-pending.png"),
        });
        await submit.click();
        await expect(
          dialog.getByRole("heading", { name: /Bot Created/ }),
        ).toBeVisible();
        await expect(
          dialog.getByRole("button", { name: "Done", exact: true }),
        ).toBeVisible();
        await expect(page).toHaveURL(chatUrl);
        expect(page.context().pages()).toHaveLength(1);
        await dialog.getByRole("button", { name: "Done", exact: true }).click();
        await expect(dialog).toHaveCount(0);
        await expect(page.locator("textarea")).toBeVisible();
        if (nyxagentEnabled) {
          await expect(
            page.getByText(/Your Telegram bot @helper_bot is linked/),
          ).toBeVisible({ timeout: 15_000 });
        }
        await expect(page).toHaveURL(chatUrl);
      });
    },
  );
}
