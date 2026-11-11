import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useChannelBotHealth } from "@/hooks/use-channel-bots";
import { useNyxBotChannels } from "@/hooks/use-nyxbot-agents";
import { useOrgSpecialistChannel } from "@/hooks/use-org-specialist-channel";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { Button } from "@/components/ui/button";
import { formatDateTime } from "@/lib/utils";
import type { ChannelBotDetail } from "@/types/channels";
import type { ChannelBotDiagnostics } from "@/schemas/channel-bot-health";
import type { NyxAgentChannelAgent } from "@/schemas/assistant-nyxagent";

export const LARK_VERIFY_SUCCESS = "App credentials verified. This does not verify the callback, a member's chat account, or a real conversation.";

/** Reuse the existing, live-authorized connection and member setup APIs. */
export function LarkBotSetupStatus({ bot }: { readonly bot: ChannelBotDetail }) {
  useUiTranslation();
  const health = useChannelBotHealth(bot.id);
  const channels = useNyxBotChannels();
  // Another admin's current org binding can follow this person's retired one.
  const candidates = channels.data?.filter((row) => row.channel_bot_id === bot.id);
  const connection = candidates?.find((row) => row.status === "active")
    ?? candidates?.find((row) => row.status === "pending") ?? candidates?.[0];
  const member = useOrgSpecialistChannel(connection?.org_specialist ? connection.id : "");
  return <LarkBotSetupStatusView
    bot={bot}
    diagnostics={health.data?.diagnostics}
    connection={connection}
    memberVerified={member.settings.error ? undefined : member.settings.data?.member_verified}
    connectionsLoaded={channels.data !== undefined && !channels.error}
    refreshing={health.isFetching || channels.isFetching || member.settings.isFetching}
    onRefresh={() => {
      void health.refetch();
      void channels.refetch();
      if (connection?.org_specialist) void member.settings.refetch();
    }}
  />;
}

export function LarkBotSetupStatusView({
  bot, diagnostics: d, connection, memberVerified, connectionsLoaded, refreshing, onRefresh,
}: {
  readonly bot: ChannelBotDetail;
  readonly diagnostics?: ChannelBotDiagnostics;
  readonly connection?: NyxAgentChannelAgent;
  readonly memberVerified?: boolean;
  readonly connectionsLoaded: boolean;
  readonly refreshing?: boolean;
  readonly onRefresh: () => void;
}) {
  useUiTranslation();
  const observed = (at: string | null | undefined) => at ? `Last observed ${formatDateTime(at)}` : "Not yet observed";
  const credentialStatus = bot.last_verification?.status;
  const credentials = credentialStatus === "verified" ? "App credentials verified only"
    : credentialStatus === "pending" ? "Checking app credentials…"
      : credentialStatus === "failed" ? "Last credential check failed — retry Verify Bot"
        : credentialStatus === "incomplete" ? "Last check incomplete — retry Verify Bot"
          : "No recorded credential check — use Verify Bot";
  const route = d?.last_route_result === "no_route" ? "Message received, but no eligible route was found"
    : d?.last_route_result === "unavailable" ? "Message received; route lookup could not complete"
      : d?.last_route_result === "matched" ? "A real incoming message matched a route"
        : "No incoming route check observed yet";
  const connectionState = connection
    ? connection.status === "active" ? "Agent connected" : `Connection ${connection.status}`
    : bot.conversations_count > 0 ? "Route configured"
      : connectionsLoaded ? "Connect an agent or configure a route" : "Connection status unavailable";
  const setupUrl = connection
    ? `/assistant?${new URLSearchParams({ ...(connection.agent_id ? { agent: connection.agent_id } : {}), details: "channels", connection: connection.id })}`
    : "/assistant?details=channels";
  return <DetailSection title={translateUi("Setup status")} action={<Button variant="ghost" size="sm" disabled={refreshing} onClick={onRefresh}>{translateUi("Refresh setup status")}</Button>}>
    <div className="space-y-2 p-4 type-secondary text-text-secondary">
      <p>{translateUi("Verify Bot checks app credentials only. Callback setup, each member’s identity, and a real conversation are separate steps.")}</p>
      <p>{translateUi("These are the latest observations, not an end-to-end health check. Not yet observed does not mean misconfigured.")}</p>
    </div>
    <DetailRow label={translateUi("App credentials")} value={credentials} />
    <DetailRow label={translateUi("Callback challenge")} value={observed(d?.last_callback_challenge_at)} />
    <DetailRow label={translateUi("Authenticated inbound")} value={d?.last_inbound_at ? `${observed(d.last_inbound_at)}${d.last_inbound_source ? ` via ${d.last_inbound_source}` : ""}` : "No real message observed yet — send the bot a message"} />
    <DetailRow label={translateUi("Route / agent connection")} value={`${connectionState}. ${d?.last_route_at ? `Last check ${formatDateTime(d.last_route_at)}: ` : ""}${route}.`} />
    <DetailRow label={translateUi("Your member identity")} value={connection?.org_specialist
      ? memberVerified === true ? "Your chat account is verified" : memberVerified === false ? "Not verified — verify your own chat account" : "Verification status unavailable — continue setup"
      : connectionsLoaded ? "Not required unless connected to an organization specialist" : "Connection status unavailable"} />
    <DetailRow label={translateUi("Outbound delivery")} value={d?.last_outbound_error ? `Last send failed (${d.last_outbound_error.reason ?? d.last_outbound_error.code}) at ${formatDateTime(d.last_outbound_error.at)}. ${d.last_outbound_at ? observed(d.last_outbound_at) : "No successful outbound observed yet"}.` : d?.last_outbound_at ? `${observed(d.last_outbound_at)} — delivery alone does not confirm an agent answer` : "No successful outbound observed yet"} />
    <div className="space-y-2 p-4 type-secondary text-text-secondary">
      {connection?.org_specialist ? <p>{translateUi("Each organization member, including admins, must verify individually. Generate a code, DM it to this bot, wait for observation, then confirm in your own NyxID session. Expired codes can be replaced.")}</p> : null}
      <a href={setupUrl} className="type-button text-foreground underline">
        {connection?.org_specialist ? translateUi("Continue setup / Verify my chat account") : connection ? translateUi("Continue setup in agent Channels") : translateUi("Connect to NyxBot or a specialist")}
      </a>
      <p>{translateUi("Configure the callback in the Lark / Feishu console, finish any member verification, then send a real message and check the reply. Refresh these observations after each step.")}</p>
    </div>
  </DetailSection>;
}
