import { translateErrorMessage } from "@/lib/error-messages";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { useDrawerSearchEntries, useDrawerSearchEnabled } from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger,
  GroupedDisclosure,
} from "@/components/ui/grouped-list";
import { GuestReadiness } from "./guest-readiness";
import { botAudienceDescription } from "@/lib/assistant/bot-audience";
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
  useUiTranslation();
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
  useUiTranslation();
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
  useUiTranslation();
  const agentId =
    chat.agent_id ??
    channel.agent_id ??
    agents.find((a) => a.kind === "nyxbot")?.id;
  const orgSpecialist = Boolean(channel.org_specialist);
  const agentReadiness = useGuestReadiness(agentId, !orgSpecialist);
  const scoped = Boolean(
    chat.conversation_id &&
    (agentReadiness.data?.conversation_overrides_truncated ||
      agentReadiness.data?.conversation_overrides.includes(
        chat.conversation_id,
      )),
  );
  const chatReadiness = useGuestReadiness(
    agentId,
    scoped && !orgSpecialist,
    chat.conversation_id ?? undefined,
  );
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
    chat.kind === "private"
      ? "Private chat"
      : chat.kind === "channel"
        ? "Channel"
        : "Group";
  const summary = orgSpecialist
    ? group
      ? `${chat.reply_mode === "all" ? "Answers every message" : "Answers when mentioned or replied to"} · verified organization members${chat.members === "everyone" ? " and enabled guests" : " only"}`
      : "Private conversation with the organization specialist"
    : group
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
            {!orgSpecialist ? <GuestReadiness
              readiness={scoped ? chatReadiness.data : agentReadiness.data}
            /> : null}
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
              title={translateUi("Answers")}
              accessory={
                <SettingSelect
                  label={translateUi("Replies in {{value1}}", { value1: title })}
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
                title={translateUi("Threads")}
                accessory={
                  <SettingSelect
                    label={translateUi("Thread follow in {{value1}}", { value1: title })}
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
              title={translateUi("Who can talk")}
              accessory={
                <SettingSelect
                  label={translateUi("Who can talk in {{value1}}", { value1: title })}
                  value={chat.members_setting ?? "default"}
                  disabled={update.isPending}
                  options={[
                    {
                      value: "default",
                      label: orgSpecialist
                        ? chat.owner_seen
                          ? "Members and enabled guests (admin present)"
                          : "Members, until an admin talks here"
                        : chat.owner_seen
                          ? "Everyone (you talk here)"
                          : "You, until you talk here",
                    },
                    { value: "everyone", label: orgSpecialist ? "Members and enabled guests" : "Everyone there" },
                    { value: "owner", label: orgSpecialist ? "Verified organization members" : "Only you" },
                  ]}
                  onChange={(members) => void change({ members })}
                />
              }
            />
          </>
        ) : null}
        <GroupedRow
          title={translateUi("Answered by")}
          accessory={
            <SettingSelect
              label={translateUi("Agent for {{value1}}", { value1: title })}
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
          title={translateUi("Posts on its own")}
          accessory={
            <Switch
              checked={chat.allow_posts}
              disabled={update.isPending}
              aria-label={translateUi("Let the agent post in {{value1}} on its own", { value1: title })}
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
  useUiTranslation();
  const [open, setOpen] = useState(false);
  const [shown, setShown] = useState(CHATS_SHOWN);
  const [error, setError] = useState<string>();
  const inDrawer = useDrawerSearchEnabled();
  const chats = useNyxBotChannelChats(channel.id, open || inDrawer);
  const agentId = channel.agent_id ?? agents.find((a) => a.kind === "nyxbot")?.id;
  const readiness = useGuestReadiness(agentId, open && !channel.org_specialist);
  const posts = useSetNyxBotPrivateChatPosts();
  const [applyToExisting, setApplyToExisting] = useState(false);
  const rows = chats.data ?? [];
  const roots = rows.filter(
    (chat) =>
      !chat.parent_chat_id ||
      !rows.some((parent) => parent.id === chat.parent_chat_id),
  );

  const searchId = `channel-chats-${channel.id}`;
  useDrawerSearchEntries([
    { id: searchId, title: "Chats and posting", breadcrumb: `Channels › ${channel.bot_label}`, keywords: "Private chats posts members", pages: [searchId], reveal: () => setOpen(true) },
    ...rows.map((chat) => ({ id: `channel-chat-${chat.id}`, title: channelChatTitle(chat.kind, chat.title, channel.platform),
      breadcrumb: `Channels › ${channel.bot_label} › Chats`, keywords: `${chat.kind ?? ""} ${chat.members ?? ""} Answered by Posts on its own`,
      pages: [searchId], reveal: () => { setShown(rows.length); setOpen(true); },
    })),
  ]);
  async function setPosts(enabled: boolean) {
    setError(undefined);
    try {
      await posts.mutateAsync({
        channelAgentId: channel.id,
        enabled,
        applyToExisting,
      });
      setApplyToExisting(false);
    } catch (cause) {
      setError(errorMessage(cause, "Could not change private chat posting."));
    }
  }

  return (
    <>
      <GroupedDisclosure data-drawer-page-trigger={searchId} onClick={() => setOpen(true)}>{translateUi("Chats and posting")}</GroupedDisclosure>
      <GroupedDetail
        searchPage={searchId}
        open={open}
        title={translateUi("{{value1}} · Chats", { value1: channel.bot_label })}
        info="Manage posting and the bot’s behavior in each connected chat. Changes to these controls apply immediately."
        onReveal={() => setOpen(true)}
        backLabel={translateUi("Back to Channels")}
        onBack={() => setOpen(false)}
      >
        <GroupedList
          title={translateUi("Private chats")}
          data-drawer-search-target={searchId}
          footer={
            <>
              <p>{botAudienceDescription(channel)}</p>
              <p>{channel.org_specialist
                ? translateUi("An admin talking in a group opens its default audience to enabled guests; other members cannot widen it.")
                : translateUi("Members of a group can talk to the bot once you have talked to it there. In groups it answers when mentioned or replied to unless set to every message.")}</p>
              <p>{translateUi("The agent can send updates after a customer first messages this bot. Existing chats keep their settings; you can still turn posting off for individual chats. Groups and topics need their own permission. This does not change who can talk to the bot.")}</p>
              {applyToExisting ? (
                <p>{translateUi("Applies the current setting to up to 1,000 changed private chats, including individual overrides.")}</p>
              ) : null}
              {!channel.org_specialist ? <GuestReadiness readiness={readiness.data} /> : null}
              {error ? (
                <p role="alert" className="text-destructive">
                  {translateUi(error)}
                </p>
              ) : null}
            </>
          }
        >
          <GroupedRow
            title={translateUi("Allow posts in new private chats")}
            accessory={
              <Switch
                aria-label={translateUi("Allow posts in new private chats with {{value1}}", { value1: channel.bot_label })}
                checked={channel.private_chat_posts}
                disabled={posts.isPending}
                onCheckedChange={(value) => void setPosts(value)}
              />
            }
          />
          <GroupedRow asChild interactive>
            <Label>
              <span className="min-w-0 flex-1">{translateUi("Also apply to existing private chats")}</span>
              <Switch
                checked={applyToExisting}
                disabled={posts.isPending}
                onCheckedChange={(checked) =>
                  setApplyToExisting(checked === true)
                }
              />
            </Label>
          </GroupedRow>
          {applyToExisting ? (
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={posts.isPending}
                onClick={() => void setPosts(channel.private_chat_posts)}
              >{translateUi("Apply current setting")}</button>
            </GroupedRow>
          ) : null}
        </GroupedList>
        {chats.error ? (
          <GroupedList
            footer={
              <p role="alert" className="text-destructive">{translateUi("Could not load chats. ")}{translateErrorMessage(chats.error.message)}
              </p>
            }
          >
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void chats.refetch()}>{translateUi("Retry chats")}</button>
            </GroupedRow>
          </GroupedList>
        ) : chats.isPending ? (
          <GroupedList>
            <GroupedRow title={translateUi("Loading chats...")} />
          </GroupedList>
        ) : rows.length ? (
          <ul
            aria-label={translateUi("Chats of {{value1}}", { value1: channel.bot_label })}
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
            <GroupedRow title={translateUi("No chats yet.")} />
            <GroupedRow title={translateUi("Message the bot, or add it to a group and mention it.")} />
          </GroupedList>
        )}
        {roots.length > shown ? (
          <GroupedList>
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() => setShown((value) => value + CHATS_SHOWN * 2)}
              >{translateUi("Show ")}{Math.min(roots.length - shown, CHATS_SHOWN * 2)}{translateUi(" more")}</button>
            </GroupedRow>
          </GroupedList>
        ) : null}
      </GroupedDetail>
    </>
  );
}
