import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, MessageSquare, Plus, Search, Settings2, X } from "lucide-react";
import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import { AgentAvatar } from "./nyxbot-agent-avatar";
import { ChannelPlatformGlyph } from "@/components/shared/channel-platform-glyph";
import { agentTitle, splitChannelThreads } from "@/lib/assistant/nyxbot-labels";
import { channelSourceTitle, conversationChannelCounts, conversationChannelName, conversationDisplayTitle } from "@/lib/assistant/channel-thread-presentation";
import { ChannelSourceThreads } from "./channel-source-threads";
import type { SidebarAgents } from "./assistant-sidebar";
import type { Conversation } from "@/types/assistant";
import { usePinnedConversationIds } from "@/hooks/use-conversation-pins";
import { pinnedConversationsFirst } from "@/stores/assistant-pins-store";

export function ThreadChannelSummary({ threads }: { readonly threads: readonly Conversation[] }) {
  useUiTranslation();
  const channels = conversationChannelCounts(threads);
  if (!channels.length) return null;
  return (
    <div aria-label={translateUi("Thread channels")} className="mt-3 flex flex-wrap items-center gap-2 type-secondary text-text-tertiary">
      <span className="shrink-0">{translateUi("Channels")}:</span>
      {channels.map(({ platform, name, count }) => {
        const label = translateUi("{{channel}} ({{count}})", { channel: name, count });
        return (
          <span key={platform} role="img" aria-label={label} title={label}
            className="inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full bg-overlay px-2 py-0.5">
            <ChannelPlatformGlyph platform={platform} className="size-4 shrink-0" />
            <span aria-hidden="true" className="tabular-nums">{count}</span>
          </span>
        );
      })}
    </div>
  );
}

function threadPeriod(thread: Conversation): string {
  const updated = new Date(thread.last_message_at);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const week = new Date(today);
  week.setDate(week.getDate() - 7);
  if (updated >= today) return "Today";
  if (updated >= yesterday) return "Yesterday";
  if (updated >= week) return "Previous 7 days";
  return "Earlier";
}

export function AssistantThreadList({
  model,
  renderThread,
  activeThreadId,
  notice,
  onClose,
}: {
  readonly model: SidebarAgents;
  readonly renderThread: (thread: Conversation) => ReactNode;
  readonly activeThreadId?: string;
  readonly notice?: ReactNode;
  readonly onClose?: () => void;
}) {
  useUiTranslation();
  const [query, setQuery] = useState("");
  const backButton = useRef<HTMLButtonElement>(null);
  const mobile = Boolean(model.onBackToWorkspace);
  useLayoutEffect(() => {
    if (mobile) backButton.current?.focus({ preventScroll: true });
  }, [mobile, model.selectedAgentId]);
  const pinnedIds = usePinnedConversationIds();
  const agent = model.agents.find((item) => item.id === model.selectedAgentId);
  if (!agent) return null;
  const name = agentTitle(agent);
  const search = query.trim().toLocaleLowerCase();
  const availableThreads = agent.can_use === false ? [] : model.threads;
  const filtered = availableThreads.filter((thread) =>
    [thread.title, conversationDisplayTitle(thread), thread.channel?.chat_title, thread.channel?.parent_title,
      thread.channel?.bot_label, conversationChannelName(thread),
      thread.channel ? channelSourceTitle(thread.channel) : undefined, thread.platform_thread?.platform]
      .some((value) => value?.toLocaleLowerCase().includes(search)),
  );
  const { own, bots } = splitChannelThreads(filtered);
  const periods = new Map<string, Conversation[]>();
  for (const thread of pinnedConversationsFirst(own, pinnedIds)) {
    const period = pinnedIds.has(thread.id) ? "Pinned" : threadPeriod(thread);
    const rows = periods.get(period);
    if (rows) rows.push(thread);
    else periods.set(period, [thread]);
  }
  const canChat = agent.status !== "destroyed" && agent.can_use !== false;

  return (
    <div className="assistant-thread-panel flex h-full min-h-0 flex-col" onKeyDown={(event) => {
      if (event.key === "Escape" && !event.defaultPrevented && onClose) {
        event.preventDefault();
        onClose();
      }
    }}>
      <header className="shrink-0 px-4 pb-3 pt-4">
        {model.onBackToWorkspace ? <button ref={backButton} type="button" data-keep-drawer-open="" onClick={model.onBackToWorkspace}
          className="mb-4 flex items-center gap-1.5 rounded-md py-1 type-secondary text-text-tertiary hover:text-foreground">
          <ArrowLeft aria-hidden="true" className="h-3.5 w-3.5" />{translateUi("Back to workspace")}
        </button> : null}
        <div className="flex min-w-0 items-center gap-3">
          <AgentAvatar agent={agent} size="lg" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate type-card-title text-foreground">{name}</h2>
            <p className="mt-0.5 text-12 text-text-tertiary">
              {translateUi("{{count}} threads", { count: availableThreads.length })}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {model.onAgentDetails ? (
              <button type="button" data-keep-drawer-open="" onClick={() => model.onAgentDetails?.(agent.id)}
                aria-label={translateUi("Agent details")} data-icon-button=""
                className="flex shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground">
                <Settings2 aria-hidden="true" className="h-4 w-4" />
              </button>
            ) : null}
            {onClose ? <button type="button" onClick={onClose} aria-label={translateUi("Close")}
              title={translateUi("Close")} data-icon-button=""
              className="flex shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground">
              <X aria-hidden="true" className="h-4 w-4" />
            </button> : null}
          </div>
        </div>
        <ThreadChannelSummary threads={availableThreads} />
        <div className="mt-3 flex items-center justify-between gap-2">
          <span className="type-group-label text-text-tertiary">{translateUi("Threads")}</span>
          {canChat ? (
            <button type="button" onClick={() => model.onNewThread(agent.id)}
              aria-label={translateUi("New chat with {{value1}}", { value1: name })}
              className="assistant-thread-new flex items-center gap-1.5 rounded-md px-2 py-1 type-button transition-colors">
              <Plus aria-hidden="true" className="h-3.5 w-3.5" />{translateUi("New chat")}
            </button>
          ) : null}
        </div>
        <div className="relative mt-2">
          <Search aria-hidden="true" className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-text-tertiary" />
          <input type="search" value={query} onChange={(event) => setQuery(event.target.value)}
            aria-label={translateUi("Search threads")} placeholder={translateUi("Search threads")}
            className="assistant-thread-search w-full rounded-lg border border-hairline bg-background/40 py-2 pl-8 pr-8 text-13 text-foreground outline-none placeholder:text-text-tertiary focus:border-ring" />
          {query ? (
            <button type="button" data-keep-drawer-open="" onClick={() => setQuery("")} aria-label={translateUi("Clear search")}
              className="absolute right-2 top-1/2 -translate-y-1/2 rounded text-text-tertiary hover:text-foreground">
              <X aria-hidden="true" className="h-3.5 w-3.5" />
            </button>
          ) : null}
        </div>
      </header>
      <nav aria-label={translateUi("Threads with {{value1}}", { value1: name })}
        data-scroll-fade="" className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-4">
        {notice}
        {own.length ? (
          <section aria-label={translateUi("Personal conversations")} className="mb-5">
            <div className="px-2 pb-3 pt-1">
              <h3 className="type-row-title text-foreground">{translateUi("Personal conversations")}</h3>
              <p className="mt-1 type-secondary text-text-tertiary">
                {translateUi("Private chats here or through a personal channel connection.")}
              </p>
            </div>
            {[...periods].map(([period, threads]) => (
              <section key={period} aria-label={translateUi(period)} className="mb-4 last:mb-0">
                <h4 className="px-2 pb-2 pt-1 type-group-label text-text-tertiary">{translateUi(period)}</h4>
                <div className="space-y-1">{threads.map(renderThread)}</div>
              </section>
            ))}
          </section>
        ) : null}
        {bots.length ? (
          <section aria-label={translateUi("Connected bot chats")} className="mb-4">
            <div className="px-2 pb-2 pt-1">
              <h3 className="type-row-title text-foreground">{translateUi("Connected bot chats")}</h3>
              <p className="mt-1 type-secondary text-text-tertiary">
                {translateUi("Conversations from linked bots, grouped by source chat.")}
              </p>
            </div>
            {bots.map((bot) => <ChannelSourceThreads key={bot.key} bot={bot} searching={Boolean(search)}
              activeId={activeThreadId} unreadIds={model.unreadThreadIds} renderThread={renderThread} />)}
          </section>
        ) : null}
        {!filtered.length ? (
          <div role="status" className="flex flex-col items-center gap-3 px-4 py-10 text-center text-13 text-text-tertiary">
            <MessageSquare aria-hidden="true" className="h-6 w-6 opacity-50" />
            <p>{agent.can_use === false ? translateUi("Chat access is restricted.") : model.threadsLoading ? translateUi("Loading threads...")
              : search ? translateUi("No matching threads") : translateUi("No threads yet")}</p>
          </div>
        ) : null}
      </nav>
    </div>
  );
}
