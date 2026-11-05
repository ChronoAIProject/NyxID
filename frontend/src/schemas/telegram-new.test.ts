import { expect, it } from "vitest";
import { telegramCreationLinkSchema, telegramNewRequestSchema } from "./telegram-new";

const request = {
  id: "3c638c7f-210a-44fc-9b67-f6c878d67c54", status: "waiting_bot", revision: 1,
  label: "Support", owner_user_id: "owner", expires_at: "2026-10-27T02:00:00Z",
  telegram_bot_id: null, bot_username: null, channel_bot_id: null,
};
it("accepts an old response without delivery or recovery fields", () => {
  expect(telegramNewRequestSchema.parse(request).can_open_creation_form).toBeUndefined();
});
it.each([
  "https://evil.test/newbot/NyxSetupBot/support_bot?name=Support",
  "https://t.me.evil.test/newbot/NyxSetupBot/support_bot?name=Support",
  "https://t.me/newbot/NyxSetupBot/support_bot?name=Support&start=private",
  "https://t.me/newbot/NyxSetupBot/support_bot?name=Support#private",
  "https://t.me/newbot/NyxSetupBot/support_bot?name=Support&name=Other",
  "https://t.me/NyxSetupBot?start=private",
])("rejects an unscoped recovery URL: %s", (creation_url) => {
  expect(telegramCreationLinkSchema.safeParse({ request, creation_url }).success).toBe(false);
});
it("accepts only the native managed form URL", () => {
  expect(telegramCreationLinkSchema.safeParse({ request, creation_url: "https://t.me/newbot/NyxSetupBot/support_bot?name=Support%20%26%20%E4%BD%A0" }).success).toBe(true);
});
