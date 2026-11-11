import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import {
  GroupedFooter,
  GroupedList,
  GroupedRow,
  GroupedDisclosure,
} from "@/components/ui/grouped-list";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useNyxBotChannelThreads, useStopNyxBotChannelThread } from "@/hooks/use-nyxbot-agents";
import { formatRelativeTime } from "@/lib/utils";
import type { NyxAgentChannelChat, NyxAgentChannelThread } from "@/schemas/assistant-nyxagent";

function ThreadRow({
  thread,
  chat,
}: {
  readonly thread: NyxAgentChannelThread;
  readonly chat: NyxAgentChannelChat;
}) {
  useUiTranslation();
  const stop = useStopNyxBotChannelThread();
  const labels = { active: "Following", opening: "Starting", stopped: "Stopped", expired: "Expired", unavailable: "Unavailable" };
  return (
    <>
      <GroupedRow asChild className="block space-y-1">
        <li>
          <div className="flex flex-wrap items-center gap-2">
            {thread.conversation_id ? (
              <Link
                to="/assistant"
                search={{ c: thread.conversation_id }}
                className="min-w-0 flex-1 break-words text-14 hover:underline"
              >
                {thread.label}
              </Link>
            ) : (
              <span className="flex-1 text-14">{thread.label}</span>
            )}
            <span className="type-secondary text-text-tertiary">
              {thread.state === "active" && thread.follow_readiness !== "ready"
                ? translateUi("Unavailable")
                : labels[thread.state]}
            </span>
            {thread.state === "active" || thread.state === "opening" ? (
              <Button
                size="sm"
                variant="ghost"
                isLoading={stop.isPending}
                onClick={() =>
                  stop.mutate({
                    channelId: chat.channel_agent_id,
                    chatId: chat.id,
                    threadId: thread.id,
                  })
                }
              >{translateUi("Stop following")}</Button>
            ) : null}
          </div>
          <p className="type-secondary text-text-tertiary">
            {thread.last_admitted_at
              ? translateUi("Last activity {{value1}}. ", { value1: formatRelativeTime(thread.last_admitted_at) })
              : ""}
            {thread.state === "active" && thread.expires_at
              ? translateUi("Idle expiry {{value1}}. ", { value1: new Date(thread.expires_at).toLocaleString(getFormattingLocale()) })
              : ""}
          </p>
        </li>
      </GroupedRow>
      <GroupedFooter>
        <p>
          {(thread.dropped_message_count ?? 0) > 0
            ? translateUi("{{value1}} messages arrived while busy and were not queued. ", { value1: thread.dropped_message_count })
            : ""}
          {thread.kind === "topic" ? translateUi("Follows the whole topic. ") : ""}
          {thread.context_status === "metadata_only"
            ? translateUi("Earlier message bodies were unavailable.")
            : thread.context_status === "partial"
              ? translateUi("Earlier context is partial.")
              : ""}
        </p>
        {stop.error ? (
          <p role="alert" className="text-13 text-destructive">
            {stop.error.message}
          </p>
        ) : null}
      </GroupedFooter>
    </>
  );
}

export function ChannelThreads({
  chat,
}: {
  readonly chat: NyxAgentChannelChat;
}) {
  useUiTranslation();
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState(false);
  const threads = useNyxBotChannelThreads(chat.channel_agent_id, chat.id, history ? "all" : "active", open);
  const rows = threads.data?.pages.flatMap((page) => page.threads) ?? [];
  return (
    <GroupedList
      title={translateUi("Threads")}
      footer={
        <>
          {chat.reply_mode === "all" ? (
            <p>{translateUi("Every message still allows replies after you stop following.")}</p>
          ) : null}
          {threads.error ? (
            <p role="alert" className="text-destructive">
              {threads.error.message}
            </p>
          ) : null}
        </>
      }
    >
      <GroupedDisclosure aria-expanded={open} onClick={() => setOpen(!open)}>{translateUi("Followed threads (")}{chat.followed_thread_count ?? 0})
      </GroupedDisclosure>
      {chat.conversation_id ? (
        <GroupedRow asChild interactive>
          <Link to="/assistant" search={{ c: chat.conversation_id }}>{translateUi("Chat history")}</Link>
        </GroupedRow>
      ) : null}
      {open ? (
        <>
          <GroupedRow asChild interactive>
            <Label>
              <span className="min-w-0 flex-1">{translateUi("Include stopped and expired threads")}</span>
              <Switch
                checked={history}
                onCheckedChange={(value) => setHistory(value === true)}
              />
            </Label>
          </GroupedRow>
          {threads.error ? (
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void threads.refetch()}>{translateUi("Retry threads")}</button>
            </GroupedRow>
          ) : threads.isPending ? (
            <GroupedRow title={translateUi("Loading threads...")} />
          ) : rows.length ? (
            <ul aria-label={translateUi("Platform threads")}>
              {rows.map((thread) => (
                <ThreadRow key={thread.id} thread={thread} chat={chat} />
              ))}
            </ul>
          ) : (
            <GroupedRow
              title={history ? translateUi("No recorded threads.") : translateUi("No active threads.")}
            />
          )}
          {threads.hasNextPage ? (
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={threads.isFetchingNextPage}
                onClick={() => void threads.fetchNextPage()}
              >{translateUi("Show more threads")}</button>
            </GroupedRow>
          ) : null}
        </>
      ) : null}
    </GroupedList>
  );
}
