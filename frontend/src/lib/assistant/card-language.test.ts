import { describe, expect, it } from "vitest";
import type { ChatMessage } from "./chat-types";
import { conversationCardLanguage } from "./card-language";

function user(content: string, extra?: Partial<ChatMessage>): ChatMessage {
  return { role: "user", id: content, content, status: "complete", timestamp: 0, ...extra };
}

const english = [user("Could you check the permissions for this skill?"), user("Please show me the current settings.")];
const simplified = [user("请帮我检查这个服务的配置"), user("我需要确认这个操作的范围。")];
const traditional = [user("請幫我檢查這個服務的設定"), user("我需要確認這個操作的範圍。")];

describe("conversation card language", () => {
  it("requires sustained language and switches after two new signals", () => {
    expect(conversationCardLanguage(english.slice(0, 1), "zh-TW")).toBe("zh-TW");
    expect(conversationCardLanguage([...simplified, english[0]!], "en")).toBe("zh-CN");
    expect(conversationCardLanguage([...simplified, ...english], "zh-TW")).toBe("en");
    expect(conversationCardLanguage([...english, ...traditional], "en")).toBe("zh-TW");
  });
  it("retains the established script across script-neutral Chinese messages", () => {
    const neutral = [user("我想看看今天的公告"), user("你能看看昨天的公告")];
    expect(conversationCardLanguage([...traditional, ...neutral], "en")).toBe("zh-TW");
    expect(conversationCardLanguage([...simplified, ...neutral], "zh-TW")).toBe("zh-CN");
    expect(conversationCardLanguage(neutral, "zh-TW")).toBe("zh-TW");
  });
  it("ignores commands, code, links, tool output and automated turns", () => {
    const noise = [
      user("API"), user("https://example.org/please/show/me/the/settings"), user("```\nPlease show me the API credentials.\n```"),
      user("const result = 'Please show me the API credentials'"), user("> Please show me the current settings."),
      user("Please show me the current settings.", { role: "assistant" }),
      user("Please show me the current settings.", { origin: "trigger" }),
      user("Please show me the current settings.", { continuation_of: "card" }),
      user("Please show me the current settings.", { role: "event" }),
    ];
    expect(conversationCardLanguage([...traditional, ...noise], "en")).toBe("zh-TW");
  });
  it("uses the app for unsupported or ambiguous text", () => {
    expect(conversationCardLanguage([user("Pouvez-vous vérifier les autorisations ?"), user("Je voudrais voir les paramètres.")], "zh-CN")).toBe("zh-CN");
    expect(conversationCardLanguage([...english, user("Pouvez-vous vérifier les autorisations ?"), user("Je voudrais voir les paramètres.")], "zh-TW")).toBe("zh-TW");
    expect(conversationCardLanguage([user("この設定を確認してください"), user("この画面の設定を見てください")], "en")).toBe("en");
    expect(conversationCardLanguage([user("请帮我 check the settings for this service"), user("请帮我 check the settings for this file")], "zh-TW")).toBe("zh-TW");
  });
});
