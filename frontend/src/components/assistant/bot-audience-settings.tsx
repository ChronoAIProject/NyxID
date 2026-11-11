import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import {
  GroupedList,
  GroupedRow,
  GroupedSelectTrigger,
} from "@/components/ui/grouped-list";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { useOrgSpecialistChannel } from "@/hooks/use-org-specialist-channel";
import {
  useGuestReadiness,
  useSetNyxBotPrivateChats,
} from "@/hooks/use-nyxbot-agents";
import { botAudienceDescription } from "@/lib/assistant/bot-audience";
import { useDrawerSearchTarget } from "@/lib/drawer-search";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { GuestReadiness } from "./guest-readiness";
import { AudienceReadiness } from "./audience-readiness";
import type {
  AssistantAgent,
  NyxAgentChannelAgent,
} from "@/schemas/assistant-nyxagent";
import { type OrgChannelSettings } from "@/schemas/org-specialist-channel";

export function BotAudienceSettings({
  channel,
  agent,
  canManage,
  busy = false,
  onReconnect,
}: {
  readonly channel: NyxAgentChannelAgent;
  readonly agent?: AssistantAgent;
  readonly canManage: boolean;
  readonly busy?: boolean;
  readonly onReconnect?: () => void;
}) {
  useUiTranslation();
  const live = channel.status === "active";
  const drafts = useGroupedDraftGuard();
  const searchId = `channel-audience-${channel.id}`;
  const searchTarget = useDrawerSearchTarget({
    id: searchId,
    title: "Who can talk to this bot",
    breadcrumb: `Channels › ${channel.bot_label}`,
    keywords:
      "Guests guest access private chats audience Grants organization spending billing",
  });
  return (
    <section
      ref={searchTarget}
      data-drawer-search-target={searchId}
      aria-label={translateUi("Who can talk to this bot · {{value1}}", {
        value1: channel.bot_label,
      })}
      className="space-y-3"
    >
      {channel.org_specialist ? (
        <OrganizationAudience
          channel={channel}
          disabled={!live || !canManage}
        />
      ) : (
        <PersonalAudience
          channel={channel}
          agent={agent}
          disabled={!live || !canManage}
        />
      )}
      {!live ? (
        <div className="space-y-2 px-4 type-secondary text-text-tertiary">
          <p role="status">
            {channel.status === "pending"
              ? translateUi(
                  "Connection pending. Finish connecting this bot to change who can talk to it. Your saved settings are shown here.",
                )
              : translateUi(
                  "This connection is unavailable. Saved guest settings are kept; reconnect the same bot and specialist to use them again.",
                )}
          </p>
          {canManage && onReconnect ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={async () => {
                if (!drafts || (await drafts.confirmDiscard())) onReconnect();
              }}
            >
              {channel.status === "pending"
                ? translateUi("Finish connecting")
                : translateUi("Reconnect")}
            </Button>
          ) : (
            <p>
              {channel.org_specialist
                ? translateUi(
                    "Ask an organization admin to restore access and reconnect this bot.",
                  )
                : translateUi(
                    "Choose an available agent above, then reconnect this bot.",
                  )}
            </p>
          )}
        </div>
      ) : null}
    </section>
  );
}

function PersonalAudience({
  channel,
  agent,
  disabled,
}: {
  readonly channel: NyxAgentChannelAgent;
  readonly agent?: AssistantAgent;
  readonly disabled: boolean;
}) {
  useUiTranslation();
  const access = useSetNyxBotPrivateChats();
  const readiness = useGuestReadiness(channel.agent_id ?? agent?.id, !disabled);
  return (
    <GroupedList
      title={translateUi("Who can talk to this bot")}
      footer={
        <>
          <p>{botAudienceDescription(channel)}</p>
          <GuestReadiness
            readiness={readiness.data ?? channel.guest_readiness}
          />
          <AudienceReadiness configuration={readiness.data?.configuration} />
          {readiness.isError ? (
            <p>
              {translateUi(
                "Customer readiness is unavailable. Refresh to check before relying on replies.",
              )}
            </p>
          ) : null}
          {agent?.kind === "specialist" ? (
            <a
              className="type-button underline"
              href={`/assistant?agent=${encodeURIComponent(agent.id)}&details=grants`}
            >
              {translateUi("Guest service access · Agent details → Grants")}
            </a>
          ) : null}
          {access.error ? (
            <p role="alert" className="text-destructive">
              {access.error.message}
            </p>
          ) : null}
        </>
      }
    >
      <GroupedRow
        title={translateUi("Private chats")}
        accessory={
          <Select
            value={channel.private_chats}
            disabled={disabled || access.isPending}
            onValueChange={(value) =>
              access.mutate({
                channelAgentId: channel.id,
                privateChats: value as "owner" | "everyone",
              })
            }
          >
            <GroupedSelectTrigger
              aria-label={translateUi(
                "Who can talk to {{value1}} in private chats",
                { value1: channel.bot_label },
              )}
            >
              <SelectValue />
            </GroupedSelectTrigger>
            <SelectContent>
              <SelectItem value="owner">{translateUi("Only you")}</SelectItem>
              <SelectItem value="everyone">{translateUi("Anyone")}</SelectItem>
            </SelectContent>
          </Select>
        }
      />
    </GroupedList>
  );
}

function OrganizationAudience({
  channel,
  disabled,
}: {
  readonly channel: NyxAgentChannelAgent;
  readonly disabled: boolean;
}) {
  useUiTranslation();
  const api = useOrgSpecialistChannel(channel.id);
  const settings = api.settings.data;
  if (!settings)
    return (
      <GroupedList
        title={translateUi("Who can talk to this bot")}
        footer={
          <>
            <p>{botAudienceDescription(channel)}</p>
            {api.settings.error ? (
              <p role="alert" className="text-destructive">
                {api.settings.error.message}
              </p>
            ) : (
              <p role="status">{translateUi("Loading guest settings…")}</p>
            )}
          </>
        }
      >
        <GroupedRow
          title={translateUi("Allow guests")}
          accessory={
            <Switch
              aria-label={translateUi("Allow guests")}
              checked={channel.org_specialist?.guest_enabled ?? false}
              disabled
            />
          }
        />
        <GroupedRow
          title={translateUi("Organization spend today")}
          description={translateUi("Unavailable until guest settings load")}
        />
      </GroupedList>
    );
  return (
    <GuestSettings
      channel={channel}
      settings={settings}
      api={api}
      disabled={
        disabled ||
        !settings.can_manage ||
        Boolean(api.settings.error) ||
        (settings.channel_status !== undefined &&
          settings.channel_status !== "active")
      }
    />
  );
}

function GuestSettings({
  channel,
  settings,
  api,
  disabled,
}: {
  readonly channel: NyxAgentChannelAgent;
  readonly settings: OrgChannelSettings;
  readonly api: ReturnType<typeof useOrgSpecialistChannel>;
  readonly disabled: boolean;
}) {
  useUiTranslation();
  const legacy = settings.guest_policy_mode !== "grants";
  const grantsLink = `/assistant?agent=${encodeURIComponent(settings.agent_id ?? channel.agent_id ?? "")}&details=grants`;
  return (
    <GroupedList
      title={translateUi("Who can talk to this bot")}
      footer={
        <>
          <p>{botAudienceDescription(channel)}</p>
          <p>{translateUi("Group chats keep their own audience settings.")}</p>
          {!settings.grants_ready ? (
            <p role="status">
              {translateUi(
                "Guest settings are upgrading. Existing service limits and the legacy daily cap still apply until every server is ready.",
              )}
            </p>
          ) : null}
          {settings.readiness_reason ? (
            <p role="status">
              {settings.readiness_reason === "org_wallet_not_provisioned"
                ? translateUi(
                    "Provision the organization billing wallet and subscription before enabling guests.",
                  )
                : translateUi("The organization wallet is suspended.")}{" "}
              <a
                className="type-button underline"
                href={`/billing?ownerId=${encodeURIComponent(settings.org_id)}`}
              >
                {translateUi("Open organization Billing")}
              </a>
            </p>
          ) : null}
          {api.save.error ? (
            <p role="alert" className="text-destructive">
              {api.save.error.message}
            </p>
          ) : null}
        </>
      }
    >
      <GroupedRow
        title={translateUi("Allow guests")}
        accessory={
          <Switch
            aria-label={translateUi("Allow guests")}
            checked={settings.guest_enabled}
            disabled={
              disabled ||
              api.save.isPending ||
              ((!settings.grants_ready || Boolean(settings.readiness_reason)) &&
                !settings.guest_enabled)
            }
            onCheckedChange={(enabled) =>
              api.save.mutate({
                enabled,
                expected_revision: settings.guest_revision,
              })
            }
          />
        }
      />
      <GroupedRow className="block space-y-1">
        {legacy ? (
          <p>
            {translateUi(
              "Existing guest limits are kept. Review them in Grants.",
            )}
          </p>
        ) : (
          <>
            <p>
              {translateUi(
                "Enabling guests applies this specialist's Grants, including the default Use access.",
              )}
            </p>
            <p>
              {translateUi("Guests can use")}:{" "}
              {settings.effective_guest_services.length
                ? settings.effective_guest_services
                    .map((s) =>
                      s.label === "Unavailable service"
                        ? translateUi("Unavailable service")
                        : s.label,
                    )
                    .join(", ")
                : translateUi("Model-only chat")}
            </p>
          </>
        )}
        <a className="type-button underline" href={grantsLink}>
          {translateUi("Guest service access · Agent details → Grants")}
        </a>
      </GroupedRow>
      <GroupedRow
        title={translateUi("Organization spend today")}
        description={translateUi("{{value1}} credits ({{value2}} reserved)", {
          value1: settings.spent_today_credits,
          value2: settings.reserved_today_credits,
        })}
      />
      <GroupedRow
        description={translateUi(
          "This bot's guest spend today, including earlier connections. The organization pays.",
        )}
      />
    </GroupedList>
  );
}
