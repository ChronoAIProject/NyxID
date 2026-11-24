import { useId, useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { useUiTranslation, translateUi } from "@/i18n/use-ui-translation";
import { channelPlatformName, type ChannelThreadGroup } from "@/lib/assistant/nyxbot-labels";
import { channelSourceGroups, type ChannelSourceGroup } from "@/lib/assistant/channel-thread-presentation";
import { usePinnedConversationIds } from "@/hooks/use-conversation-pins";
import { pinnedConversationsFirst } from "@/stores/assistant-pins-store";
import { cn } from "@/lib/utils";
import type { Conversation } from "@/types/assistant";

function SourceGroup({ source, bot, activeId, searching, unreadIds, renderThread }: {
  readonly source: ChannelSourceGroup;
  readonly bot: ChannelThreadGroup;
  readonly activeId?: string;
  readonly searching: boolean;
  readonly unreadIds?: ReadonlySet<string>;
  readonly renderThread: (thread: Conversation) => ReactNode;
}) {
  useUiTranslation();
  const listId = useId();
  const holdsActive = source.threads.some((thread) => thread.id === activeId);
  const [folded, setFolded] = useState<{ active?: string; open: boolean }>();
  const open = searching || (folded && (folded.active === activeId || !holdsActive)
    ? folded.open : holdsActive);
  const running = source.threads.some((thread) => thread.active_turn);
  const unread = source.threads.some((thread) => unreadIds?.has(thread.id));
  const platform = channelPlatformName(bot.platform);
  return (
    <section className="mb-2" aria-label={source.title}>
      <button type="button" aria-expanded={open} aria-controls={listId}
        data-channel-group={bot.key} data-keep-drawer-open=""
        aria-label={translateUi("{{chat}} on {{platform}}, {{count}} threads{{status}}", {
          chat: source.title, platform, count: source.count,
          status: `${running ? translateUi(", working") : ""}${unread ? translateUi(", unread") : ""}`,
        })}
        onClick={() => setFolded({ active: activeId, open: !open })}
        className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left transition-colors hover:bg-overlay focus-visible:outline-2 focus-visible:outline-ring">
        <ChevronRight aria-hidden="true" className={cn("h-3 w-3 shrink-0 text-text-tertiary transition-transform", open && "rotate-90")} />
        <span className="min-w-0 flex-1">
          <span className="block truncate type-row-title text-foreground">{source.title}</span>
          <span className="mt-0.5 block truncate type-secondary text-text-tertiary">
            {translateUi("{{platform}} · Bot: {{bot}}", { platform, bot: bot.label })}
          </span>
        </span>
        {(running || (unread && !open)) ? <span aria-hidden="true" data-unread-marker={unread ? "" : undefined}
          className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary" /> : null}
        <span className="shrink-0 type-secondary text-text-tertiary">{translateUi("{{count}} threads", { count: source.count })}</span>
      </button>
      <div id={listId} hidden={!open} className="ml-3.5 space-y-1 border-l border-border/60 pl-2">
        {source.threads.map(renderThread)}
      </div>
      {open && source.followed ? <p className="pb-1 pl-6 pt-2 type-secondary text-text-tertiary">
        {translateUi("Separate discussions from this chat.")}
      </p> : null}
    </section>
  );
}

/** Source chat replaces the bot → parent → thread tree with one group level. */
export function ChannelSourceThreads({ bot, activeId, searching, unreadIds, renderThread }: {
  readonly bot: ChannelThreadGroup;
  readonly activeId?: string;
  readonly searching: boolean;
  readonly unreadIds?: ReadonlySet<string>;
  readonly renderThread: (thread: Conversation) => ReactNode;
}) {
  useUiTranslation();
  const pinnedIds = usePinnedConversationIds();
  const [shown, setShown] = useState(5);
  const ordered = pinnedConversationsFirst(bot.threads, pinnedIds);
  const visible = ordered.filter((thread, index) => searching || index < shown || thread.id === activeId);
  const hidden = ordered.length - visible.length;
  return <>
    {channelSourceGroups(bot, visible, pinnedIds).map((source) => <SourceGroup key={source.key}
      source={source} bot={bot} activeId={activeId} searching={searching} unreadIds={unreadIds} renderThread={renderThread} />)}
    {hidden > 0 ? <button type="button" data-keep-drawer-open="" onClick={() => setShown((value) => value + 20)}
      className="w-full rounded-lg px-3 py-1 text-left type-secondary text-text-tertiary hover:bg-overlay">
      {translateUi("Show {{count}} more", { count: Math.min(hidden, 20) })}
    </button> : null}
  </>;
}
