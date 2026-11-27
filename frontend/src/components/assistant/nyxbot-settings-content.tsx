import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { CompactScroll } from "@/components/ui/compact-title";
import { FormViewport } from "@/components/ui/form-viewport";
import { describeChanges, labelValue } from "@/lib/grouped-form-changes";
import { useAuthStore } from "@/stores/auth-store";
import { FEATURE_FLAG, userHasFeature } from "@/lib/feature-flags";
import { ChannelAdapterBindings } from "./channel-adapter-bindings";
import { AutomationPreferences } from "./automation-preferences";
import { NotificationPreferences } from "./notification-preferences";
import { PlatformPersonalBindings } from "./platform-personal-bindings";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { AlertTriangle } from "lucide-react";
import {
  GroupedList,
  GroupedRow,
  GroupedFooter,
  GroupedInput as Input,
  GroupedSelectTrigger as SelectTrigger,
} from "@/components/ui/grouped-list";
import {
  GroupedField as FormItem,
  GroupedDescription as FormDescription,
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectValue,
} from "@/components/ui/select";
import { ACCESS_MODE_DESCRIPTION, ACCESS_MODE_LABEL } from "@/lib/assistant/nyxbot-labels";
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

const SETTINGS_LABELS = {
  max_auto_continuations: "Automatic task continuations",
  max_live_subagents: "Live specialists",
  max_concurrent_subagent_turns: "Specialists working at once",
  max_group_handoffs: "Group hand-offs per message",
  max_group_handoffs_per_hour: "Group hand-offs per hour",
} as const;

export function NyxBotSettingsContent() {
  useUiTranslation();
  const settings = useNyxBotSettings();
  const adaptersEnabled = useAuthStore(s=>userHasFeature(s.user,FEATURE_FLAG.NYXBOT_ADAPTERS));
  return (
    <GroupedFormScope viewport>
      <section aria-label={translateUi("NyxBot settings")} className="flex min-h-0 flex-1 flex-col">
      <FormViewport className="flex min-h-0 flex-1 flex-col"><CompactScroll className="flex-1 space-y-6 pb-4">
        {settings.isPending ? (
          <GroupedList title={translateUi("General")}>
            <GroupedRow title={translateUi("Loading settings...")} role="status" />
          </GroupedList>
        ) : settings.error ? (
          <GroupedList
            title={translateUi("General")}
            footer={
              <ErrorBanner
                message={translateUi("Could not load settings. {{value1}}", { value1: settings.error.message })}
                onRetry={() => void settings.refetch()}
              />
            }
          />
        ) : (
          <SettingsForm settings={settings.data} />
        )}
        <AutomationPreferences />
        <NotificationPreferences />
        <div className="@container/channel-bots">
          <ChannelBotsSection />
        </div>
        {adaptersEnabled && <ChannelAdapterBindings />}
        <PlatformPersonalBindings />
        </CompactScroll></FormViewport>
        <GroupedFormBar viewport />
      </section>
    </GroupedFormScope>
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
    system_skills_for_specialists:
      settings.system_skills_for_specialists ?? "default",
  };
}

function SettingsForm({ settings }: { readonly settings: NyxAgentSettings }) {
  useUiTranslation();
  const update = useUpdateNyxBotSettings();
  const systemSkillsEnabled = useAuthStore((s) =>
    userHasFeature(s.user, FEATURE_FLAG.SYSTEM_SKILLS),
  );
  const form = useAppForm<NyxAgentSettingsForm>({
    resolver: zodResolver(nyxAgentSettingsFormSchema(settings)),
    defaultValues: formValues(settings),
  });
  const [error, setError] = useState<string>();
  const mode = form.watch("access_mode");

  async function save(values: NyxAgentSettingsForm) {
    setError(undefined);
    const dirty = form.formState.dirtyFields;
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
      ...(dirty.system_skills_for_specialists
        ? {
            system_skills_for_specialists: values.system_skills_for_specialists,
          }
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
        aria-label={translateUi("NyxBot preferences")}
        // Limits are validated by the schema so messages match the rest of the form.
        noValidate
        onSubmit={form.handleSubmit(save)}
        className="space-y-6"
      >
        <GroupedList title={translateUi("General")}>
          <FormField
            control={form.control}
            name="access_mode"
            render={({ field }) => (
              <FormItem inline>
                <FormLabel className="flex-1">{translateUi("Access mode")}</FormLabel>
                <Select value={field.value} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger aria-label={translateUi("Access mode")}>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {nyxAgentAccessModeSchema.options.map((option) => (
                      <SelectItem key={option} value={option}>
                        {translateUi(ACCESS_MODE_LABEL[option])}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormDescription>{translateUi("When NyxBot and your specialists ask you before acting. An agent can have its own mode in its details.")}</FormDescription>
                <FormDescription>{translateUi(ACCESS_MODE_DESCRIPTION[mode])}</FormDescription>
                {mode === "autopilot" ? (
                  <GroupedFooter>
                    <div
                      role="alert"
                      className="flex items-start gap-2 text-warning"
                    >
                      <AlertTriangle aria-hidden="true" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                      <span>{translateUi("Approvals are bypassed: NyxBot deletes keys, channel bots, services and nodes and changes access without asking you first. Deletions cannot be undone. Automation, machine, saved-login, publication and service-approval confirmations are also bypassed and recorded in chat.")}</span>
                    </div>
                  </GroupedFooter>
                ) : null}
              </FormItem>
            )}
          />
        </GroupedList>
        <GroupedList
          title={translateUi("Task limits")}
          footer={translateUi("Every specialist turn and group hand-off is billed to your credits like any other NyxBot turn, so higher limits can spend credits faster.")}
        >
          {(
            [
              {
                name: "max_auto_continuations",
                label: SETTINGS_LABELS.max_auto_continuations,
                min: 0,
                max: 32,
                description:
                  "0–32 · 0 turns this off. Resumes with the same context after a tool or time budget; Stop always ends the task.",
              },
              {
                name: "max_live_subagents",
                label: SETTINGS_LABELS.max_live_subagents,
                min: 0,
                max: settings.max_live_subagents_limit,
                description: `0–${settings.max_live_subagents_limit} · 0 stops new specialists`,
              },
              {
                name: "max_concurrent_subagent_turns",
                label: SETTINGS_LABELS.max_concurrent_subagent_turns,
                min: 1,
                max: settings.max_concurrent_subagent_turns_limit,
                description: `1–${settings.max_concurrent_subagent_turns_limit} · Maximum specialists working simultaneously`,
              },
              {
                name: "max_group_handoffs",
                label: SETTINGS_LABELS.max_group_handoffs,
                min: 0,
                max: settings.max_group_handoffs_limit,
                description: `0–${settings.max_group_handoffs_limit} · 0 turns off hand-offs after a message`,
              },
              {
                name: "max_group_handoffs_per_hour",
                label: SETTINGS_LABELS.max_group_handoffs_per_hour,
                min: 0,
                max: settings.max_group_handoffs_per_hour_limit,
                description: `0–${settings.max_group_handoffs_per_hour_limit} · Across all your groups`,
              },
            ] as const
          ).map(({ name, label, min, max, description }) => (
            <FormField
              key={name}
              control={form.control}
              name={name}
              render={({ field }) => (
                <FormItem inline>
                  <div className="min-w-0 flex-1">
                    <FormLabel>
                      {label}
                    </FormLabel>
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
        {systemSkillsEnabled && (
        <GroupedList title={translateUi("Built-in skills")}>
          <FormField
            control={form.control}
            name="system_skills_for_specialists"
            render={({ field }) => (
              <FormItem inline>
                <FormLabel className="flex-1">{translateUi("Built-in skills for specialists")}</FormLabel>
                <Select value={field.value} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger aria-label={translateUi("Built-in skills for specialists")}>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value="default">
                      {translateUi("Use NyxBot’s choices as the default")} </SelectItem>
                    <SelectItem value="override">
                      {translateUi("Apply NyxBot’s choices to all specialists")} </SelectItem>
                  </SelectContent>
                </Select>
                <FormDescription>
                  {field.value === "override"
                    ? translateUi("Specialists use your NyxBot’s choices. Their saved choices are retained and apply again when you switch back to defaults.")
                    : translateUi("Each specialist inherits your NyxBot’s choices and can have its own choice.")}
                </FormDescription>
              </FormItem>
            )}
          />
        </GroupedList>
        )}
        <GroupedList
          title={translateUi("Specialist access")}
          footer={
            error ? (
              <p role="alert" className="text-destructive">
                {translateUi(error)}
              </p>
            ) : undefined
          }
        >
          <FormField
            control={form.control}
            name="specialist_access_requests"
            render={({ field }) => (
              <FormItem inline>
                <FormLabel className="flex-1">{translateUi("Specialist access requests")}</FormLabel>
                <Select value={field.value} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger aria-label={translateUi("Specialist access requests")}>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value="denied">{translateUi("Deny requests")}</SelectItem>
                    <SelectItem value="routed_to_nyxbot">{translateUi("Route to NyxBot")}</SelectItem>
                  </SelectContent>
                </Select>
                <FormDescription>{translateUi("Only you, and NyxBot for work it assigned or a group it is in, can grant access; specialists cannot grant themselves or each other anything.")}</FormDescription>
                <FormDescription>
                    {field.value === "denied"
                      ? translateUi("A specialist you talk to directly tells you what it is missing and continues; you grant it in the specialist's details. Only work NyxBot assigned, or work in a group NyxBot is in, reports to NyxBot instead.")
                      : translateUi("For work NyxBot assigned, or in a group with NyxBot, a specialist's request goes to NyxBot, which approves or denies it; you can decide it on its card instead. A specialist you talk to directly still tells you what it is missing.")}
                </FormDescription>
              </FormItem>
            )}
          />
        </GroupedList>
        <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [...(Object.keys(SETTINGS_LABELS) as (keyof typeof SETTINGS_LABELS)[]).map(key => ({ key, label: SETTINGS_LABELS[key] })), { key: "system_skills_for_specialists", label: "Built-in skills for specialists", format: labelValue({ default: "Use NyxBot’s choices as the default", override: "Apply NyxBot’s choices to all specialists" }) }, { key: "access_mode", label: "Access mode", format: labelValue(ACCESS_MODE_LABEL) }, { key: "specialist_access_requests", label: "Specialist access requests", format: labelValue({ denied: "Deny requests", routed_to_nyxbot: "Route to NyxBot" }) }])}
          label={translateUi("NyxBot preferences")}
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
