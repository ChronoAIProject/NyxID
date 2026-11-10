import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateUi } from "@/i18n/use-ui-translation";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useNyxBotChannelThreads, useStopNyxBotChannelThread } from "@/hooks/use-nyxbot-agents";
import { cn, formatRelativeTime } from "@/lib/utils";
import type { NyxAgentChannelChat, NyxAgentChannelThread } from "@/schemas/assistant-nyxagent";

function ThreadRow({ thread, chat }: { readonly thread: NyxAgentChannelThread; readonly chat: NyxAgentChannelChat }) {
  const { t } = useUiTranslation();
  const stop = useStopNyxBotChannelThread();
  const labels = { active: "Following", opening: "Starting", stopped: "Stopped", expired: "Expired", unavailable: "Unavailable" };
  return <li className="space-y-1 rounded-md border border-hairline px-2 py-1.5">
    <div className="flex flex-wrap items-center gap-2">
      {thread.conversation_id ? <Link to="/assistant" search={{ c: thread.conversation_id }} className="min-w-0 flex-1 truncate text-12 hover:underline">{thread.label}</Link>
        : <span className="flex-1 text-12">{thread.label}</span>}
      <Badge variant={thread.state === "active" ? "success" : "secondary"}>{thread.state === "active" && thread.follow_readiness !== "ready" ? t("Unavailable") : labels[thread.state]}</Badge>
      {thread.state === "active" || thread.state === "opening" ? <Button size="sm" variant="ghost" isLoading={stop.isPending}
        onClick={() => stop.mutate({ channelId: chat.channel_agent_id, chatId: chat.id, threadId: thread.id })}>{t("Stop following")}</Button> : null}
    </div>
    <p className="text-11 text-muted-foreground">
      {thread.last_admitted_at ? t("Last activity {{value}}. ", { value: formatRelativeTime(thread.last_admitted_at) }) : ""}
      {thread.state === "active" && thread.expires_at ? t("Idle expiry {{value}}. ", { value: new Date(thread.expires_at).toLocaleString(getFormattingLocale()) }) : ""}
      {(thread.dropped_message_count ?? 0) > 0 ? t("{{dropped_message_count}} messages arrived while busy and were not queued. ", { dropped_message_count: thread.dropped_message_count }) : ""}
      {thread.kind === "topic" ? t("Follows the whole topic. ") : ""}
      {thread.context_status === "metadata_only" ? t("Earlier message bodies were unavailable.") : thread.context_status === "partial" ? t("Earlier context is partial.") : ""}
    </p>
    {stop.error ? <p role="alert" className="text-11 text-destructive">{translateUi(stop.error.message ?? "")}</p> : null}
  </li>;
}

export function ChannelThreads({ chat }: { readonly chat: NyxAgentChannelChat }) {
  const { t } = useUiTranslation();
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState(false);
  const threads = useNyxBotChannelThreads(chat.channel_agent_id, chat.id, history ? "all" : "active", open);
  const rows = threads.data?.pages.flatMap((page) => page.threads) ?? [];
  return <div className="space-y-2">
    <div className="flex flex-wrap items-center gap-3 text-11">
      <button type="button" aria-expanded={open} className="flex items-center gap-1 text-muted-foreground hover:text-foreground" onClick={() => setOpen(!open)}>
        <ChevronRight aria-hidden="true" className={cn("h-3 w-3", open && "rotate-90")} />{t("Followed threads (")}{chat.followed_thread_count ?? 0})
      </button>
      {chat.conversation_id ? <Link to="/assistant" search={{ c: chat.conversation_id }} className="text-muted-foreground hover:underline">{t("Chat history")}</Link> : null}
    </div>
    {open ? <div className="space-y-2 border-l border-hairline pl-2">
      {chat.reply_mode === "all" ? <p className="text-11 text-muted-foreground">{t("Every message still allows replies after you stop following.")}</p> : null}
      <label className="flex items-center gap-2 text-11 text-muted-foreground"><Checkbox checked={history} onCheckedChange={(value) => setHistory(value === true)} />{t("Include stopped and expired threads")}</label>
      {threads.error ? <ErrorBanner message={threads.error.message} onRetry={() => void threads.refetch()} />
        : threads.isPending ? <p className="text-11 text-muted-foreground">{t("Loading threads...")}</p>
        : rows.length ? <ul aria-label={t("Platform threads")} className="space-y-1">{rows.map((thread) => <ThreadRow key={thread.id} thread={thread} chat={chat} />)}</ul>
        : <p className="text-11 text-muted-foreground">{t(history ? "No recorded threads." : "No active threads.")}</p>}
      {threads.hasNextPage ? <Button size="sm" variant="ghost" isLoading={threads.isFetchingNextPage} onClick={() => void threads.fetchNextPage()}>{t("Show more threads")}</Button> : null}
    </div> : null}
  </div>;
}
