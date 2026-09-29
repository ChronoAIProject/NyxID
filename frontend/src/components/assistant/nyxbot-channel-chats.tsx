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
      <SelectTrigger aria-label={label} className="h-7 w-[150px] rounded-md px-2 text-[12px]">
        <SelectValue />
      </SelectTrigger>
      <SelectContent className="z-[90]">
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

  return (
    <li className="space-y-2 rounded-md bg-overlay px-2.5 py-2">
      <div className="flex items-center gap-2">
        <ChatKindIcon kind={chat.kind} className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
        <p className="min-w-0 flex-1 truncate text-[12px] font-medium text-foreground">{title}</p>
        {chat.last_message_at ? (
          <span className="shrink-0 text-[11px] text-text-tertiary">
            {formatRelativeTime(chat.last_message_at)}
          </span>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        {group ? (
          <>
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
            <SettingSelect
              label={`Who can talk in ${title}`}
              value={chat.members}
              disabled={update.isPending}
              options={[
                { value: "everyone", label: "Everyone there" },
                { value: "owner", label: "Only you" },
              ]}
              onChange={(members) => void change({ members })}
            />
          </>
        ) : null}
        <SettingSelect
          label={`Agent for ${title}`}
          value={chat.agent_id ?? "default"}
          disabled={update.isPending}
          options={[
            { value: "default", label: `Bot's agent (${botAgentName})` },
            ...agents.map((agent) => ({ value: agent.id, label: agentTitle(agent) })),
          ]}
          onChange={(agent_id) => void change({ agent_id })}
        />
        <label className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <Switch
            checked={chat.allow_posts}
            disabled={update.isPending}
            aria-label={`Let the agent post in ${title} on its own`}
            onCheckedChange={(allow_posts) => void change({ allow_posts })}
          />
          Posts on its own
        </label>
      </div>
      {notice ? <p className="text-[11px] text-muted-foreground">{notice}</p> : null}
    </li>
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
        className="flex items-center gap-1 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
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
            <span className="text-[11px] text-muted-foreground">Private chats with the bot</span>
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
          <p className="text-[11px] text-text-tertiary">
            People other than you talk to the agent as guests: it never acts on your account for
            them, NyxBot uses none of your services for them, and a chat&apos;s own specialist
            only reads with its services. Members of a group can talk to it once you have talked
            to the bot there. In groups it answers when mentioned or replied to unless set to
            every message.
          </p>
          {error ? (
            <p role="alert" className="text-[11px] text-destructive">
              {error}
            </p>
          ) : null}
          {chats.error ? (
            <ErrorBanner
              message={`Could not load chats. ${chats.error.message}`}
              onRetry={() => void chats.refetch()}
            />
          ) : chats.isPending ? (
            <p className="text-[11px] text-text-tertiary">Loading chats...</p>
          ) : rows.length ? (
            <ul aria-label={`Chats of ${channel.bot_label}`} className="space-y-1.5">
              {rows.slice(0, shown).map((chat) => (
                <ChatRow
                  key={chat.id}
                  channel={channel}
                  chat={chat}
                  agents={agents}
                  botAgentName={botAgentName}
                  onError={setError}
                />
              ))}
            </ul>
          ) : (
            <p className="text-[11px] text-text-tertiary">
              No chats yet. Message the bot, or add it to a group and mention it.
            </p>
          )}
          {rows.length > shown ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setShown((value) => value + CHATS_SHOWN * 2)}
            >
              Show {Math.min(rows.length - shown, CHATS_SHOWN * 2)} more
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
