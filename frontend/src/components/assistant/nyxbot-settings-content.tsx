import { useCappedItems } from "@/hooks/use-capped-items";
import { useSectionSearchParam } from "@/hooks/use-section-search-param";
import { radioKeys } from "@/lib/radio-group";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { CompactScroll } from "@/components/ui/compact-title";
import { FormViewport } from "@/components/ui/form-viewport";
import { describeChanges, labelValue } from "@/lib/grouped-form-changes";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { ChannelAdapterBindings } from "./channel-adapter-bindings";
import { AutomationPreferences, AutomationRow } from "./automation-preferences";
import { NotificationPreferences } from "./notification-preferences";
import { PlatformPersonalBindings } from "./platform-personal-bindings";
import { useEffect, useState, type ReactNode } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  AlertTriangle,
  Bell,
  Check,
  ListChecks,
  MessageSquare,
  ShieldCheck,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { DialogHeader } from "@/components/ui/dialog";
import { CompactChrome } from "@/components/ui/compact-title";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { GroupedDetailContext } from "@/lib/grouped-detail";
import { useNotificationsEnabled } from "@/hooks/use-assistant-notifications";
import { useNyxAgentLive } from "@/hooks/use-nyxagent-live";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { useChannelBots } from "@/hooks/use-channel-bots";
import { useTriggers } from "@/hooks/use-triggers";
import { Link } from "@tanstack/react-router";
import { GroupedDisclosure } from "@/components/ui/grouped-list";
import {
  availableNyxBotSettingsSection,
  NYXBOT_SETTINGS_SECTIONS,
  NYXBOT_SETTINGS_TITLES,
  nyxBotSettingsAncestry,
  nyxBotSettingsParent,
  parseNyxBotSettingsSection,
  type NyxBotSettingsSection,
} from "@/lib/assistant/nyxbot-settings-sections";
import {
  GroupedList,
  GroupedRow,
  GroupedFooter,
  GroupedInput as Input,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedMessage as FormMessage,
} from "@/components/ui/grouped-fields";
import {
  GroupedFormScope,
  GroupedFormBar,
  GroupedFormActions,
} from "@/components/ui/grouped-form";
import {
  Form,
  FormControl,
  FormField,
  FormLabel,
  useAppForm,
  FormDescription as RowDescription,
} from "@/components/ui/form";
import { ACCESS_MODE_DESCRIPTION, ACCESS_MODE_LABEL } from "@/lib/assistant/nyxbot-labels";
import { SETTINGS_LABELS, SETTINGS_PAGE_FIELDS, SPECIALIST_ACCESS_LABELS } from "@/lib/assistant/nyxbot-settings-fields";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ChannelBotsManager } from "@/components/assistant/nyxbot-channels";
import {
  useNyxBotAgents,
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import {
  nyxAgentAccessModeSchema,
  nyxAgentSettingsFormSchema,
  type NyxAgentSettings,
  type NyxAgentSettingsForm,
  type NyxAgentSettingsUpdate,
} from "@/schemas/assistant-nyxagent";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

const ROOT_GROUPS: readonly (readonly NyxBotSettingsSection[])[] = [
  ["permissions"],
  ["tasks", "automations", "notifications"],
  ["channels"],
];
const ROOT_ICONS: Partial<Record<NyxBotSettingsSection, LucideIcon>> = {
  permissions: ShieldCheck,
  tasks: ListChecks,
  automations: Zap,
  notifications: Bell,
  channels: MessageSquare,
};

export function NyxBotSettingsContent() {
  useUiTranslation();
  const panel = useAccountPanel();
  useNyxAgentLive(true);
  const settings = useNyxBotSettings();
  const bots = useChannelBots({ scope: "all" });
  const { value: rawSection, settled } = useSectionSearchParam("panelTab");
  const triggers = useTriggers();
  const adapters = useAuthStore((s) =>
    userHasFeature(s.user, FEATURE_FLAG.NYXBOT_ADAPTERS),
  );
  const notifications = useNotificationsEnabled();
  const available = { "channels/adapters": adapters, notifications };
  const requested = parseNyxBotSettingsSection(panel.current.panelTab);
  const selected = availableNyxBotSettingsSection(requested, available);
  const ancestry = nyxBotSettingsAncestry(selected);
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  useEffect(() => {
    if (settled && (
      requested !== selected ||
      (rawSection !== undefined && rawSection !== selected)
    ))
      void panel.update({ panelTab: selected }, "replace", { force: true });
  }, [requested, selected, rawSection, settled, panel]);
  const navigate = (section?: NyxBotSettingsSection) =>
    void panel.update({ panelTab: section }, "replace");
  const summaries: Partial<Record<NyxBotSettingsSection, ReactNode>> = {};
  if (settings.isSuccess && !settings.error) {
    summaries.permissions = translateUi(
      ACCESS_MODE_LABEL[formValues(settings.data).access_mode],
    );
    const push = settings.data.notifications?.push;
    if (push) {
      const count = Object.values(push).filter(Boolean).length;
      summaries.notifications = count
        ? translateUi("{{count}} on", { count })
        : translateUi("Off");
    }
  }
  if (bots.isSuccess && !bots.error)
    summaries.channels = String(bots.data.length);
  if (triggers.isSuccess && !triggers.error)
    summaries.automations = String(
      triggers.data.triggers.filter(
        (t) => t.delivery.type === "assistant" && t.status === "active",
      ).length,
    );
  function disclosure(section: NyxBotSettingsSection) {
    const Icon = ROOT_ICONS[section];
    return (
      <GroupedDisclosure
        key={section}
        data-drawer-page-trigger={section}
        onClick={() => navigate(section)}
        leading={Icon ? <Icon aria-hidden /> : undefined}
        accessory={
          summaries[section] != null ? (
            <span className="text-13 text-text-tertiary">
              {summaries[section]}
            </span>
          ) : undefined
        }
      >
        {translateUi(NYXBOT_SETTINGS_TITLES[section])}
      </GroupedDisclosure>
    );
  }
  function settingsPage(section: NyxBotSettingsSection) {
    const loadError = settings.error ? <ErrorBanner
      message={translateUi("Could not load settings. {{value1}}", { value1: settings.error.message })}
      onRetry={() => void settings.refetch()}
    /> : undefined;
    if (settings.data) return <SettingsForm settings={settings.data} section={section} loadError={loadError} />;
    return <GroupedList footer={loadError}>
      {settings.isPending && <GroupedRow title={translateUi("Loading settings...")} role="status" />}
    </GroupedList>;
  }
  function page(section: NyxBotSettingsSection): ReactNode {
    const children = NYXBOT_SETTINGS_SECTIONS.filter(
      (child) =>
        nyxBotSettingsParent(child) === section &&
        available[child as keyof typeof available] !== false,
    );
    return (
      <GroupedDetail
        key={section}
        open={ancestry.includes(section)}
        compactHeader={selected === section}
        title={translateUi(NYXBOT_SETTINGS_TITLES[section])}
        searchPage={section}
        backLabel={translateUi(
          nyxBotSettingsParent(section)
            ? NYXBOT_SETTINGS_TITLES[nyxBotSettingsParent(section)!]
            : "NyxBot settings",
        )}
        onBack={() => navigate(nyxBotSettingsParent(section))}
      >
        {section === "permissions" || section === "tasks"
          ? settingsPage(section)
          : null}
        {section === "automations" && <><AutomationList /><AutomationPreferences detail /></>}
        {section === "notifications" && <NotificationPreferences />}
        {section === "channels" && (
          <div className="@container/channel-bots">
            <ChannelBotsSection />
          </div>
        )}
        {section === "channels/adapters" && adapters && (
          <ChannelAdapterBindings />
        )}
        {section === "channels/links" && <PlatformPersonalBindings />}
        {children.length > 0 && (
          <GroupedList>{children.map(disclosure)}</GroupedList>
        )}
        {children.map(page)}
      </GroupedDetail>
    );
  }
  return (
    <GroupedFormScope viewport>
      <section
        aria-label={translateUi("NyxBot settings")}
        className="flex min-h-0 flex-1 flex-col"
      >
        <FormViewport className="flex min-h-0 flex-1 flex-col">
          <div
            ref={setContainer}
            className="relative flex min-h-0 flex-1 flex-col"
          >
            <CompactChrome enabled={!selected} className="[[data-compact=true]]:border-b [[data-compact=true]]:border-border/50">
              <DialogHeader className="shrink-0 pr-6">
                <h2
                  tabIndex={-1}
                  className="type-card-title outline-none [[data-compact=true]_&]:text-15"
                >
                  {translateUi("NyxBot settings")}
                </h2>
              </DialogHeader>
            </CompactChrome>
            <GroupedDetailContext value={{ container, visible: true }}>
              <CompactScroll
                active={!selected}
                className="flex-1 space-y-6 pb-4 pt-4"
              >
                {ROOT_GROUPS.map((group, index) => (
                  <GroupedList key={index} footer={index === 0 ? translateUi("NyxBot runs with full access to your connected services and account; specialists use only what you or NyxBot grant them. These settings apply to all of your agents.") : undefined}>
                    {group.filter(section => available[section as keyof typeof available] !== false).map(disclosure)}
                  </GroupedList>
                ))}
              </CompactScroll>
              {NYXBOT_SETTINGS_SECTIONS.filter(
                (section) => !nyxBotSettingsParent(section) && available[section as keyof typeof available] !== false,
              ).map(page)}
            </GroupedDetailContext>
          </div>
        </FormViewport>
        <GroupedFormBar viewport />
      </section>
    </GroupedFormScope>
  );
}

function AutomationList() {
  useUiTranslation();
  const settings = useNyxBotSettings();
  const triggers = useTriggers();
  const visible = useCappedItems(triggers.data?.triggers.filter(t => t.delivery.type === "assistant") ?? [], 3);
  return (
    <GroupedList
      footer={
        triggers.error ? (
          <ErrorBanner
            message={triggers.error.message}
            onRetry={() => void triggers.refetch()}
          />
        ) : undefined
      }
    >
      {triggers.isPending && (
        <GroupedRow
          role="status"
          title={translateUi("Loading automations...")}
        />
      )}
      {!triggers.error && visible.items.map(row => <AutomationRow key={row.id} row={row} timezone={settings.data?.timezone} />)}
      {visible.remaining > 0 && <GroupedRow asChild interactive><button type="button" onClick={visible.showAll}>{translateUi("Show all")}</button></GroupedRow>}

      <GroupedRow asChild interactive>
        <Link to="/assistant/automations">
          {translateUi("Manage automations")}
        </Link>
      </GroupedRow>
    </GroupedList>
  );
}

function formValues(settings: NyxAgentSettings): NyxAgentSettingsForm {
  return {
    access_mode: settings.access_mode ?? (settings.skip_destructive_confirmation ? "autopilot" : "nyxmode"),
    max_auto_continuations: settings.max_auto_continuations ?? 8,
    max_live_subagents: settings.max_live_subagents,
    max_concurrent_subagent_turns: settings.max_concurrent_subagent_turns,
    max_group_handoffs: settings.max_group_handoffs,
    max_group_handoffs_per_hour: settings.max_group_handoffs_per_hour,
    specialist_access_requests: settings.specialist_access_requests ?? "denied",
  };
}

function SettingsForm({
  settings,
  section,
  loadError,
}: {
  readonly loadError?: ReactNode;
  readonly settings: NyxAgentSettings;
  readonly section: NyxBotSettingsSection;
}) {
  useUiTranslation();
  const update = useUpdateNyxBotSettings();
  const form = useAppForm<NyxAgentSettingsForm>({
    resolver: zodResolver(nyxAgentSettingsFormSchema(settings)),
    defaultValues: formValues(settings),
  });
  const [error, setError] = useState<string>();
  const mode = form.watch("access_mode");
  const fields = SETTINGS_PAGE_FIELDS[section] ?? [];
  const errors = <>{loadError}{error && <p role="alert" className="text-destructive">{error}</p>}</>;
  const { reset } = form;
  useEffect(() => {
    reset(formValues(settings), { keepDirtyValues: true });
  }, [settings, reset]);

  async function save(values: NyxAgentSettingsForm) {
    setError(undefined);
    const dirty = Object.fromEntries(
      fields.map((key) => [key, form.formState.dirtyFields[key]]),
    );
    // Send only what changed, so a concurrent edit elsewhere is not overwritten.
    const body: NyxAgentSettingsUpdate = {
      ...(dirty.access_mode ? { access_mode: values.access_mode } : {}),
      ...(dirty.max_auto_continuations
        ? { max_auto_continuations: values.max_auto_continuations }
        : {}),
      ...(dirty.max_live_subagents
        ? { max_live_subagents: values.max_live_subagents }
        : {}),
      ...(dirty.max_concurrent_subagent_turns
        ? {
            max_concurrent_subagent_turns: values.max_concurrent_subagent_turns,
          }
        : {}),
      ...(dirty.max_group_handoffs
        ? { max_group_handoffs: values.max_group_handoffs }
        : {}),
      ...(dirty.max_group_handoffs_per_hour
        ? { max_group_handoffs_per_hour: values.max_group_handoffs_per_hour }
        : {}),
      ...(dirty.specialist_access_requests
        ? { specialist_access_requests: values.specialist_access_requests }
        : {}),
    };
    try {
      form.reset(formValues(await update.mutateAsync(body)));
    } catch (cause) {
      setError(errorMessage(cause, translateUi("Could not save settings. Try again.")));
    }
  }

  return (
    <Form {...form}>
      <form
        aria-label={translateUi(NYXBOT_SETTINGS_TITLES[section])}
        // Limits are validated by the schema so messages match the rest of the form.
        noValidate
        onSubmit={form.handleSubmit(save)}
        className="space-y-6"
      >
        {section === "permissions" && (
          <GroupedList
            title={translateUi("Access mode")}
            footer={
              <>
                <p>
                  {translateUi(
                    "When NyxBot and your specialists ask you before acting. An agent can have its own mode in its details.",
                  )}
                </p>
                {errors}
                {mode === "autopilot" && (
                  <div
                    role="alert"
                    className="flex items-start gap-2 text-warning"
                  >
                    <AlertTriangle
                      aria-hidden
                      className="mt-0.5 h-3.5 w-3.5 shrink-0"
                    />
                    <span>
                      {translateUi(
                        "Approvals are bypassed: NyxBot deletes keys, channel bots, services and nodes and changes access without asking you first. Deletions cannot be undone. Automation, machine, saved-login, publication and service-approval confirmations are also bypassed and recorded in chat.",
                      )}
                    </span>
                  </div>
                )}
              </>
            }
            role="radiogroup" aria-label={translateUi("Access mode")}
          >
            {nyxAgentAccessModeSchema.options.map((option) => (
              <GroupedRow key={option} asChild interactive>
                <button
                  type="button"
                  role="radio"
                  aria-checked={mode === option}
                  tabIndex={mode === option ? 0 : -1}
                  onKeyDown={radioKeys}
                  onClick={() => form.setValue("access_mode", option)}
                >
                  <span className="min-w-0 flex-1">
                    <span>{translateUi(ACCESS_MODE_LABEL[option])}</span>
                    <span className="block text-13 text-text-tertiary">
                      {translateUi(ACCESS_MODE_DESCRIPTION[option])}
                    </span>
                  </span>
                  {mode === option && (
                    <Check aria-hidden className="size-[1.125em] shrink-0" />
                  )}
                </button>
              </GroupedRow>
            ))}
          </GroupedList>
        )}
        {section === "tasks" && [fields.slice(0, 1), fields.slice(1)].map((groupFields, index) => (
          <GroupedList key={index}
            title={index === 1 ? translateUi("Specialists & group hand-offs") : undefined}
            footer={<>{index === 1 && <p>{translateUi("Every specialist turn and group hand-off is billed to your credits like any other NyxBot turn, so higher limits can spend credits faster.")}</p>}{index === 1 && errors}</>}
          >
            {(
              [
                {
                  name: "max_auto_continuations",
                  label: SETTINGS_LABELS.max_auto_continuations,
                  min: 0,
                  max: 32,
                  description: translateUi("0–32 · 0 turns this off. Resumes with the same context after a tool or time budget; Stop always ends the task."),
                },
                {
                  name: "max_live_subagents",
                  label: SETTINGS_LABELS.max_live_subagents,
                  min: 0,
                  max: settings.max_live_subagents_limit,
                  description: translateUi("0–{{limit}} · 0 stops new specialists", { limit: settings.max_live_subagents_limit }),
                },
                {
                  name: "max_concurrent_subagent_turns",
                  label: SETTINGS_LABELS.max_concurrent_subagent_turns,
                  min: 1,
                  max: settings.max_concurrent_subagent_turns_limit,
                  description: translateUi("1–{{limit}} · Maximum specialists working simultaneously", { limit: settings.max_concurrent_subagent_turns_limit }),
                },
                {
                  name: "max_group_handoffs",
                  label: SETTINGS_LABELS.max_group_handoffs,
                  min: 0,
                  max: settings.max_group_handoffs_limit,
                  description: translateUi("0–{{limit}} · 0 turns off hand-offs after a message", { limit: settings.max_group_handoffs_limit }),
                },
                {
                  name: "max_group_handoffs_per_hour",
                  label: SETTINGS_LABELS.max_group_handoffs_per_hour,
                  min: 0,
                  max: settings.max_group_handoffs_per_hour_limit,
                  description: translateUi("0–{{limit}} · Across all your groups", { limit: settings.max_group_handoffs_per_hour_limit }),
                },
              ] as const
            )
              .filter(({ name }) => groupFields.includes(name))
              .map(({ name, label, min, max, description }) => (
                <FormField
                  key={name}
                  control={form.control}
                  name={name}
                  render={({ field }) => (
                    <FormItem inline>
                      <div className="min-w-0 flex-1">
                        <FormLabel>{translateUi(label)}</FormLabel>
                        <RowDescription className="type-secondary text-text-tertiary">
                          {description}
                        </RowDescription>
                      </div>
                      <FormControl>
                        <Input
                          className="w-20 text-right"
                          type="number"
                          inputMode="numeric"
                          min={min}
                          max={max}
                          step={1}
                          {...field}
                          value={Number.isNaN(field.value) ? "" : field.value}
                          onChange={(event) =>
                            field.onChange(event.target.valueAsNumber)
                          }
                        />
                      </FormControl>
                      <FormMessage className="sm:col-span-2" />
                    </FormItem>
                  )}
                />
              ))}
          </GroupedList>
        ))}
        {section === "permissions" && (
          <GroupedList title={translateUi("Specialist access requests")} role="radiogroup" aria-label={translateUi("Specialist access requests")}>
            {(["denied", "routed_to_nyxbot"] as const).map((option) => (
              <GroupedRow key={option} asChild interactive>
                <button
                  type="button"
                  role="radio"
                  aria-checked={form.watch("specialist_access_requests") === option}
                  tabIndex={form.watch("specialist_access_requests") === option ? 0 : -1}
                  onKeyDown={radioKeys}
                  onClick={() =>
                    form.setValue("specialist_access_requests", option)
                  }
                >
                  <span className="min-w-0 flex-1">
                    <span>
                      {translateUi(SPECIALIST_ACCESS_LABELS[option])}
                    </span>
                    <span className="block text-13 text-text-tertiary">
                      {option === "denied"
                        ? translateUi(
                            "A specialist you talk to directly tells you what it is missing and continues; you grant it in the specialist's details. Only work NyxBot assigned, or work in a group NyxBot is in, reports to NyxBot instead.",
                          )
                        : translateUi(
                            "For work NyxBot assigned, or in a group with NyxBot, a specialist's request goes to NyxBot, which approves or denies it; you can decide it on its card instead. A specialist you talk to directly still tells you what it is missing.",
                          )}
                    </span>
                  </span>
                  {form.watch("specialist_access_requests") === option && (
                    <Check aria-hidden className="size-[1.125em] shrink-0" />
                  )}
                </button>
              </GroupedRow>
            ))}
            <GroupedFooter>
              {errors}
              {translateUi(
                "Only you, and NyxBot for work it assigned or a group it is in, can grant access; specialists cannot grant themselves or each other anything.",
              )}
            </GroupedFooter>
          </GroupedList>
        )}
        <GroupedFormActions
          changes={() =>
            describeChanges(form.formState.defaultValues, form.getValues(), [
              ...(
                Object.keys(SETTINGS_LABELS) as (keyof typeof SETTINGS_LABELS)[]
              ).map((key) => ({ key, label: SETTINGS_LABELS[key] })),
              {
                key: "access_mode",
                label: "Access mode",
                format: labelValue(ACCESS_MODE_LABEL),
              },
              {
                key: "specialist_access_requests",
                label: "Specialist access requests",
                format: labelValue(SPECIALIST_ACCESS_LABELS),
              },
            ])
          }
          label={translateUi(NYXBOT_SETTINGS_TITLES[section])}
          dirty={form.formState.isDirty}
          pending={update.isPending}
          save={form.handleSubmit(save)}
          discard={() => {
            form.reset();
            setError(undefined);
          }}
          saveLabel={translateUi("Save settings")}
        />
      </form>
    </Form>
  );
}

function ChannelBotsSection() {
  useUiTranslation();
  const agents = useNyxBotAgents();
  return (
    <div aria-label={translateUi("Channel bots")} className="space-y-6">
      <GroupedList
        title={translateUi("Channel bots")}
        footer={translateUi("Each channel bot reaches one agent: NyxBot or a specialist. The agent only replies to the chat account you verify with a one-time link.")}
      >
        <GroupedRow title={translateUi("Choose who answers each bot")} />
      </GroupedList>
      <ChannelBotsManager agents={agents.data?.agents ?? []} />
    </div>
  );
}
