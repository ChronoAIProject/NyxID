import { afterEach, describe, expect, it } from "vitest";
import { appI18n } from "@/i18n";
import { channelSourceGroups, channelSourceTitle, conversationDisplayTitle } from "./channel-thread-presentation";
import type { Conversation } from "@/types/assistant";

const thread: Conversation = {
  id: "conversation", title: "Thread · 2026-11-02 10:16 UTC", title_source: "user",
  created_at: "2026-11-02T10:16:12Z", last_message_at: "2026-11-02T11:16:00Z",
  channel: { platform: "lark", channel_agent_id: "bot", bot_label: "NyxBot",
    chat_id: "child", thread_id: "child", parent_chat_id: "parent", parent_title: "Engineering", chat_kind: "group" },
};

afterEach(async () => { await appI18n.changeLanguage("en"); });

describe("channel thread presentation", () => {
  it("distinguishes creation times in the same minute without using last activity", () => {
    expect(conversationDisplayTitle(thread)).toContain("10:16:12");
    expect(conversationDisplayTitle({ ...thread, created_at: "2026-11-02T10:16:45Z" })).toContain("10:16:45");
    expect(conversationDisplayTitle(thread)).not.toContain("11:16");
  });

  it("keeps user titles and does not replace a child title with its chat title", () => {
    expect(conversationDisplayTitle({ ...thread, title: "Thread · 2026-11-02 10:15 UTC" })).toBe("Thread · 2026-11-02 10:15 UTC");
    expect(conversationDisplayTitle({ ...thread, title: "Deployment", channel: { ...thread.channel!, chat_title: "Engineering" } })).toBe("Deployment");
    expect(conversationDisplayTitle({ ...thread, created_at: "invalid" })).toBe(thread.title);
  });

  it("preserves the legacy named-chat display and gives missing parents a platform fallback", () => {
    expect(conversationDisplayTitle({ ...thread, channel: { platform: "lark", chat_title: "Team" } })).toBe("Team");
    expect(channelSourceTitle({ ...thread.channel!, parent_title: "  " })).toBe("Lark group");
  });

  it("keeps full counts and parent metadata when a list only shows a subset", () => {
    const sibling = { ...thread, id: "second", channel: { ...thread.channel!, parent_title: null } };
    const bot = { key: "bot", label: "NyxBot", platform: "lark", threads: [thread, sibling] };
    expect(channelSourceGroups(bot, [sibling], new Set())).toMatchObject([
      { title: "Engineering", followed: true, count: 2, threads: [sibling] },
    ]);
    expect(channelSourceGroups(bot, [sibling, thread], new Set([sibling.id])).map((source) => source.threads[0]?.id)).toEqual(["second", "conversation"]);
  });

  it("uses the selected interface language for generic thread labels", async () => {
    await appI18n.changeLanguage("zh-CN");
    expect(conversationDisplayTitle(thread)).toMatch(/^讨论串 · /);
  });
});
