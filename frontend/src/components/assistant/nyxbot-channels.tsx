import { useState } from "react";
import { ExternalLink } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { CopyableField } from "@/components/shared/copyable-field";
import { ChannelChats } from "@/components/assistant/nyxbot-channel-chats";
import { ErrorBanner } from "@/components/shared/error-banner";
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
import { formatDateTime, formatRelativeTime } from "@/lib/utils";
import type {
  AssistantAgent,
  NyxAgentChannelAgent,
  NyxAgentChannelLink,
} from "@/schemas/assistant-nyxagent";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

const channelStatus: Record<string, { label: string; variant: "success" | "warning" | "destructive" }> =
  {
    active: { label: "Active", variant: "success" },
    pending: { label: "Pending", variant: "warning" },
    failed: { label: "Failed", variant: "destructive" },
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
    <Select value={value} onValueChange={onChange} disabled={disabled || !agents.length}>
      <SelectTrigger aria-label={label} className="h-7 w-[140px] rounded-md px-2">
        <SelectValue placeholder="Choose agent" />
      </SelectTrigger>
      <SelectContent className="z-[90]">
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
  // Only live agents can receive a bot.
  const targets = agents.filter((row) => row.status !== "destroyed");
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

  return (
    <div className="space-y-3">
      {channels.error ? (
        <ErrorBanner
          message={`Could not load connected bots. ${channels.error.message}`}
          onRetry={() => void channels.refetch()}
        />
      ) : null}
      {error ? (
        <p role="alert" className="text-[12px] text-destructive">
          {error}
        </p>
      ) : null}
      {connected.length ? (
        <ul aria-label="Connected channel bots" className="space-y-2">
          {connected.map((row) => {
            const status = channelStatus[row.status];
            return (
              <li key={row.id} className="space-y-2 rounded-lg border border-border px-3 py-2.5">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 space-y-1">
                    <p className="truncate text-[12px] font-medium text-foreground">
                      {row.bot_label}
                      {row.bot_username ? (
                        <span className="ml-1.5 font-normal text-text-tertiary">
                          @{row.bot_username}
                        </span>
                      ) : null}
                    </p>
                    <div className="flex flex-wrap items-center gap-1.5">
                      <Badge variant="secondary">{channelPlatformName(row.platform)}</Badge>
                      {row.org_id ? <Badge variant="secondary">{orgName(row.org_id)}</Badge> : null}
                      <Badge variant={status?.variant ?? "secondary"}>
                        {status?.label ?? row.status}
                      </Badge>
                      <Badge variant={row.owner_linked ? "success" : "warning"}>
                        {row.owner_linked ? "Owner linked" : "Owner not linked"}
                      </Badge>
                    </div>
                  </div>
                  <AgentSelect
                    agents={targets}
                    value={agentOf(row)}
                    label={`Agent for ${row.bot_label}`}
                    disabled={busy}
                    onChange={(agentId) => void relink(row, agentId)}
                  />
                </div>
                {row.last_error ? (
                  <p className="text-[11px] text-destructive">Last error: {row.last_error}</p>
                ) : null}
                {!row.owner_linked && row.inbound_hint ? (
                  <p className="text-[11px] text-muted-foreground">{row.inbound_hint}</p>
                ) : null}
                {row.delivery_status === "failing" ? (
                  <p className="text-[11px] text-destructive">
                    Messages are not reaching the agent: {row.delivery_reason ?? "delivery failed"}
                    {row.delivery_failed_at
                      ? ` (${formatRelativeTime(row.delivery_failed_at)})`
                      : ""}
                    .
                  </p>
                ) : null}
                {row.status === "failed" ? null : (
                  <ChannelChats
                    channel={row}
                    agents={targets}
                    botAgentName={(() => {
                      const current = agents.find((candidate) => candidate.id === agentOf(row));
                      return current ? agentLabel(current) : "NyxBot";
                    })()}
                  />
                )}
                {confirming === row.id ? (
                  <div className="flex flex-wrap items-center justify-end gap-2 rounded-lg bg-overlay px-3 py-2">
                    <span className="mr-auto text-[12px] text-muted-foreground">
                      Stop answering through {row.bot_label}?
                    </span>
                    <Button size="sm" variant="ghost" onClick={() => setConfirming(undefined)}>
                      Cancel
                    </Button>
                    <Button
                      size="sm"
                      variant="destructive"
                      isLoading={disconnect.isPending}
                      onClick={() => void remove(row)}
                    >
                      Disconnect
                    </Button>
                  </div>
                ) : (
                  <div className="flex justify-end gap-1.5">
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy}
                      onClick={() => void requestLink(row.channel_bot_id, agentOf(row))}
                    >
                      {row.owner_linked ? "Link another account" : "Link account"}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => setConfirming(row.id)}
                    >
                      Disconnect
                    </Button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      ) : null}
      {ownerLink ? (
        <OwnerLink
          label={ownerLink.label}
          link={ownerLink.link}
          busy={busy}
          onRenew={() => void requestLink(ownerLink.botId, ownerLink.agentId)}
        />
      ) : null}
      {bots.isPending ? (
        <p className="text-[12px] text-text-tertiary">Loading your channel bots...</p>
      ) : bots.error ? (
        <ErrorBanner
          message="Could not load your channel bots."
          onRetry={() => void bots.refetch()}
        />
      ) : available.length ? (
        <ul aria-label="Channel bots you can connect" className="space-y-2">
          {available.map((bot) => {
            const target = agent?.id ?? connectTargets[bot.id] ?? nyxbot?.id;
            const targetName = agent ? agentLabel(agent) : undefined;
            return (
              <li
                key={bot.id}
                className="flex items-center justify-between gap-2 rounded-lg border border-border px-3 py-2.5"
              >
                <div className="min-w-0">
                  <p className="truncate text-[12px] font-medium text-foreground">{bot.label}</p>
                  <p className="text-[11px] text-text-tertiary">
                    {channelPlatformName(bot.platform)}
                    {bot.user_id !== currentUserId ? ` · ${orgName(bot.user_id) ?? ""}` : ""}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  {agent ? null : (
                    <AgentSelect
                      agents={targets}
                      value={target}
                      label={`Agent for ${bot.label}`}
                      disabled={busy}
                      onChange={(agentId) =>
                        setConnectTargets((current) => ({ ...current, [bot.id]: agentId }))
                      }
                    />
                  )}
                  <Button
                    size="sm"
                    variant="default"
                    disabled={busy || !target}
                    aria-label={`Connect ${bot.label}${targetName ? ` to ${targetName}` : ""}`}
                    onClick={() => void requestLink(bot.id, target)}
                  >
                    Connect
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      ) : connected.length ? null : (
        <p className="rounded-lg bg-overlay px-4 py-3 text-[12px] text-muted-foreground">
          {bots.data?.length
            ? "All your channel bots are connected to other agents. Move one here with its agent selector in NyxBot settings."
            : "You have no channel bots yet. Register one under Channel Bots, then connect it here."}
        </p>
      )}
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
    <div
      role="region"
      aria-label="Owner link"
      className="space-y-3 rounded-xl border border-info/15 bg-info/[0.04] px-4 py-3"
    >
      <div className="space-y-1">
        <p className="text-[12px] font-medium text-foreground">Verify your account for {label}</p>
        <p className="text-[12px] text-muted-foreground">{link.instructions}</p>
      </div>
      {link.url ? (
        <Button asChild variant="primary" size="sm">
          <a href={link.url} target="_blank" rel="noopener noreferrer">
            <ExternalLink aria-hidden="true" />
            Open link
          </a>
        </Button>
      ) : null}
      <CopyableField label="Link code" value={link.code} size="sm" />
      <div className="flex items-center justify-between gap-3">
        <p className="text-[11px] text-text-tertiary">
          Single use. Expires {formatDateTime(link.expires_at)}.
        </p>
        <Button size="sm" variant="outline" disabled={busy} onClick={onRenew}>
          New link
        </Button>
      </div>
    </div>
  );
}
