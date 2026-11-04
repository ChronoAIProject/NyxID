import { useDrawerSearchEntries, useDrawerSearchEnabled } from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger,
  GroupedDisclosure,
} from "@/components/ui/grouped-list";
import { GuestReadiness } from "./guest-readiness";
import { ChannelThreads } from "./nyxbot-channel-threads";
import { useState } from "react";
import { Megaphone, User, Users } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import {
  useGuestReadiness,
  useNyxBotChannelChats,
  useSetNyxBotPrivateChats,
  useSetNyxBotPrivateChatPosts,
  useUpdateNyxBotChannelChat,
} from "@/hooks/use-nyxbot-agents";
import { agentTitle, channelChatTitle } from "@/lib/assistant/nyxbot-labels";
import { formatRelativeTime } from "@/lib/utils";
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
    <Select
      value={value}
      onValueChange={(next) => onChange(next as T)}
      disabled={disabled}
    >
      <GroupedSelectTrigger aria-label={label} className="max-w-full">
        <SelectValue />
      </GroupedSelectTrigger>
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
  const agentId = chat.agent_id ?? channel.agent_id ?? agents.find((a) => a.kind === "nyxbot")?.id;
  const agentReadiness = useGuestReadiness(agentId, true);
  const scoped = Boolean(chat.conversation_id && (
    agentReadiness.data?.conversation_overrides_truncated ||
    agentReadiness.data?.conversation_overrides.includes(chat.conversation_id)
  ));
  const chatReadiness = useGuestReadiness(agentId, scoped, chat.conversation_id ?? undefined);
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
    <div className="space-y-3">
      <GroupedList
        data-drawer-search-target={`channel-chat-${chat.id}`}
        info="Control who answers in this chat, who may talk to the bot and whether it may post on its own. Guest service permissions still apply."
        title={`${title} · ${kindLabel}${chat.last_message_at ? ` · ${formatRelativeTime(chat.last_message_at)}` : ""}`}
        footer={
          <>
            <p>{summary}</p>
            <GuestReadiness
              readiness={scoped ? chatReadiness.data : agentReadiness.data}
            />
            {group && chat.follow_guidance ? (
              <p>{chat.follow_guidance}</p>
            ) : null}
            {notice ? <p role="status">{notice}</p> : null}
          </>
        }
      >
        {group ? (
          <>
            <GroupedRow
              title="Answers"
              accessory={
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
              }
            />
            {chat.thread_capabilities?.thread_follow ? (
              <GroupedRow
                title="Threads"
                accessory={
                  <SettingSelect
                    label={`Thread follow in ${title}`}
                    value={chat.threads ?? "off"}
                    disabled={update.isPending}
                    options={[
                      { value: "follow", label: "Follow when addressed" },
                      { value: "off", label: "Off" },
                    ]}
                    onChange={(threads) => void change({ threads })}
                  />
                }
              />
            ) : null}
            <GroupedRow
              title="Who can talk"
              accessory={
                <SettingSelect
                  label={`Who can talk in ${title}`}
                  value={chat.members_setting ?? "default"}
                  disabled={update.isPending}
                  options={[
                    {
                      value: "default",
                      label: chat.owner_seen
                        ? "Everyone (you talk here)"
                        : "You, until you talk here",
                    },
                    { value: "everyone", label: "Everyone there" },
                    { value: "owner", label: "Only you" },
                  ]}
                  onChange={(members) => void change({ members })}
                />
              }
            />
          </>
        ) : null}
        <GroupedRow
          title="Answered by"
          accessory={
            <SettingSelect
              label={`Agent for ${title}`}
              value={chat.agent_id ?? "default"}
              disabled={update.isPending}
              options={[
                {
                  value: "default",
                  label: `${botAgentName} (the bot's agent)`,
                },
                ...agents.map((agent) => ({
                  value: agent.id,
                  label: agentTitle(agent),
                })),
              ]}
              onChange={(agent_id) => void change({ agent_id })}
            />
          }
        />
        <GroupedRow
          title="Posts on its own"
          accessory={
            <Switch
              checked={chat.allow_posts}
              disabled={update.isPending}
              aria-label={`Let the agent post in ${title} on its own`}
              onCheckedChange={(allow_posts) => void change({ allow_posts })}
            />
          }
        />
      </GroupedList>
      {group &&
      (chat.thread_capabilities?.thread_follow ||
        chat.has_thread_history ||
        (chat.followed_thread_count ?? 0) > 0) ? (
        <ChannelThreads chat={chat} />
      ) : null}
    </div>
  );
}

/**
 * The chats a connected bot is in, with who may talk to its agent. Loaded
 * on opening, or while indexed by the agent drawer search.
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
  const inDrawer = useDrawerSearchEnabled();
  const chats = useNyxBotChannelChats(channel.id, open || inDrawer);
  const agentId = channel.agent_id ?? agents.find((a) => a.kind === "nyxbot")?.id;
  const readiness = useGuestReadiness(agentId, open);
  const access = useSetNyxBotPrivateChats();
  const posts = useSetNyxBotPrivateChatPosts();
  const [applyToExisting, setApplyToExisting] = useState(false);
  const rows = chats.data ?? [];
  const roots = rows.filter((chat) => !chat.parent_chat_id || !rows.some((parent) => parent.id === chat.parent_chat_id));

  const searchId = `channel-chats-${channel.id}`;
  useDrawerSearchEntries([
    { id: searchId, title: "Chats and who can talk", breadcrumb: `Channels › ${channel.bot_label}`, keywords: "Private chats posts members", pages: [searchId], reveal: () => setOpen(true) },
    ...rows.map((chat) => ({ id: `channel-chat-${chat.id}`, title: channelChatTitle(chat.kind, chat.title, channel.platform),
      breadcrumb: `Channels › ${channel.bot_label} › Chats`, keywords: `${chat.kind ?? ""} ${chat.members ?? ""} Answered by Posts on its own`,
      pages: [searchId], reveal: () => { setShown(rows.length); setOpen(true); },
    })),
  ]);
  async function setAccess(privateChats: "owner" | "everyone") {
    setError(undefined);
    try {
      await access.mutateAsync({ channelAgentId: channel.id, privateChats });
    } catch (cause) {
      setError(errorMessage(cause, "Could not change who can talk in private chats."));
    }
  }

  async function setPosts(enabled: boolean) {
    setError(undefined);
    try {
      await posts.mutateAsync({ channelAgentId: channel.id, enabled, applyToExisting });
      setApplyToExisting(false);
    } catch (cause) {
      setError(errorMessage(cause, "Could not change private chat posting."));
    }
  }

  return (
    <>
      <GroupedDisclosure data-drawer-page-trigger={searchId} onClick={() => setOpen(true)}>
        Chats and who can talk
      </GroupedDisclosure>
      <GroupedDetail
        searchPage={searchId}
        open={open}
        title={`${channel.bot_label} · Chats`}
        info="Manage private-chat access and the bot’s behavior in each connected chat. Changes to these controls apply immediately."
        onReveal={() => setOpen(true)}
        backLabel="Back to Channels"
        onBack={() => setOpen(false)}
      >
        <GroupedList
          title="Private chats"
          data-drawer-search-target={searchId}
          footer={
            <>
              <p>
                People other than you talk to the agent as guests: it never acts
                on your account for them, NyxBot uses none of your services for
                them, and a chat’s own specialist only reads with its services.
                Members of a group can talk to it once you have talked to the
                bot there. In groups it answers when mentioned or replied to
                unless set to every message.
              </p>
              <p>
                The agent can send updates after a customer first messages this
                bot. Existing chats keep their settings; you can still turn
                posting off for individual chats. Groups and topics need their
                own permission. This does not change who can talk to the bot.
              </p>
              {applyToExisting ? (
                <p>
                  Applies the current setting to up to 1,000 changed private
                  chats, including individual overrides.
                </p>
              ) : null}
              <GuestReadiness readiness={readiness.data} />
              {error ? (
                <p role="alert" className="text-destructive">
                  {error}
                </p>
              ) : null}
            </>
          }
        >
          <GroupedRow
            title="Private chats with the bot"
            accessory={
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
            }
          />
          <GroupedRow
            title="Allow posts in new private chats"
            accessory={
              <Switch
                aria-label={`Allow posts in new private chats with ${channel.bot_label}`}
                checked={channel.private_chat_posts}
                disabled={posts.isPending}
                onCheckedChange={(value) => void setPosts(value)}
              />
            }
          />
          <GroupedRow asChild interactive>
            <label>
              <span className="min-w-0 flex-1">
                Also apply to existing private chats
              </span>
              <Switch
                checked={applyToExisting}
                disabled={posts.isPending}
                onCheckedChange={(checked) =>
                  setApplyToExisting(checked === true)
                }
              />
            </label>
          </GroupedRow>
          {applyToExisting ? (
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={posts.isPending}
                onClick={() => void setPosts(channel.private_chat_posts)}
              >
                Apply current setting
              </button>
            </GroupedRow>
          ) : null}
        </GroupedList>
        {chats.error ? (
          <GroupedList
            footer={
              <p role="alert" className="text-destructive">
                Could not load chats. {chats.error.message}
              </p>
            }
          >
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void chats.refetch()}>
                Retry chats
              </button>
            </GroupedRow>
          </GroupedList>
        ) : chats.isPending ? (
          <GroupedList>
            <GroupedRow title="Loading chats..." />
          </GroupedList>
        ) : rows.length ? (
          <ul
            aria-label={`Chats of ${channel.bot_label}`}
            className="space-y-6"
          >
            {roots.slice(0, shown).map((chat) => (
              <li key={chat.id} className="space-y-6">
                <ChatRow
                  channel={channel}
                  chat={chat}
                  agents={agents}
                  botAgentName={botAgentName}
                  onError={setError}
                />
                {rows
                  .filter((child) => child.parent_chat_id === chat.id)
                  .map((child) => (
                    <ChatRow
                      key={child.id}
                      channel={channel}
                      chat={child}
                      agents={agents}
                      botAgentName={botAgentName}
                      onError={setError}
                    />
                  ))}
              </li>
            ))}
          </ul>
        ) : (
          <GroupedList>
            <GroupedRow title="No chats yet." />
            <GroupedRow title="Message the bot, or add it to a group and mention it." />
          </GroupedList>
        )}
        {roots.length > shown ? (
          <GroupedList>
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() => setShown((value) => value + CHATS_SHOWN * 2)}
              >
                Show {Math.min(roots.length - shown, CHATS_SHOWN * 2)} more
              </button>
            </GroupedRow>
          </GroupedList>
        ) : null}
      </GroupedDetail>
    </>
  );
}
