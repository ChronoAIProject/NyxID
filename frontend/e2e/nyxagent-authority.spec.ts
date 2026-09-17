import { test, expect } from "@playwright/test";
import { openAssistant, sendMessage, composerInput, stopButton } from "./helpers";
import { mockDashboard } from "./managed-onboarding-fixtures";

async function settled(page: import("@playwright/test").Page) {
  await expect(stopButton(page)).not.toBeVisible({ timeout: 6000 });
  await expect(composerInput(page)).toBeEnabled();
}

test("service card Allow is durable, focuses the composer, and only an explicit retry calls the tool", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  const card = page.getByRole("region", { name: "Allow this chat to use GitHub?" });
  await expect(card).toBeVisible();
  await settled(page);
  await card.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(card).toHaveCount(0);
  await expect(composerInput(page)).toBeFocused();
  await expect(page.getByText("GitHub access granted. Repository lookup succeeded.")).toHaveCount(
    0,
  );
  await page.reload();
  await expect(
    page.getByRole("status").filter({ hasText: "Allowed · Allow this chat to use GitHub?" }),
  ).toBeVisible();
  await sendMessage(page, "Continue");
  await expect(page.getByText("GitHub access granted. Repository lookup succeeded.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
});

test("Deny produces a refusal on retry without opening another card", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Use GitHub");
  const card = page.getByRole("region", { name: "Allow this chat to use GitHub?" });
  await expect(card).toBeVisible();
  await settled(page);
  await card.getByRole("button", { name: "Deny", exact: true }).click();
  await expect(composerInput(page)).toBeFocused();
  await sendMessage(page, "Continue");
  await expect(page.getByText("You denied this request. I will not proceed.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
});

test("account permission and single action confirmation are separate cards", async ({ page }) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "Manage my account");
  const account = page.getByRole("region", {
    name: /Allow this chat to manage your NyxID account/,
  });
  await expect(account).toBeVisible();
  await settled(page);
  await account.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(composerInput(page)).toBeFocused();
  await sendMessage(page, "Delete agent key ci-bot");
  const action = page.getByRole("region", { name: /Confirm: Delete agent key 'ci-bot'/ });
  await expect(action).toBeVisible();
  await settled(page);
  await action.getByRole("button", { name: "Allow", exact: true }).click();
  await expect(composerInput(page)).toBeFocused();
  await expect(page.getByText("Deleted agent key ci-bot.")).toHaveCount(0);
  await sendMessage(page, "Continue");
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: /Used · Confirm:/ })).toBeVisible();
});

test("assistant chat keys are hidden until requested and detail links back to the chat", async ({
  page,
}) => {
  await mockDashboard(page);
  const conversation = `nyxa-${"a".repeat(32)}`;
  const key = {
    id: "chat-key",
    name: "NyxID Assistant chat aaaaaaaa",
    platform: "nyxid-assistant",
    key_prefix: "nyxid_ag_fixture",
    scopes: "proxy",
    is_active: true,
    allowed_service_ids: [],
    allowed_services: [],
    allowed_node_ids: [],
    allowed_nodes: [],
    allow_all_services: false,
    allow_all_nodes: true,
    allow_auto_connected_services: true,
    bindings_count: 0,
    callback_url: null,
    created_at: "2026-09-17T00:00:00Z",
    assistant_conversation_id: conversation,
  };
  await page.route("**/api/v1/api-keys**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const body = path.endsWith("/api-keys")
      ? { keys: [key] }
      : path.endsWith("/chat-key")
        ? key
        : path.endsWith("/bindings")
          ? { bindings: [] }
          : path.endsWith("/credentials")
            ? { credentials: [] }
            : path.endsWith("/chat-key/usage")
              ? {
                  api_key_id: key.id,
                  api_key_name: key.name,
                  platform: key.platform,
                  request_count: 0,
                  success_count: 0,
                  error_count: 0,
                  error_rate: 0,
                  last_used_at: null,
                  prompt_tokens: 0,
                  completion_tokens: 0,
                  total_tokens: 0,
                  reported_cost: null,
                  top_services: [],
                  daily_buckets: [],
                }
              : { usage: [] };
    await route.fulfill({ json: body });
  });
  await page.route("**/api/v1/users/me/onboarding", (route) =>
    route.fulfill({ json: { completed: true, completed_at: "2026-09-17T00:00:00Z" } }),
  );
  await page.route("**/api/v1/keys", (route) => route.fulfill({ json: { keys: [] } }));
  await page.route("**/api/v1/catalog", (route) => route.fulfill({ json: { entries: [] } }));
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/keys?tab=nyxid");
  const toggle = page.getByRole("switch", { name: "Show assistant chat keys" });
  await expect(toggle).toBeVisible();
  await expect(page.getByText(key.name, { exact: true })).toHaveCount(0);
  await toggle.click();
  await page.getByText(key.name, { exact: true }).filter({ visible: true }).click();
  await expect(page.getByRole("link", { name: "Open chat" })).toHaveAttribute(
    "href",
    `/assistant?c=${conversation}`,
  );
  await expect(page.getByText("Daily Activity", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("switching an existing chat to Full access executes deletion without a card", async ({
  page,
}) => {
  await openAssistant(page, { faults: { nyxagentEnabled: true } });
  await sendMessage(page, "List connected services");
  await settled(page);
  await page.getByRole("combobox", { name: "Mode" }).click();
  await page.getByRole("option", { name: /Full access/ }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("all your connected services and nodes");
  await expect(dialog).toContainText("delete resources");
  await dialog.getByRole("button", { name: "Enable full access" }).click();
  await expect(page.getByRole("dialog", { includeHidden: true })).toHaveCount(0);
  await expect(page.locator("header").getByText("Full access", { exact: true })).toBeVisible();
  await sendMessage(page, "Delete agent key ci-bot");
  await expect(page.getByRole("combobox", { name: "Mode" })).toBeDisabled();
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
  await settled(page);
  await page.reload();
  await expect(page.locator("header").getByText("Full access", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "New chat", exact: true }).first().click();
  await expect(page.getByRole("combobox", { name: "Mode" })).toContainText("Full access");
  await sendMessage(page, "Delete agent key ci-bot");
  await expect(page.getByText("Deleted agent key ci-bot.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Allow", exact: true })).toHaveCount(0);
});
