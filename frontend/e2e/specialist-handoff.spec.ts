import { expect, test, type Page } from "@playwright/test";
import { openAssistant } from "./helpers";

const task =
  "Help me create a Telegram bot named Support and link it to my researcher specialist.";

async function specialistReply(page: Page, text: string): Promise<string> {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  const sourceId = await page.evaluate((reply) => {
    const key = "nyxagent-http-fixture-v4";
    const state = JSON.parse(sessionStorage.getItem(key)!);
    const agent = state.agents.find(
      (row: { kind: string }) => row.kind === "specialist",
    );
    const entry = state.rows.find(
      (row: [string, { agentId: string }]) => row[1].agentId === agent.id,
    );
    const message = entry[1].history.messages.find(
      (row: { role: string }) => row.role === "assistant",
    );
    message.text = reply;
    message.origin = "user";
    sessionStorage.setItem(key, JSON.stringify(state));
    return entry[0] as string;
  }, text);
  await page.goto(`/assistant?mock=1&c=${sourceId}`);
  return sourceId;
}

for (const width of [1440, 390]) {
  test(`specialist handoff starts a fresh NyxBot thread in the same app at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const artifact = `\`\`\`nyxbot-handoff\n${JSON.stringify({ version: 1, title: "Create a Telegram bot", task })}\n\`\`\``;
    const source = await specialistReply(
      page,
      `Account setup needs NyxBot.\n\n${artifact}`,
    );
    const card = page.getByRole("region", { name: "Continue with NyxBot" });
    // A named section with an aria-label is a region; no turn starts on mount.
    await expect(card.getByText(task, { exact: true })).toBeVisible();
    const before = await page.evaluate(
      () =>
        JSON.parse(sessionStorage.getItem("nyxagent-http-fixture-v4")!).rows
          .length,
    );
    await page.screenshot({
      path: testInfo.outputPath("specialist-handoff.png"),
    });
    await card.getByRole("button", { name: "Continue with NyxBot" }).click();
    await expect(page).not.toHaveURL(new RegExp(`c=${source}`));
    await expect(page).toHaveURL(/c=nyxa-[a-f0-9]{32}/);
    await expect
      .poll(() =>
        page.evaluate(() => {
          const state = JSON.parse(
            sessionStorage.getItem("nyxagent-http-fixture-v4")!,
          );
          const id = new URL(window.location.href).searchParams.get("c");
          const row = state.rows.find(
            (entry: [string]) => entry[0] === id,
          )?.[1];
          return {
            count: state.rows.length,
            kind: state.agents.find(
              (agent: { id: string }) => agent.id === row?.agentId,
            )?.kind,
            text: row?.history.messages.find(
              (message: { role: string }) => message.role === "user",
            )?.text,
          };
        }),
      )
      .toEqual({
        count: before + 1,
        kind: "nyxbot",
        text: `${task}\n\n[Specialist chat](/assistant?c=${source})`,
      });
    expect(page.context().pages()).toHaveLength(1);
    const destination = page.url();
    await page.goto(`/assistant?mock=1&c=${source}`);
    await expect(
      card.getByRole("button", { name: "Open NyxBot chat" }),
    ).toBeVisible();
    await page.reload();
    await card.getByRole("button", { name: "Open NyxBot chat" }).click();
    await expect(page).toHaveURL(destination);
    expect(
      await page.evaluate(
        () =>
          JSON.parse(sessionStorage.getItem("nyxagent-http-fixture-v4")!).rows
            .length,
      ),
    ).toBe(before + 1);
  });
}

test("hosted Telegram setup opens over the source chat and creates no tab", async ({
  page,
}, testInfo) => {
  const link = {
    id: "hosted-bot-link",
    status: "pending",
    platform: "telegram-new",
    label: "Support",
    owner_id: null,
    requested_by: "Your NyxBot",
    expires_at: "2099-01-01T00:00:00Z",
    bot_id: null,
    connection_id: null,
    telegram_request_id: null,
    callback_url: "https://example.com/callback",
    last_error: null,
    delivery_status: null,
  };
  await page.route("**/api/v1/channel-connect-links/**", (route) =>
    route.fulfill({ json: link }),
  );
  const source = await specialistReply(
    page,
    "[Create Telegram bot](/connect/bot/nyx_bcl_browser_example)",
  );
  await page.getByRole("link", { name: "Create Telegram bot" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Set up your channel bot",
    exact: true,
  });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Create your Telegram channel bot" }),
  ).toBeVisible();
  await expect(dialog.getByText("Your NyxBot", { exact: false })).toBeVisible();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Continue in Telegram", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await page.mouse.click(10, 500);
  await expect(dialog).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("telegram-in-chat.png") });
  await expect(page).toHaveURL(new RegExp(`c=${source}`));
  expect(page.context().pages()).toHaveLength(1);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByRole("link", { name: "Create Telegram bot" }),
  ).toBeFocused();
});

test("one-time hosted bot results survive Escape/outside click and return to the same chat", async ({
  page,
}) => {
  let completed = false;
  const botId = "11111111-1111-4111-8111-111111111111";
  const link = {
    id: "hosted-manual",
    status: "pending",
    platform: "telegram",
    label: "Support",
    owner_id: null,
    requested_by: "Your NyxBot",
    expires_at: "2099-01-01T00:00:00Z",
    bot_id: null,
    connection_id: null,
    telegram_request_id: null,
    callback_url: "https://example.com/callback",
    last_error: null,
    delivery_status: null,
  };
  await page.route("**/api/v1/channel-connect-links/**", async (route) => {
    if (new URL(route.request().url()).pathname.endsWith("/complete")) {
      completed = true;
      await route.fulfill({
        json: {
          id: botId,
          platform: "telegram",
          label: "Support",
          platform_bot_username: "support_bot",
          status: "active",
          webhook_url: "https://nyxid.test/webhook",
          webhook_secret: "one-time-verify-value",
          webhook_secret_label: "Webhook Secret",
          setup_instructions: [],
        },
      });
    } else
      await route.fulfill({
        json: {
          ...link,
          status: completed ? "completed" : "pending",
          bot_id: completed ? botId : null,
        },
      });
  });
  const source = await specialistReply(
    page,
    "[Set up Telegram](/connect/bot/nyx_bcl_manual)",
  );
  await expect(
    page.getByRole("link", { name: "Set up Telegram" }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveClass(/assistant-display/);
  const rootStyle = await page.locator("html").getAttribute("style");
  const rootClass = await page.locator("html").getAttribute("class");
  await page.getByRole("link", { name: "Set up Telegram" }).click();
  const dialog = page.getByRole("dialog", {
    name: "Set up your channel bot",
    exact: true,
  });
  await dialog.getByLabel("Bot token", { exact: true }).fill("demo-bot-token");
  await dialog
    .getByRole("button", { name: "Connect bot", exact: true })
    .click();
  await expect(
    dialog.getByText("one-time-verify-value", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("link", { name: "Open channel bot" }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.mouse.click(10, 500);
  await expect(
    dialog.getByText("one-time-verify-value", { exact: true }),
  ).toBeVisible();
  await dialog.getByRole("button", { name: "Continue", exact: true }).click();
  await dialog
    .getByRole("button", { name: "Back to chat", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`c=${source}`));
  expect(page.context().pages()).toHaveLength(1);
  await expect(page.locator("html")).toHaveAttribute("class", rootClass!);
  await expect(page.locator("html")).toHaveAttribute("style", rootStyle!);
});
