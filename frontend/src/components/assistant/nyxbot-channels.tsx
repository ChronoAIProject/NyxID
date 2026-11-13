import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useDrawerSearchEntries } from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger,
  GroupedDisclosure,
  GroupedInput,
} from "@/components/ui/grouped-list";
import { useEffect, useRef, useState } from "react";
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
import { OrgSpecialistChannelSettings } from "@/components/assistant/org-specialist-channel-settings";
import { BotAudienceSettings } from "@/components/assistant/bot-audience-settings";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useOrgs } from "@/hooks/use-orgs";
import { useAuthStore } from "@/stores/auth-store";
import {
  nyxBotOf,
  useConnectNyxBotChannel,
  useDisconnectNyxBotChannel,
  useLinkNyxBotChannel,
  useNyxBotChannels,
  useRetryNyxBotChannelGateway,
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
  useUiTranslation();
  return (
    <Select
      value={value}
      onValueChange={onChange}
      disabled={disabled || !agents.length}
    >
      <GroupedSelectTrigger aria-label={label} className="max-w-full">
        <SelectValue placeholder={translateUi("Choose agent")} />
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
  connectionId,
}: {
  readonly agents: readonly AssistantAgent[];
  readonly agent?: AssistantAgent;
  readonly connectionId?: string;
}) {
  useUiTranslation();
  const channels = useNyxBotChannels();
  const drafts = useGroupedDraftGuard();
  const focusedConnection = useRef<HTMLLIElement | null>(null);
  const [connectedOrg, setConnectedOrg] = useState<string>();
  const hasRequestedConnection = channels.data?.some((row) => row.id === connectionId) ?? false;
  useEffect(() => {
    if (connectionId && focusedConnection.current) {
      const target = focusedConnection.current.querySelector<HTMLElement>("[data-member-verification]") ?? focusedConnection.current;
      target.scrollIntoView?.({ block: "start" });
      target.focus({ preventScroll: true });
    }
  }, [connectionId, hasRequestedConnection]);
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
  const retryGateway = useRetryNyxBotChannelGateway();
  const [ownerLink, setOwnerLink] = useState<{
    readonly botId: string;
    readonly agentId?: string;
    readonly label: string;
    readonly link: NyxAgentChannelLink;
  }>();
  const [confirming, setConfirming] = useState<string>();
  const [connectTargets, setConnectTargets] = useState<Record<string, string>>(
    {},
  );
  const [error, setError] = useState<string>();
  const nyxbot = nyxBotOf(agents);
  const orgChannels = useFeature(FEATURE_FLAG.ORG_SPECIALIST_CHANNELS);
  const adminOf = (id: string | null | undefined) =>
    orgs.data?.some((org) => org.id === id && org.your_role === "admin") ??
    false;
  // Organization targets require a same-org bot and live admin authority.
  const targets = agents.filter(
    (row) =>
      row.status !== "destroyed" &&
      !row.share_role &&
      (row.owner_kind !== "org" || (orgChannels && adminOf(row.owner_id))),
  );
  const agentOf = (row: NyxAgentChannelAgent) => row.agent_id ?? nyxbot?.id;
  const connected = (channels.data ?? []).filter(
    (row) => !agent || agentOf(row) === agent.id,
  );
  const connectedBotIds = new Set(
    (channels.data ?? []).map((row) => row.channel_bot_id),
  );
  const available = (bots.data ?? []).filter(
    (bot) =>
      !connectedBotIds.has(bot.id) &&
      (agent?.owner_kind !== "org" ||
        (orgChannels &&
          bot.user_id === agent.owner_id &&
          adminOf(agent.owner_id))),
  );
  const busy =
    connect.isPending ||
    disconnect.isPending ||
    link.isPending ||
    retryGateway.isPending;

  async function retryNative(row: NyxAgentChannelAgent) {
    setError(undefined);
    try {
      await retryGateway.mutateAsync({ channelAgentId: row.id });
    } catch (cause) {
      setError(errorMessage(cause, "Could not retry native gateway activation."));
    }
  }

  async function requestLink(botId: string, agentId: string | undefined) {
    if (connectedBotIds.has(botId) && drafts && !await drafts.confirmDiscard()) return;
    setError(undefined);
    setConnectedOrg(undefined);
    try {
      const result = await connect.mutateAsync({ botId, agentId });
      if (result.channel_agent.org_specialist) {
        setOwnerLink(undefined);
        if (result.channel_agent.status === "active") setConnectedOrg(result.channel_agent.bot_label);
        setAvailableOpen(false);
      }
      if (!result.channel_agent.org_specialist)
        setOwnerLink({
          botId,
          agentId,
          label: result.channel_agent.bot_label,
          link: result.link,
        });
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
    if (drafts && !await drafts.confirmDiscard()) return;
    setError(undefined);
    try {
      await link.mutateAsync({ channelAgentId: row.id, agentId });
    } catch (cause) {
      setError(
        errorMessage(
          cause,
          "Could not move this bot to that agent. Try again.",
        ),
      );
    }
  }

  async function remove(row: NyxAgentChannelAgent) {
    if (drafts && !await drafts.confirmDiscard()) return;
    setError(undefined);
    setConnectedOrg(undefined);
    try {
      await disconnect.mutateAsync(row.id);
      setConfirming(undefined);
      if (ownerLink?.botId === row.channel_bot_id) setOwnerLink(undefined);
    } catch (cause) {
      setError(
        errorMessage(cause, "Could not disconnect this bot. Try again."),
      );
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
          title={translateUi("Connection status")}
          footer={
            <p role="alert" className="text-destructive">
              {error ??
                translateUi("Could not load connected bots. {{value1}}", { value1: channels.error?.message })}
            </p>
          }
        >
          <GroupedRow asChild interactive>
            <button type="button" onClick={() => void channels.refetch()}>{translateUi("Refresh connected bots")}</button>
          </GroupedRow>
        </GroupedList>
      ) : null}
      {connectedOrg ? <p role="status" className="type-secondary text-text-secondary">
        {connectedOrg}{translateUi(" is connected to the organization specialist. Each member, including admins, must verify their own chat account below before using member permissions. Connecting the bot does not verify anyone.")}</p> : null}
      {connectionId && channels.data && !connected.some((row) => row.id === connectionId) ? <p role="status" className="type-secondary text-text-secondary">{translateUi("This connection is no longer available to you. Refresh connected bots or ask an organization admin.")}</p> : null}
      {connected.length ? (
        <ul aria-label={translateUi("Connected channel bots")} className="space-y-6">
          {connected.map((row) => {
            const target = targets.find((candidate) => candidate.id === agentOf(row));
            const live = row.status === "pending" || row.status === "active";
            const canManage =
              !row.org_specialist || adminOf(row.org_specialist.org_id);
            const rowTargets = targets.filter((a) =>
              row.org_specialist
                ? a.owner_kind === "org" &&
                  a.owner_id === row.org_specialist.org_id
                : a.owner_kind !== "org",
            );
            return (
              <li key={row.id} ref={row.id === connectionId ? focusedConnection : undefined} tabIndex={-1} aria-label={translateUi("{{value1}} connection", { value1: row.bot_label })} className="space-y-6 scroll-mt-4 focus-visible:outline focus-visible:outline-ring">
                <GroupedList
                  title={translateUi("Connected bot")}
                  footer={
                    <>
                      {row.last_error ? (
                        <p
                          className="text-13 text-destructive"
                          data-channel-error={row.last_error}
                        >{translateUi("Last error: ")}{nyxBotChannelError(row.last_error)}
                        </p>
                      ) : null}
                      {row.platform === "discord" ? <DiscordGatewayStatus row={row} /> : null}
                      {!row.org_specialist && !row.owner_linked && row.inbound_hint ? (
                        <p>{row.inbound_hint}</p>
                      ) : null}
                      {["failing", "unconfirmed", "interrupted", "media_failed"].includes(
                        row.delivery_status ?? "",
                      ) ? (
                        <p className="text-13 text-destructive">
                          {row.delivery_status === "unconfirmed"
                            ? translateUi("Intake not confirmed: ")
                            : row.delivery_status === "failing"
                            ? translateUi("Messages are not reaching the agent: ")
                            : translateUi("Attachment problem: ")}
                          {row.delivery_reason ?? (row.delivery_status === "unconfirmed" ? translateUi("No authenticated inbound has been confirmed.") : translateUi("delivery failed"))}
                          {row.delivery_failed_at
                            ? translateUi(" ({{value}})", { value: formatRelativeTime(row.delivery_failed_at) })
                            : ""}
                          .
                        </p>
                      ) : null}
                      {confirming === row.id ? (
                        <p>{translateUi("Stop answering through ")}{row.bot_label}?</p>
                      ) : null}
                    </>
                  }
                >
                  <GroupedRow
                    search={{ id: `connected-bot-${row.id}`, title: row.bot_label, breadcrumb: "Channels › Connected bot", keywords: `${row.bot_username ?? ""} ${channelPlatformName(row.platform)} ${channelStatus[row.status] ?? "Unavailable"} ${row.org_specialist ? "Organization specialist" : row.owner_linked ? "Owner linked" : "Owner not linked"}` }}
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
                      row.org_specialist ? "Organization specialist" : row.owner_linked ? "Owner linked" : "Owner not linked",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  />
                  <GroupedRow
                    title={translateUi("Answered by")}
                    accessory={
                      <AgentSelect
                        agents={rowTargets}
                        value={agentOf(row)}
                        label={translateUi("Agent for {{value1}}", { value1: row.bot_label })}
                        disabled={busy || !canManage}
                        onChange={(agentId) => void relink(row, agentId)}
                      />
                    }
                  />
                  {live && canManage ? (
                    <ChannelChats
                      channel={row}
                      agents={rowTargets}
                      botAgentName={(() => {
                        const current = agents.find((candidate) => candidate.id === agentOf(row));
                        return current ? agentLabel(current) : "NyxBot";
                      })()}
                    />
                  ) : null}
                  {canManage && row.platform === "discord" && row.transport === "direct" && row.status === "active" && row.gateway_native_activation ? (
                    <GroupedRow
                      asChild
                      interactive
                      search={{ id: `retry-gateway-${row.id}`, title: "Retry native gateway now", breadcrumb: `Channels › ${row.bot_label}`, keywords: "Discord gateway activation retry" }}
                    >
                      <button type="button" disabled={busy || !row.owner_linked} onClick={() => void retryNative(row)}>{translateUi("Retry native gateway now")}</button>
                    </GroupedRow>
                  ) : null}
                  {canManage && live && !row.org_specialist ? (
                    <GroupedRow asChild interactive>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() =>
                          void requestLink(row.channel_bot_id, agentOf(row))
                        }
                      >
                        {row.owner_linked
                          ? translateUi("Link another account")
                          : translateUi("Link account")}
                      </button>
                    </GroupedRow>
                  ) : null}
                  {!canManage ? null : confirming === row.id ? (
                    <>
                      <GroupedRow asChild interactive>
                        <button
                          type="button"
                          disabled={disconnect.isPending}
                          onClick={() => setConfirming(undefined)}
                        >{translateUi("Cancel")}</button>
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
                        >{translateUi("Disconnect")}</button>
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
                      >{translateUi("Disconnect")}</button>
                    </GroupedRow>
                  )}
                </GroupedList>
                <BotAudienceSettings channel={row} agent={agent?.id === agentOf(row) ? agent : agents.find((candidate) => candidate.id === agentOf(row))}
                  canManage={canManage} busy={busy || !target}
                  onReconnect={() => { if (target) void requestLink(row.channel_bot_id, target.id); }} />
                {row.org_specialist && live ? (
                  <OrgSpecialistChannelSettings id={row.id} />
                ) : null}
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
        title={translateUi("Channel bots")}
        footer={
          !available.length && !bots.isPending && !bots.error
            ? bots.data?.length
              ? translateUi("All your channel bots are connected to other agents. Move one here with its agent selector in NyxBot settings.")
              : translateUi("You have no channel bots yet. Register one under Channel Bots, then connect it here.")
            : undefined
        }
      >
        <GroupedDisclosure data-drawer-page-trigger="available-bots" search={{ id: "connect-another-bot", title: "Connect another bot", breadcrumb: "Channels" }} onClick={() => setAvailableOpen(true)}>{translateUi("Connect another bot")}</GroupedDisclosure>
      </GroupedList>
      <GroupedDetail
        searchPage="available-bots"
        open={availableOpen}
        title={translateUi("Connect another bot")}
        onReveal={() => setAvailableOpen(true)}
        backLabel={translateUi("Back to Channels")}
        onBack={() => setAvailableOpen(false)}
      >
        <GroupedList
          title={translateUi("Available bots")}
          footer={
            <>
              {bots.error ? (
                <p role="alert" className="text-destructive">{translateUi("Could not load your channel bots.")}</p>
              ) : null}
              {error && availableOpen ? (
                <p role="alert" className="text-destructive">
                  {translateUi(error)}
                </p>
              ) : null}
            </>
          }
        >
          <GroupedRow>
            <GroupedInput
              aria-label={translateUi("Search channel bots")}
              placeholder={translateUi("Search bots")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </GroupedRow>
          {bots.isPending ? (
            <GroupedRow title={translateUi("Loading your channel bots...")} />
          ) : bots.error ? (
            <GroupedRow asChild interactive>
              <button type="button" onClick={() => void bots.refetch()}>{translateUi("Retry channel bots")}</button>
            </GroupedRow>
          ) : null}
          <ul aria-label={translateUi("Channel bots you can connect")}>
            {available
              .filter((bot) =>
                `${bot.label} ${channelPlatformName(bot.platform)}`
                  .toLowerCase()
                  .includes(search.toLowerCase()),
              )
              .map((bot) => {
                const target = agent?.id ?? connectTargets[bot.id] ?? nyxbot?.id;
                const targetName = agent ? agentLabel(agent) : undefined;
                const botTargets = targets.filter(
                  (candidate) => candidate.owner_kind !== "org" || candidate.owner_id === bot.user_id,
                );
                return (
                  <GroupedRow data-drawer-search-target={`available-bot-${bot.id}`} key={bot.id} asChild>
                    <li>
                      <div className="min-w-0 flex-1">
                        <p>{bot.label}</p>
                        <p className="type-secondary text-text-tertiary">
                          {channelPlatformName(bot.platform)}
                          {bot.user_id !== currentUserId
                            ? translateUi(" · {{value}}", { value: orgName(bot.user_id) ?? "" })
                            : ""}
                        </p>
                      </div>
                      <div className="ml-auto flex max-w-full flex-wrap items-center gap-2">
                        {agent ? null : (
                          <AgentSelect
                            agents={botTargets}
                            value={target}
                            label={translateUi("Agent for {{value1}}", { value1: bot.label })}
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
                          aria-label={targetName ? translateUi("Connect {{bot}} to {{target}}", { bot: bot.label, target: targetName }) : translateUi("Connect {{bot}}", { bot: bot.label })}
                          onClick={() => void requestLink(bot.id, target)}
                        >{translateUi("Connect")}</Button>
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
            <GroupedRow title={translateUi("No available bots found.")} />
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
  useUiTranslation();
  return (
    <GroupedList
      title={translateUi("Owner link")}
      aria-label={translateUi("Owner link")}
      footer={
        <>
          <p>{link.instructions}</p>
          <p>{translateUi("Single use. Expires ")}{formatDateTime(link.expires_at)}.</p>
        </>
      }
    >
      <GroupedRow title={translateUi("Verify your account for {{value1}}", { value1: label })} />
      {link.url ? (
        <GroupedRow asChild interactive>
          <a href={link.url} target="_blank" rel="noopener noreferrer">
            <span className="flex-1">{translateUi("Open link")}</span>
            <ExternalLink aria-hidden />
          </a>
        </GroupedRow>
      ) : null}
      <GroupedRow>
        <CopyableField grouped label={translateUi("Link code")} value={link.code} size="sm" />
      </GroupedRow>
      <GroupedRow asChild interactive>
        <button type="button" disabled={busy} onClick={onRenew}>{translateUi("New link")}</button>
      </GroupedRow>
    </GroupedList>
  );
}

function DiscordGatewayStatus({ row }: { readonly row: NyxAgentChannelAgent }) {
  useUiTranslation();
  if (row.discord_intake) {
    const intake = row.discord_intake;
    return <div className="space-y-1 type-secondary text-muted-foreground">
      <p>{translateUi("Receiving side: ")}{intake.receiving_side === "native_gateway" ? translateUi("Agent Event Gateway") : intake.receiving_side === "interactions_only" ? translateUi("Discord interactions") : translateUi("NyxID relay")}{translateUi(". Intake: ")}{intake.state}.</p>
      <p>{translateUi("Last confirmed inbound: ")}{intake.last_inbound_at ? formatDateTime(intake.last_inbound_at) : translateUi("not observed")}.</p>
      {intake.handed_over_at ? <p>{translateUi("Handed over: ")}{formatDateTime(intake.handed_over_at)}.</p> : null}
      {intake.block_reason ? <p>{translateUi("Intake blocked by: ")}{intake.block_reason}.</p> : null}
      {row.gateway_native_activation ? <p>{translateUi("Native selection: ")}{row.gateway_native_activation_detail ?? row.gateway_native_activation}{translateUi(". Selection does not confirm connection health.")}</p> : null}
    </div>;
  }
  if (row.transport === "gateway") {
    if (row.gateway_source?.type !== "discord") {
      return <p className="text-13 text-muted-foreground">{translateUi("Agent Event Gateway with NyxID relay intake. Native Discord is not confirmed.")}</p>;
    }
    return <p className="text-13 text-muted-foreground">{translateUi("Native gateway selected. Activation: ")}{row.gateway_native_activation ?? translateUi("not reported")}.
      {row.gateway_native_delivery ? translateUi(" Gateway delivery: {{value1}}.", { value1: row.gateway_native_delivery }) : translateUi(" Native delivery has not been confirmed.")}
    </p>;
  }
  if (!row.gateway_native_activation) return null;
  const draining = row.gateway_native_activation === "relay_draining";
  return <div className="space-y-1 text-13 text-muted-foreground">
    <p>{draining ? translateUi("Waiting for the native gateway to release Discord before the relay can resume.") : row.status === "active" && row.delivery_status === "ok" ? translateUi("Working on NyxID relay.") : translateUi("NyxID relay selected. Check channel health for delivery status.")}</p>
    {row.gateway_native_activation !== "relay_ready" && !draining ? <>
      <p>{translateUi("Native gateway activation did not complete: ")}{row.gateway_native_activation_detail ?? row.gateway_native_activation}.</p>
      <p>{translateUi("A successful relay reply does not confirm native gateway acceptance.")}</p>
      {row.gateway_next_retry_at ? <p>{translateUi("Next automatic retry: ")}{formatDateTime(row.gateway_next_retry_at)}{translateUi(" (while the gateway flag is enabled; deferred while answering).")}</p> : null}
      {row.gateway_retry_available_at ? <p>{translateUi("Manual retry available after ")}{formatDateTime(row.gateway_retry_available_at)}.</p> : null}
    </> : null}
  </div>;
}
