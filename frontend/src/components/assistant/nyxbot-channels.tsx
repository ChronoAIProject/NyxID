import { useDrawerSearchEntries } from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger,
  GroupedDisclosure,
  GroupedInput,
} from "@/components/ui/grouped-list";
import { useState } from "react";
import { ExternalLink } from "lucide-react";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { CopyableField } from "@/components/shared/copyable-field";
import { ChannelChats } from "@/components/assistant/nyxbot-channel-chats";
import { useChannelBots } from "@/hooks/use-channel-bots";
import { useOrgs } from "@/hooks/use-orgs";
import { useAuthStore } from "@/stores/auth-store";
import {
  nyxBotOf,
  useConnectNyxBotChannel,
  useDisconnectNyxBotChannel,
  useLinkNyxBotChannel,
  useNyxBotChannels,
} from "@/hooks/use-nyxbot-agents";
import { agentTitle, channelPlatformName } from "@/lib/assistant/nyxbot-labels";
import { nyxBotChannelError, nyxBotChannelMutationError } from "@/lib/assistant/nyxbot-channel-errors";
import { formatDateTime, formatRelativeTime } from "@/lib/utils";
import type {
  AssistantAgent,
  NyxAgentChannelAgent,
  NyxAgentChannelLink,
} from "@/schemas/assistant-nyxagent";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message
    ? nyxBotChannelMutationError(error.message)
    : fallback;
}

const channelStatus: Record<string, string> = {
  active: "Active",
  pending: "Pending",
  failed: "Failed",
};

function agentLabel(agent: AssistantAgent): string {
  return agentTitle(agent);
}

function AgentSelect({
  agents,
  value,
  label,
  disabled,
  onChange,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly value: string | undefined;
  readonly label: string;
  readonly disabled: boolean;
  readonly onChange: (agentId: string) => void;
}) {
  return (
    <Select
      value={value}
      onValueChange={onChange}
      disabled={disabled || !agents.length}
    >
      <GroupedSelectTrigger aria-label={label} className="max-w-full">
        <SelectValue placeholder="Choose agent" />
      </GroupedSelectTrigger>
      <SelectContent>
        {agents.map((agent) => (
          <SelectItem key={agent.id} value={agent.id}>
            {agentLabel(agent)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * Channel bots and the agents they reach. Without `agent`, every connected
 * bot is listed with an agent selector (settings). With `agent`, only the bots
 * linked to that agent are listed and new bots connect to it (agent details).
 */
export function ChannelBotsManager({
  agents,
  agent,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly agent?: AssistantAgent;
}) {
  const channels = useNyxBotChannels();
  // The user's bots and those of the organizations they administer.
  const bots = useChannelBots({ scope: "all" });
  const orgs = useOrgs();
  const orgName = (orgId: string | null | undefined) => {
    if (!orgId) return undefined;
    const org = orgs.data?.find((row) => row.id === orgId);
    return org?.display_name?.trim() || org?.slug || "Organization";
  };
  const currentUserId = useAuthStore((state) => state.user?.id);
  const connect = useConnectNyxBotChannel();
  const link = useLinkNyxBotChannel();
  const disconnect = useDisconnectNyxBotChannel();
  const [ownerLink, setOwnerLink] = useState<{
    readonly botId: string;
    readonly agentId?: string;
    readonly label: string;
    readonly link: NyxAgentChannelLink;
  }>();
  const [confirming, setConfirming] = useState<string>();
  const [connectTargets, setConnectTargets] = useState<Record<string, string>>({});
  const [error, setError] = useState<string>();
  const nyxbot = nyxBotOf(agents);
  // Only live personal agents can receive bots or chats; organization specialists are private.
  const targets = agents.filter(
    (row) => row.status !== "destroyed" && row.owner_kind !== "org",
  );
  const agentOf = (row: NyxAgentChannelAgent) => row.agent_id ?? nyxbot?.id;
  const connected = (channels.data ?? []).filter(
    (row) => !agent || agentOf(row) === agent.id,
  );
  const connectedBotIds = new Set((channels.data ?? []).map((row) => row.channel_bot_id));
  const available = (bots.data ?? []).filter((bot) => !connectedBotIds.has(bot.id));
  const busy = connect.isPending || disconnect.isPending || link.isPending;

  async function requestLink(botId: string, agentId: string | undefined) {
    setError(undefined);
    try {
      const result = await connect.mutateAsync({ botId, agentId });
      setOwnerLink({ botId, agentId, label: result.channel_agent.bot_label, link: result.link });
    } catch (cause) {
      setError(errorMessage(cause, "Could not connect this bot. Try again."));
    }
  }

  async function relink(row: NyxAgentChannelAgent, agentId: string) {
    if (row.status === "failed") {
      await requestLink(row.channel_bot_id, agentId);
      return;
    }
    if (agentId === agentOf(row)) return;
    setError(undefined);
    try {
      await link.mutateAsync({ channelAgentId: row.id, agentId });
    } catch (cause) {
      setError(errorMessage(cause, "Could not move this bot to that agent. Try again."));
    }
  }

  async function remove(row: NyxAgentChannelAgent) {
    setError(undefined);
    try {
      await disconnect.mutateAsync(row.id);
      setConfirming(undefined);
      if (ownerLink?.botId === row.channel_bot_id) setOwnerLink(undefined);
    } catch (cause) {
      setError(errorMessage(cause, "Could not disconnect this bot. Try again."));
    }
  }

  const [availableOpen, setAvailableOpen] = useState(false);
  const [search, setSearch] = useState("");
  useDrawerSearchEntries(available.map((bot) => ({ id: `available-bot-${bot.id}`, title: bot.label,
    breadcrumb: "Channels › Available bots", keywords: channelPlatformName(bot.platform), pages: ["available-bots"],
    reveal: () => { setSearch(""); setAvailableOpen(true); },
  })));
  return (
    <div className="space-y-6">
      {channels.error || (error && !availableOpen) ? (
        <GroupedList
          title="Connection status"
          footer={
            <p role="alert" className="text-destructive">
              {error ??
                `Could not load connected bots. ${channels.error?.message}`}
            </p>
          }
        >
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => void channels.refetch()}>
              Refresh connected bots
            </button>
          </GroupedRow>
        </GroupedList>
      ) : null}
      {connected.length ? (
        <ul aria-label="Connected channel bots" className="space-y-6">
          {connected.map((row) => {
            const target = targets.find((candidate) => candidate.id === agentOf(row));
            const live = row.status === "pending" || row.status === "active";
            return (
              <li key={row.id}>
                <GroupedList
                  title="Connected bot"
                  footer={
                    <>
                      {row.last_error ? (
                        <p
                          className="text-11 text-destructive"
                          data-channel-error={row.last_error}
                        >
                          Last error: {nyxBotChannelError(row.last_error)}
                        </p>
                      ) : null}
                      {!row.owner_linked && row.inbound_hint ? (
                        <p>{row.inbound_hint}</p>
                      ) : null}
                      {["failing", "interrupted", "media_failed"].includes(
                        row.delivery_status ?? "",
                      ) ? (
                        <p className="text-11 text-destructive">
                          {row.delivery_status === "failing"
                            ? "Messages are not reaching the agent: "
                            : "Attachment problem: "}
                          {row.delivery_reason ?? "delivery failed"}
                          {row.delivery_failed_at
                            ? ` (${formatRelativeTime(row.delivery_failed_at)})`
                            : ""}
                          .
                        </p>
                      ) : null}
                      {confirming === row.id ? (
                        <p>Stop answering through {row.bot_label}?</p>
                      ) : null}
                    </>
                  }
                >
                  <GroupedRow
                    search={{ id: `connected-bot-${row.id}`, title: row.bot_label, breadcrumb: "Channels › Connected bot", keywords: `${row.bot_username ?? ""} ${channelPlatformName(row.platform)} ${channelStatus[row.status] ?? "Unavailable"} ${row.owner_linked ? "Owner linked" : "Owner not linked"}` }}
                    title={
                      <>
                        {row.bot_label}
                        {row.bot_username ? (
                          <span className="ml-1.5 font-normal text-text-tertiary">
                            @{row.bot_username}
                          </span>
                        ) : null}
                      </>
                    }
                    description={[
                      channelPlatformName(row.platform),
                      row.org_id ? orgName(row.org_id) : undefined,
                      channelStatus[row.status] ?? "Unavailable",
                      row.owner_linked ? "Owner linked" : "Owner not linked",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  />
                  <GroupedRow
                    title="Answered by"
                    accessory={
                      <AgentSelect
                        agents={targets}
                        value={agentOf(row)}
                        label={`Agent for ${row.bot_label}`}
                        disabled={busy}
                        onChange={(agentId) => void relink(row, agentId)}
                      />
                    }
                  />
                  {live ? (
                    <ChannelChats
                      channel={row}
                      agents={targets}
                      botAgentName={(() => {
                        const current = agents.find((candidate) => candidate.id === agentOf(row));
                        return current ? agentLabel(current) : "NyxBot";
                      })()}
                    />
                  ) : null}
                  {row.status === "failed" ? (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={busy || !target}
                        onClick={() =>
                          void requestLink(row.channel_bot_id, target?.id)
                        }
                      >
                        Reconnect
                      </button>
                    </GroupedRow>
                  ) : live ? (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() =>
                          void requestLink(row.channel_bot_id, agentOf(row))
                        }
                      >
                        {row.owner_linked
                          ? "Link another account"
                          : "Link account"}
                      </button>
                    </GroupedRow>
                  ) : null}
                  {confirming === row.id ? (
                    <>
                      <GroupedRow asChild interactive>
                        <button
                          type="button"
                          disabled={disconnect.isPending}
                          onClick={() => setConfirming(undefined)}
                        >
                          Cancel
                        </button>
                      </GroupedRow>
                      <GroupedRow
                        asChild
                        interactive
                        className="text-destructive"
                      >
                        <button
                          type="button"
                          disabled={disconnect.isPending}
                          onClick={() => void remove(row)}
                        >
                          Disconnect
                        </button>
                      </GroupedRow>
                    </>
                  ) : (
                    <GroupedRow
                      asChild
                      interactive
                      className="text-destructive"
                    >
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => setConfirming(row.id)}
                      >
                        Disconnect
                      </button>
                    </GroupedRow>
                  )}
                </GroupedList>
              </li>
            );
          })}
        </ul>
      ) : null}
      {ownerLink && !availableOpen ? (
        <OwnerLink
          label={ownerLink.label}
          link={ownerLink.link}
          busy={busy}
          onRenew={() => void requestLink(ownerLink.botId, ownerLink.agentId)}
        />
      ) : null}
      <GroupedList
        title="Channel bots"
        footer={
          !available.length && !bots.isPending && !bots.error
            ? bots.data?.length
              ? "All your channel bots are connected to other agents. Move one here with its agent selector in NyxBot settings."
              : "You have no channel bots yet. Register one under Channel Bots, then connect it here."
            : undefined
        }
      >
        <GroupedDisclosure data-drawer-page-trigger="available-bots" search={{ id: "connect-another-bot", title: "Connect another bot", breadcrumb: "Channels" }} onClick={() => setAvailableOpen(true)}>
          Connect another bot
        </GroupedDisclosure>
      </GroupedList>
      <GroupedDetail
        searchPage="available-bots"
        open={availableOpen}
        title="Connect another bot"
        onReveal={() => setAvailableOpen(true)}
        backLabel="Back to Channels"
        onBack={() => setAvailableOpen(false)}
      >
        <GroupedList
          title="Available bots"
          footer={
            <>
              {bots.error ? (
                <p role="alert" className="text-destructive">
                  Could not load your channel bots.
                </p>
              ) : null}
              {error && availableOpen ? (
                <p role="alert" className="text-destructive">
                  {error}
                </p>
              ) : null}
            </>
          }
        >
          <GroupedRow>
            <GroupedInput
              aria-label="Search channel bots"
              placeholder="Search bots"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </GroupedRow>
          {bots.isPending ? (
            <GroupedRow title="Loading your channel bots..." />
          ) : bots.error ? (
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void bots.refetch()}>
                Retry channel bots
              </button>
            </GroupedRow>
          ) : null}
          <ul aria-label="Channel bots you can connect">
            {available
              .filter((bot) =>
                `${bot.label} ${channelPlatformName(bot.platform)}`
                  .toLowerCase()
                  .includes(search.toLowerCase()),
              )
              .map((bot) => {
                const target = agent?.id ?? connectTargets[bot.id] ?? nyxbot?.id;
                const targetName = agent ? agentLabel(agent) : undefined;
                return (
                  <GroupedRow data-drawer-search-target={`available-bot-${bot.id}`} key={bot.id} asChild>
                    <li>
                      <div className="min-w-0 flex-1">
                        <p>{bot.label}</p>
                        <p className="text-11 font-normal text-text-tertiary">
                          {channelPlatformName(bot.platform)}
                          {bot.user_id !== currentUserId
                            ? ` · ${orgName(bot.user_id) ?? ""}`
                            : ""}
                        </p>
                      </div>
                      <div className="ml-auto flex max-w-full flex-wrap items-center gap-2">
                        {agent ? null : (
                          <AgentSelect
                            agents={targets}
                            value={target}
                            label={`Agent for ${bot.label}`}
                            disabled={busy}
                            onChange={(agentId) =>
                              setConnectTargets((current) => ({
                                ...current,
                                [bot.id]: agentId,
                              }))
                            }
                          />
                        )}
                        <Button
                          variant="ghost"
                          disabled={busy || !target}
                          aria-label={`Connect ${bot.label}${targetName ? ` to ${targetName}` : ""}`}
                          onClick={() => void requestLink(bot.id, target)}
                        >
                          Connect
                        </Button>
                      </div>
                    </li>
                  </GroupedRow>
                );
              })}
          </ul>
          {!bots.isPending &&
          !bots.error &&
          !available.some((bot) =>
            `${bot.label} ${channelPlatformName(bot.platform)}`
              .toLowerCase()
              .includes(search.toLowerCase()),
          ) ? (
            <GroupedRow title="No available bots found." />
          ) : null}
        </GroupedList>
        {ownerLink ? (
          <OwnerLink
            label={ownerLink.label}
            link={ownerLink.link}
            busy={busy}
            onRenew={() => void requestLink(ownerLink.botId, ownerLink.agentId)}
          />
        ) : null}
      </GroupedDetail>
    </div>
  );
}

function OwnerLink({
  label,
  link,
  busy,
  onRenew,
}: {
  readonly label: string;
  readonly link: NyxAgentChannelLink;
  readonly busy: boolean;
  readonly onRenew: () => void;
}) {
  return (
    <GroupedList
      title="Owner link"
      aria-label="Owner link"
      footer={
        <>
          <p>{link.instructions}</p>
          <p>Single use. Expires {formatDateTime(link.expires_at)}.</p>
        </>
      }
    >
      <GroupedRow title={`Verify your account for ${label}`} />
      {link.url ? (
        <GroupedRow asChild interactive>
          <a href={link.url} target="_blank" rel="noopener noreferrer">
            <span className="flex-1">Open link</span>
            <ExternalLink aria-hidden />
          </a>
        </GroupedRow>
      ) : null}
      <GroupedRow>
        <CopyableField grouped label="Link code" value={link.code} size="sm" />
      </GroupedRow>
      <GroupedRow asChild interactive>
        <button type="button" disabled={busy} onClick={onRenew}>
          New link
        </button>
      </GroupedRow>
    </GroupedList>
  );
}
