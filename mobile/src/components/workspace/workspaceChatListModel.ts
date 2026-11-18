import type { AssistantAgent } from "../../features/agents/agentContracts";
import type { AssistantConversation } from "../../features/assistant/assistantTypes";

export const DRAWER_PREVIEW_LIMIT = 5;

export function channelPlatformName(platform: string) {
  const names: Record<string, string> = {
    "telegram-new": "Telegram",
    telegram: "Telegram",
    lark: "Lark",
    feishu: "Feishu",
    whatsapp: "WhatsApp",
    aurinko: "Email",
    x: "X",
  };
  return (
    names[platform] ||
    (platform ? platform[0]!.toUpperCase() + platform.slice(1) : "Channel")
  );
}

export function threadTitle(thread: AssistantConversation) {
  return (
    thread.channel?.chat_title?.trim() || thread.title.trim() || "Untitled chat"
  );
}

export function threadStatus(thread: AssistantConversation) {
  return thread.pending_acknowledgements > 0
    ? "Needs you"
    : thread.active_turn
      ? "Running"
      : undefined;
}

/** Bot connection IDs, rather than names, keep separate bots and chats apart. */
export function splitChannelThreads(threads: readonly AssistantConversation[]) {
  const own: AssistantConversation[] = [];
  const bots = new Map<
    string,
    {
      key: string;
      platform: string;
      label: string;
      threads: AssistantConversation[];
    }
  >();
  for (const thread of threads) {
    if (!thread.channel) {
      own.push(thread);
      continue;
    }
    const channel = thread.channel;
    const key = channel.channel_agent_id || `platform:${channel.platform}`;
    let bot = bots.get(key);
    if (!bot) {
      bot = {
        key,
        platform: channel.platform,
        label:
          channel.bot_label || `${channelPlatformName(channel.platform)} bot`,
        threads: [],
      };
      bots.set(key, bot);
    }
    bot.threads.push(thread);
  }
  return { own, bots: [...bots.values()] };
}

/** Keep an active or expanded row visible without growing the drawer preview. */
export function boundedDrawerPreview<T extends { id: string }>(
  items: readonly T[],
  preferredId?: string,
  limit = DRAWER_PREVIEW_LIMIT,
): T[] {
  const preview = items.slice(0, Math.max(0, Math.floor(limit)));
  if (
    !preview.length ||
    preferredId === undefined ||
    preview.some((item) => item.id === preferredId)
  )
    return preview;
  const preferred = items.find((item) => item.id === preferredId);
  if (preferred) preview[preview.length - 1] = preferred;
  return preview;
}

const normalizedDrawerSearch = (value: string) =>
  value.trim().replace(/\s+/gu, " ").toLowerCase();

/** Search the supplied label and metadata with the same whitespace rules. */
export function searchDrawerItems<T>(
  items: readonly T[],
  query: string,
  searchText: (item: T) => string,
): T[] {
  const needle = normalizedDrawerSearch(query);
  return needle
    ? items.filter((item) =>
        normalizedDrawerSearch(searchText(item)).includes(needle),
      )
    : [...items];
}

export function orderedDrawerAgents(agents: readonly AssistantAgent[]) {
  return [...agents].sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "nyxbot" ? -1 : 1;
    if ((a.status === "destroyed") !== (b.status === "destroyed"))
      return a.status === "destroyed" ? 1 : -1;
    return (a.display_name || a.name).localeCompare(b.display_name || b.name);
  });
}

/** Never infer ownership from a chat title or an agent's display name. */
export function drawerAgentConversations(
  pages: readonly { conversations: readonly AssistantConversation[] }[],
  agent: Pick<AssistantAgent, "id" | "kind">,
): AssistantConversation[] {
  const rows = new Map<string, AssistantConversation>();
  for (const page of pages) {
    for (const row of page.conversations) {
      if (row.agent ? row.agent.id !== agent.id : agent.kind !== "nyxbot")
        continue;
      const existing = rows.get(row.id);
      if (!existing || row.last_message_at > existing.last_message_at)
        rows.set(row.id, row);
    }
  }
  return [...rows.values()].sort((a, b) =>
    b.last_message_at.localeCompare(a.last_message_at),
  );
}
