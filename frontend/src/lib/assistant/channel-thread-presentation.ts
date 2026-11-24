import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { channelChatTitle, type ChannelThreadGroup } from "./nyxbot-labels";
import type { Conversation } from "@/types/assistant";

/** Display-only: persisted titles and platform routing identities never change. */
export function conversationDisplayTitle(conversation: Conversation): string {
  if (!conversation.channel?.thread_id) {
    return conversation.channel?.chat_title?.trim() || conversation.title;
  }
  if (conversation.title_source !== "user" &&
    /^Thread · \d{4}-\d{2}-\d{2} \d{2}:\d{2} UTC$/.test(conversation.title) &&
    !Number.isNaN(Date.parse(conversation.created_at))) {
    const time = new Date(conversation.created_at).toLocaleTimeString(getFormattingLocale(), {
      hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23", timeZone: "UTC",
    });
    return translateUi("Thread · {{time}} UTC", { time });
  }
  return conversation.title;
}

export function channelSourceTitle(channel: NonNullable<Conversation["channel"]>): string {
  return channelChatTitle(channel.chat_kind,
    channel.thread_id ? channel.parent_title?.trim() : channel.chat_title?.trim(), channel.platform);
}

export interface ChannelSourceGroup {
  readonly key: string;
  readonly title: string;
  readonly followed: boolean;
  readonly count: number;
  readonly threads: Conversation[];
}

/** Preserve each bot's pinned/ordinary partition and opaque parent identities. */
export function channelSourceGroups(
  bot: ChannelThreadGroup,
  threads: readonly Conversation[],
  pinnedIds: ReadonlySet<string>,
): ChannelSourceGroup[] {
  const titles = new Map<string, string>();
  const followed = new Set<string>();
  const counts = new Map<string, number>();
  for (const thread of bot.threads) {
    const channel = thread.channel;
    if (!channel) continue;
    const parentId = channel.parent_chat_id ?? channel.chat_id ?? thread.id;
    const title = channel.thread_id ? channel.parent_title?.trim() : channel.chat_title?.trim();
    if (title) titles.set(parentId, title);
    if (channel.thread_id) followed.add(parentId);
    const key = JSON.stringify([pinnedIds.has(thread.id), parentId]);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const groups = new Map<string, ChannelSourceGroup>();
  for (const thread of threads) {
    const channel = thread.channel!;
    const parentId = channel.parent_chat_id ?? channel.chat_id ?? thread.id;
    const key = JSON.stringify([pinnedIds.has(thread.id), parentId]);
    const group = groups.get(key);
    if (group) {
      group.threads.push(thread);
    } else {
      groups.set(key, {
        key,
        title: titles.get(parentId) ?? channelSourceTitle(channel),
        followed: followed.has(parentId),
        count: counts.get(key) ?? 0,
        threads: [thread],
      });
    }
  }
  return [...groups.values()];
}
