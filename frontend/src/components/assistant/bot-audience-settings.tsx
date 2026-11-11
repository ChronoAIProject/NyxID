import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useRef, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedSelectTrigger,
} from "@/components/ui/grouped-list";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { useOrgSpecialistChannel } from "@/hooks/use-org-specialist-channel";
import { useGuestReadiness, useSetNyxBotPrivateChats } from "@/hooks/use-nyxbot-agents";
import { botAudienceDescription } from "@/lib/assistant/bot-audience";
import { useDrawerSearchTarget } from "@/lib/drawer-search";
import { useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { GuestReadiness } from "./guest-readiness";
import { AudienceReadiness } from "./audience-readiness";
import { changeList, describeChanges } from "@/lib/grouped-form-changes";
import type {
  AssistantAgent,
  NyxAgentChannelAgent,
} from "@/schemas/assistant-nyxagent";
import {
  orgChannelGuestFormSchema,
  type OrgChannelGuestForm,
  type OrgChannelSettings,
} from "@/schemas/org-specialist-channel";

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
    keywords: "Guests guest access private chats audience daily budget delegated services billing",
  });
  return (
    <section
      ref={searchTarget}
      data-drawer-search-target={searchId}
      aria-label={translateUi("Who can talk to this bot · {{value1}}", { value1: channel.bot_label })}
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
              ? translateUi("Connection pending. Finish connecting this bot to change who can talk to it. Your saved settings are shown here.")
              : translateUi("This connection is unavailable. Saved guest settings are kept; reconnect the same bot and specialist to use them again.")}
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
              {channel.status === "pending" ? translateUi("Finish connecting") : translateUi("Reconnect")}
            </Button>
          ) : (
            <p>
              {channel.org_specialist
                ? translateUi("Ask an organization admin to restore access and reconnect this bot.")
                : translateUi("Choose an available agent above, then reconnect this bot.")}
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
          <GuestReadiness readiness={readiness.data ?? channel.guest_readiness} />
          <AudienceReadiness configuration={readiness.data?.configuration} />
          {readiness.isError ? <p>{translateUi("Customer readiness is unavailable. Refresh to check before relying on replies.")}</p> : null}
          {agent?.kind === "specialist" ? (
            <a
              className="type-button underline"
              href={`/assistant?agent=${encodeURIComponent(agent.id)}&details=grants`}
            >{translateUi("Guest service access · Agent details → Grants")}</a>
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
              aria-label={translateUi("Who can talk to {{value1}} in private chats", { value1: channel.bot_label })}
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
          title={translateUi("Daily guest budget")}
          description={translateUi("{{value1}} credits", { value1: channel.org_specialist?.daily_credits ?? "Not set" })}
        />
        <GroupedRow
          title={translateUi("Delegated services")}
          description={
            channel.org_specialist?.delegated_service_ids.join(", ") ||
            translateUi("Model-only chat")
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

function values(settings: OrgChannelSettings): OrgChannelGuestForm {
  return {
    enabled: settings.guest_enabled,
    service_ids: settings.delegated_service_ids,
    daily_credits: settings.daily_credits ?? settings.default_daily_credits,
  };
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
  const form = useAppForm<OrgChannelGuestForm>({
    resolver: zodResolver(orgChannelGuestFormSchema),
    defaultValues: values(settings),
  });
  const revision = useRef(settings.guest_revision);
  const dirty = form.formState.isDirty;
  // Spending/member verification refreshes must never replace a local draft.
  useEffect(() => {
    if (!dirty) {
      revision.current = settings.guest_revision;
      form.reset(values(settings));
    }
  }, [settings, dirty, form]);
  const [search, setSearch] = useState("");
  const selected = form.watch("service_ids");
  const services = [
    ...settings.available_services,
    ...settings.delegated_service_ids
      .filter(
        (id) =>
          !settings.available_services.some((service) => service.id === id),
      )
      .map((id) => ({ id, label: `Unavailable service (${id})` })),
  ];
  const saving = api.save.isPending;
  const error = api.settings.error ?? api.save.error;
  async function submit() {
    await form.handleSubmit(async (draft) => {
      const saved = await api.save.mutateAsync({
        ...draft,
        expected_revision: revision.current,
      });
      revision.current = saved.guest_revision;
      form.reset(values(saved));
    })();
  }

  async function toggle(enabled: boolean) {
    try {
      const saved = await api.save.mutateAsync({
        ...values(settings),
        enabled,
        expected_revision: settings.guest_revision,
      });
      revision.current = saved.guest_revision;
      form.reset(values(saved));
    } catch {
      /* The shared mutation error keeps the saved switch state visible. */
    }
  }
  return (
    <form onSubmit={submit} className="space-y-3">
      <GroupedList
        title={translateUi("Who can talk to this bot")}
        footer={
          <>
            <p>{settings.description ?? botAudienceDescription(channel)}</p>
            <p>{translateUi("With no delegated services, guests can chat with the model only. Group chats keep their own audience settings.")}</p>
            {!settings.can_manage ? (
              <p>{translateUi("Only organization admins can change guest access.")}</p>
            ) : null}
            {settings.can_manage && settings.readiness_reason ? (
              <p role="status">
                {settings.readiness_reason === "org_wallet_not_provisioned"
                  ? translateUi("Provision the organization billing wallet and subscription before enabling guests.")
                  : translateUi("The organization wallet is suspended.")}{" "}
                <a
                  href={`/billing?ownerId=${encodeURIComponent(settings.org_id)}`}
                  className="type-button underline"
                >{translateUi("Open organization Billing")}</a>
              </p>
            ) : null}
            {dirty ? (
              <p>{translateUi("Save or discard budget and service changes before switching guest access.")}</p>
            ) : null}
            {error ? (
              <p role="alert" className="text-destructive">
                {error.message}
                {dirty
                  ? translateUi(" Your edits are kept. Discard to load the latest saved settings.")
                  : ""}
              </p>
            ) : null}
          </>
        }
      >
        <GroupedRow
          title={translateUi("Allow guests")}
          description={translateUi("Enabling saves immediately with the daily budget below.")}
          accessory={
            <Switch
              aria-label={translateUi("Allow guests")}
              checked={settings.guest_enabled}
              disabled={
                disabled ||
                saving ||
                dirty ||
                (Boolean(settings.readiness_reason) && !settings.guest_enabled)
              }
              onCheckedChange={(enabled) => void toggle(enabled)}
            />
          }
        />
        <GroupedRow className="block space-y-1">
          <Label htmlFor={`guest-budget-${channel.id}`}>{translateUi("Daily guest budget (credits, UTC)")}</Label>
          <GroupedInput
            id={`guest-budget-${channel.id}`}
            {...form.register("daily_credits")}
            inputMode="decimal"
            disabled={disabled || saving}
            aria-invalid={Boolean(form.formState.errors.daily_credits)}
            aria-describedby={
              form.formState.errors.daily_credits
                ? `guest-budget-error-${channel.id}`
                : undefined
            }
          />
          {form.formState.errors.daily_credits ? (
            <span
              id={`guest-budget-error-${channel.id}`}
              role="alert"
              className="type-validation-error text-destructive"
            >
              {form.formState.errors.daily_credits.message}
            </span>
          ) : null}
        </GroupedRow>
        <GroupedRow
          title={translateUi("Delegated services")}
          description={translateUi("Only currently granted organization services can be delegated.")}
        />
        {services.length > 8 ? (
          <GroupedRow>
            <GroupedInput
              aria-label={translateUi("Search delegated services")}
              placeholder={translateUi("Search services")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </GroupedRow>
        ) : null}
        <div
          className={
            services.length > 8 ? "max-h-72 overflow-y-auto" : undefined
          }
        >
          {services
            .filter((service) =>
              service.label.toLowerCase().includes(search.toLowerCase()),
            )
            .map((service) => (
              <GroupedRow key={service.id} asChild interactive>
                <Label>
                  <span className="min-w-0 flex-1">{service.label}</span>
                  <Checkbox
                    checked={selected.includes(service.id)}
                    disabled={disabled || saving}
                    onCheckedChange={(checked) =>
                      form.setValue(
                        "service_ids",
                        checked === true
                          ? [...selected, service.id]
                          : selected.filter((id) => id !== service.id),
                      )
                    }
                  />
                </Label>
              </GroupedRow>
            ))}
        </div>
        {!services.length ? (
          <GroupedRow
            title={translateUi("Model-only chat")}
            description={translateUi("No eligible services are currently granted to this specialist.")}
          />
        ) : null}
        <GroupedRow
          title={translateUi("Organization spend today")}
          description={translateUi("{{value1}} credits ({{value2}} reserved)", { value1: settings.spent_today_credits, value2: settings.reserved_today_credits })}
        />
      </GroupedList>
      <GroupedFormActions
        label={translateUi("{{value1}} guest access", { value1: channel.bot_label })}
        dirty={dirty}
        pending={saving}
        disabled={disabled}
        changes={() =>
          describeChanges(form.formState.defaultValues, form.getValues(), [
            { key: "daily_credits", label: "Daily guest budget" },
            {
              key: "service_ids",
              label: "Delegated services",
              list: (value) =>
                changeList(value).map(
                  (id) =>
                    services.find((service) => service.id === id)?.label ?? id,
                ),
            },
          ])
        }
        save={() => submit()}
        discard={() => {
          revision.current = settings.guest_revision;
          form.reset(values(settings));
          api.save.reset();
        }}
      />
    </form>
  );
}
