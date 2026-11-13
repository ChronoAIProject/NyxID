import { afterEach, expect, it } from "vitest";
import { appI18n } from "@/i18n/instance";
import cn from "@/i18n/locales/zh-CN.json";
import tw from "@/i18n/locales/zh-TW.json";
import { nyxAgentChannelAgentSchema } from "@/schemas/assistant-nyxagent";
import { botAudienceDescription } from "./bot-audience";
const channel = nyxAgentChannelAgentSchema.parse({
  id: "b",
  channel_bot_id: "b",
  platform: "lark",
  bot_label: "Bot",
  transport: "direct",
  status: "active",
  owner_linked: false,
  created_at: "2026-11-11T00:00:00Z",
  org_specialist: {
    org_id: "org",
    guest_enabled: false,
    delegated_service_ids: [],
  },
  guest_access_description: "Untranslated server English",
});
afterEach(async () => {
  await appI18n.changeLanguage("en");
});
it.each([
  ["zh-CN", cn, "组织支付"],
  ["zh-TW", tw, "組織支付"],
] as const)(
  "uses localized %s payer/member/admin footer instead of server English",
  async (locale, catalog, phrase) => {
    appI18n.addResourceBundle(locale, "app", catalog, true, true);
    await appI18n.changeLanguage(locale);
    const copy = botAudienceDescription(channel);
    expect(copy).toContain(phrase);
    expect(copy).not.toContain("Untranslated");
    expect(copy).not.toContain("Guests");
  },
);
