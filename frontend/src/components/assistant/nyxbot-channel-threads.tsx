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
                className="min-w-0 flex-1 break-words text-12 hover:underline"
              >
                {thread.label}
              </Link>
            ) : (
              <span className="flex-1 text-12">{thread.label}</span>
            )}
            <span className="text-11 font-normal text-text-tertiary">
              {thread.state === "active" && thread.follow_readiness !== "ready"
                ? "Unavailable"
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
              >
                Stop following
              </Button>
            ) : null}
          </div>
          <p className="text-11 font-normal text-text-tertiary">
            {thread.last_admitted_at
              ? `Last activity ${formatRelativeTime(thread.last_admitted_at)}. `
              : ""}
            {thread.state === "active" && thread.expires_at
              ? `Idle expiry ${new Date(thread.expires_at).toLocaleString()}. `
              : ""}
          </p>
        </li>
      </GroupedRow>
      <GroupedFooter>
        <p>
          {(thread.dropped_message_count ?? 0) > 0
            ? `${thread.dropped_message_count} messages arrived while busy and were not queued. `
            : ""}
          {thread.kind === "topic" ? "Follows the whole topic. " : ""}
          {thread.context_status === "metadata_only"
            ? "Earlier message bodies were unavailable."
            : thread.context_status === "partial"
              ? "Earlier context is partial."
              : ""}
        </p>
        {stop.error ? (
          <p role="alert" className="text-11 text-destructive">
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
  const [open, setOpen] = useState(false);
  const [history, setHistory] = useState(false);
  const threads = useNyxBotChannelThreads(chat.channel_agent_id, chat.id, history ? "all" : "active", open);
  const rows = threads.data?.pages.flatMap((page) => page.threads) ?? [];
  return (
    <GroupedList
      title="Threads"
      footer={
        <>
          {chat.reply_mode === "all" ? (
            <p>Every message still allows replies after you stop following.</p>
          ) : null}
          {threads.error ? (
            <p role="alert" className="text-destructive">
              {threads.error.message}
            </p>
          ) : null}
        </>
      }
    >
      <GroupedDisclosure aria-expanded={open} onClick={() => setOpen(!open)}>
        Followed threads ({chat.followed_thread_count ?? 0})
      </GroupedDisclosure>
      {chat.conversation_id ? (
        <GroupedRow asChild interactive>
          <Link to="/assistant" search={{ c: chat.conversation_id }}>
            Chat history
          </Link>
        </GroupedRow>
      ) : null}
      {open ? (
        <>
          <GroupedRow asChild interactive>
            <label>
              <span className="min-w-0 flex-1">
                Include stopped and expired threads
              </span>
              <Switch
                checked={history}
                onCheckedChange={(value) => setHistory(value === true)}
              />
            </label>
          </GroupedRow>
          {threads.error ? (
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void threads.refetch()}>
                Retry threads
              </button>
            </GroupedRow>
          ) : threads.isPending ? (
            <GroupedRow title="Loading threads..." />
          ) : rows.length ? (
            <ul aria-label="Platform threads">
              {rows.map((thread) => (
                <ThreadRow key={thread.id} thread={thread} chat={chat} />
              ))}
            </ul>
          ) : (
            <GroupedRow
              title={`No ${history ? "recorded" : "active"} threads.`}
            />
          )}
          {threads.hasNextPage ? (
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={threads.isFetchingNextPage}
                onClick={() => void threads.fetchNextPage()}
              >
                Show more threads
              </button>
            </GroupedRow>
          ) : null}
        </>
      ) : null}
    </GroupedList>
  );
}
