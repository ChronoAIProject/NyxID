import { ChannelThreads } from "./nyxbot-channel-threads";
import { useState } from "react";
import { ChevronRight, Megaphone, User, Users } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useNyxBotChannelChats,
  useSetNyxBotPrivateChats,
  useUpdateNyxBotChannelChat,
} from "@/hooks/use-nyxbot-agents";
import { agentTitle, channelChatTitle } from "@/lib/assistant/nyxbot-labels";
import { cn, formatRelativeTime } from "@/lib/utils";
import type {
  AssistantAgent,
  NyxAgentChannelAgent,
  NyxAgentChannelChat,
  NyxAgentChannelChatSettings,
} from "@/schemas/assistant-nyxagent";

/** Chats shown before "Show more". */
const CHATS_SHOWN = 8;

/** A private chat, group or channel, by icon; nothing for unknown kinds. */
export function ChatKindIcon({
  kind,
  className,
}: {
  readonly kind: string | null | undefined;
  readonly className?: string;
}) {
  if (kind === "private") return <User aria-hidden="true" className={className} />;
  if (kind === "group") return <Users aria-hidden="true" className={className} />;
  if (kind === "channel") return <Megaphone aria-hidden="true" className={className} />;
  return null;
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function SettingSelect<T extends string>({
  label,
  value,
  options,
  disabled,
  onChange,
}: {
  readonly label: string;
  readonly value: T;
  readonly options: readonly { readonly value: T; readonly label: string }[];
  readonly disabled: boolean;
  readonly onChange: (value: T) => void;
}) {
  return (
    <Select value={value} onValueChange={(next) => onChange(next as T)} disabled={disabled}>
      <SelectTrigger aria-label={label} className="h-7 w-full max-w-[220px] rounded-md px-2 text-12">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => (
          <SelectItem key={option.value} value={option.value}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function ChatRow({
  channel,
  chat,
  agents,
  botAgentName,
  onError,
}: {
  readonly channel: NyxAgentChannelAgent;
  readonly chat: NyxAgentChannelChat;
  readonly agents: readonly AssistantAgent[];
  readonly botAgentName: string;
  readonly onError: (message: string | undefined) => void;
}) {
  const update = useUpdateNyxBotChannelChat();
  const [notice, setNotice] = useState<string>();
  const title = channelChatTitle(chat.kind, chat.title, channel.platform);
  const group = chat.kind !== null && chat.kind !== "private";

  async function change(settings: NyxAgentChannelChatSettings) {
    onError(undefined);
    setNotice(undefined);
    try {
      const result = await update.mutateAsync({
        channelAgentId: channel.id,
        chatId: chat.id,
        settings,
      });
      setNotice(result.warning ?? result.note);
    } catch (cause) {
      onError(errorMessage(cause, `Could not change ${title}. Try again.`));
    }
  }

  const kindLabel =
    chat.kind === "private" ? "Private chat" : chat.kind === "channel" ? "Channel" : "Group";
  const summary = group
    ? `${chat.reply_mode === "all" ? "Answers every message" : "Answers when mentioned or replied to"} · ${
        chat.members === "everyone"
          ? "everyone here can talk to it, as guests"
          : chat.members_setting === "owner"
            ? "only you can talk to it"
            : "only you until you talk to the bot here"
      }`
    : chat.owner
      ? "Your private chat with the bot"
      : `A private chat with ${title}, as a guest`;

  return (
    <div className="space-y-2 rounded-md bg-overlay px-2.5 py-2">
      <div className="flex items-center gap-2">
        <ChatKindIcon kind={chat.kind} className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
        <p className="min-w-0 flex-1 truncate text-12 font-medium text-foreground">{title}</p>
        {chat.kind ? (
          <span className="shrink-0 rounded-md border border-hairline px-1 text-10 text-text-tertiary">
            {kindLabel}
          </span>
        ) : null}
        {chat.last_message_at ? (
          <span className="shrink-0 text-11 text-text-tertiary">
            {formatRelativeTime(chat.last_message_at)}
          </span>
        ) : null}
      </div>
      <p className="text-11 text-muted-foreground">{summary}</p>
      <div className="grid grid-cols-[auto_1fr] items-center gap-x-3 gap-y-1.5 text-11 text-muted-foreground">
        {group ? (
          <>
            <span>Answers</span>
            <SettingSelect
              label={`Replies in ${title}`}
              value={chat.reply_mode}
              disabled={update.isPending}
              options={[
                { value: "mention", label: "When mentioned" },
                { value: "all", label: "Every message" },
              ]}
              onChange={(reply_mode) => void change({ reply_mode })}
            />
            {chat.thread_capabilities?.thread_follow ? <>
              <span>Threads</span>
              <SettingSelect label={`Thread follow in ${title}`} value={chat.threads ?? "off"} disabled={update.isPending}
                options={[{ value: "follow", label: "Follow when addressed" }, { value: "off", label: "Off" }]}
                onChange={(threads) => void change({ threads })} />
            </> : null}
            <span>Who can talk</span>
            <SettingSelect
              label={`Who can talk in ${title}`}
              value={chat.members_setting ?? "default"}
              disabled={update.isPending}
              options={[
                {
                  value: "default",
                  label: chat.owner_seen ? "Everyone (you talk here)" : "You, until you talk here",
                },
                { value: "everyone", label: "Everyone there" },
                { value: "owner", label: "Only you" },
              ]}
              onChange={(members) => void change({ members })}
            />
          </>
        ) : null}
        <span>Answered by</span>
        <SettingSelect
          label={`Agent for ${title}`}
          value={chat.agent_id ?? "default"}
          disabled={update.isPending}
          options={[
            { value: "default", label: `${botAgentName} (the bot's agent)` },
            ...agents.map((agent) => ({ value: agent.id, label: agentTitle(agent) })),
          ]}
          onChange={(agent_id) => void change({ agent_id })}
        />
        <span>Posts on its own</span>
        <Switch
          checked={chat.allow_posts}
          disabled={update.isPending}
          aria-label={`Let the agent post in ${title} on its own`}
          onCheckedChange={(allow_posts) => void change({ allow_posts })}
        />
      </div>
      {group && chat.follow_guidance ? <p className="text-11 text-muted-foreground">{chat.follow_guidance}</p> : null}
      {group && (chat.thread_capabilities?.thread_follow || chat.has_thread_history || (chat.followed_thread_count ?? 0) > 0) ? <ChannelThreads chat={chat} /> : null}
      {notice ? <p className="text-11 text-muted-foreground">{notice}</p> : null}
    </div>
  );
}

/**
 * The chats a connected bot is in, with who may talk to its agent. Loaded
 * only once opened, since a busy bot can be in many chats.
 */
export function ChannelChats({
  channel,
  agents,
  botAgentName,
}: {
  readonly channel: NyxAgentChannelAgent;
  readonly agents: readonly AssistantAgent[];
  readonly botAgentName: string;
}) {
  const [open, setOpen] = useState(false);
  const [shown, setShown] = useState(CHATS_SHOWN);
  const [error, setError] = useState<string>();
  const chats = useNyxBotChannelChats(channel.id, open);
  const access = useSetNyxBotPrivateChats();
  const rows = chats.data ?? [];
  const roots = rows.filter((chat) => !chat.parent_chat_id || !rows.some((parent) => parent.id === chat.parent_chat_id));

  async function setAccess(privateChats: "owner" | "everyone") {
    setError(undefined);
    try {
      await access.mutateAsync({ channelAgentId: channel.id, privateChats });
    } catch (cause) {
      setError(errorMessage(cause, "Could not change who can talk in private chats."));
    }
  }

  return (
    <div className="space-y-2">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex items-center gap-1 text-12 text-muted-foreground transition-colors hover:text-foreground"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn("h-3 w-3 transition-transform", open && "rotate-90")}
        />
        Chats and who can talk
      </button>
      {open ? (
        <div className="space-y-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-11 text-muted-foreground">Private chats with the bot</span>
            <SettingSelect
              label={`Who can talk to ${channel.bot_label} in private chats`}
              value={channel.private_chats}
              disabled={access.isPending}
              options={[
                { value: "owner", label: "Only you" },
                { value: "everyone", label: "Anyone" },
              ]}
              onChange={(value) => void setAccess(value)}
            />
          </div>
          <p className="text-11 text-text-tertiary">
            People other than you talk to the agent as guests: it never acts on your account for
            them, NyxBot uses none of your services for them, and a chat&apos;s own specialist
            only reads with its services. Members of a group can talk to it once you have talked
            to the bot there. In groups it answers when mentioned or replied to unless set to
            every message.
          </p>
          {error ? (
            <p role="alert" className="text-11 text-destructive">
              {error}
            </p>
          ) : null}
          {chats.error ? (
            <ErrorBanner
              message={`Could not load chats. ${chats.error.message}`}
              onRetry={() => void chats.refetch()}
            />
          ) : chats.isPending ? (
            <p className="text-11 text-text-tertiary">Loading chats...</p>
          ) : rows.length ? (
            <ul aria-label={`Chats of ${channel.bot_label}`} className="space-y-1.5">
              {roots.slice(0, shown).map((chat) => (
                <li key={chat.id}>
                <ChatRow
                  key={chat.id}
                  channel={channel}
                  chat={chat}
                  agents={agents}
                  botAgentName={botAgentName}
                  onError={setError}
                />
                {rows.filter((child) => child.parent_chat_id === chat.id).map((child) => <div key={child.id} className="ml-3 mt-1 border-l border-hairline pl-2">
                  <p className="mb-1 text-11 text-text-tertiary">Earlier thread settings</p>
                  <ChatRow channel={channel} chat={child} agents={agents} botAgentName={botAgentName} onError={setError} />
                </div>)}
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-11 text-text-tertiary">
              No chats yet. Message the bot, or add it to a group and mention it.
            </p>
          )}
          {roots.length > shown ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setShown((value) => value + CHATS_SHOWN * 2)}
            >
              Show {Math.min(roots.length - shown, CHATS_SHOWN * 2)} more
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
