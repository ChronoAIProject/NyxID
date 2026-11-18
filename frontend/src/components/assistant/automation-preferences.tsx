import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Label } from "@/components/ui/label";
import { describeChanges } from "@/lib/grouped-form-changes";
import { ChevronRight } from "lucide-react";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedDisclosure,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import { Link } from "@tanstack/react-router";
import { TimezoneSelect } from "@/components/shared/timezone-select";
import { browserTimezone, formatAutomationTime } from "@/lib/automation-time";
import {
  DEFAULT_SCHEDULE_MINIMUM_MINUTES,
  SCHEDULE_MINIMUM_MINUTES_LIMIT,
  DEFAULT_TRIGGER_RUNS_PER_HOUR,
  TRIGGER_RUNS_PER_HOUR_LIMIT,
  DEFAULT_TRIGGER_RUNS_PER_DAY,
  TRIGGER_RUNS_PER_DAY_LIMIT,
} from "@/lib/automation-limits";
import { useId, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useTriggers } from "@/hooks/use-triggers";
import {
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import { automationPreferencesSchema } from "@/schemas/automations";
import { useAppForm } from "@/components/ui/form";
import { GroupedFormActions } from "@/components/ui/grouped-form";
import {
  GroupedDraftOwnerContext,
  useGroupedDraftGuard,
} from "@/lib/grouped-drafts";
const message = (e: unknown) =>
  e instanceof Error ? e.message : "Could not save preferences";
export function AgentAutomations({ agentId }: { readonly agentId: string }) {
  useUiTranslation();
  const triggers = useTriggers();
  const settings = useNyxBotSettings();
  const rows =
    triggers.data?.triggers.filter(
      (t) => t.delivery.type === "assistant" && t.delivery.agent_id === agentId,
    ) ?? [];
  return (
    <GroupedList title={translateUi("Automations")}>
      {rows.map((row) => (
        <GroupedRow search={{ id: `automation-${row.id}`, title: row.label, breadcrumb: "Overview › Automations", keywords: `${row.source} ${row.status}` }} key={row.id} asChild interactive>
          <Link to="/assistant/automations" search={{ agent: agentId }}>
            <span className="min-w-0 flex-1 break-words">{row.label}</span>
            <span className="text-13 text-text-tertiary">
              {row.status === "active"
                ? row.source === "webhook" ? translateUi("Runs on webhook events") : row.next_run_at
                  ? formatAutomationTime(row.next_run_at, settings.data?.timezone ?? browserTimezone())
                  : translateUi("No run scheduled")
                : translateUi("Paused")}
            </span>
          </Link>
        </GroupedRow>
      ))}
      <GroupedRow search={{ id: "manage-automations", title: `Manage automations (${rows.length})`, breadcrumb: "Overview › Automations" }} asChild interactive>
        <Link to="/assistant/automations" search={{ agent: agentId }}>
          <span className="flex-1">{translateUi("Manage automations (")}{rows.length})</span>
          <ChevronRight
            aria-hidden
            className="size-[1.125em] shrink-0 text-text-tertiary"
          />
        </Link>
      </GroupedRow>
    </GroupedList>
  );
}

export function AutomationPreferences({ title = "Automations" }: { readonly title?: string }) {
  useUiTranslation();
  const settings = useNyxBotSettings();
  const update = useUpdateNyxBotSettings();
  const [open, setOpen] = useState(false);
  const guard = useGroupedDraftGuard();
  return (
    <div className="max-w-2xl space-y-6 text-14">
      <GroupedList title={title}>
        <GroupedDisclosure
          aria-expanded={open}
          onClick={async () => {
            if (
              !open ||
              !guard ||
              (await guard.confirmDiscard("automation-preferences"))
            )
              setOpen(!open);
          }}
        >{translateUi("Timezone and automation budgets")}</GroupedDisclosure>
      </GroupedList>
      {open && settings.data && (
        <GroupedDraftOwnerContext value="automation-preferences">
          <AutomationPreferencesForm
            timezone={settings.data.timezone ?? browserTimezone()}
            minimum={
              settings.data.schedule_minimum_minutes ??
              DEFAULT_SCHEDULE_MINIMUM_MINUTES
            }
            hourly={
              settings.data.trigger_runs_per_hour ?? DEFAULT_TRIGGER_RUNS_PER_HOUR
            }
            daily={
              settings.data.trigger_runs_per_day ?? DEFAULT_TRIGGER_RUNS_PER_DAY
            }
            save={async (v) => {
              await update.mutateAsync(v);
              setOpen(false);
            }}
          />
        </GroupedDraftOwnerContext>
      )}
    </div>
  );
}
function AutomationPreferencesForm({
  timezone,
  minimum,
  hourly,
  daily,
  save,
}: {
  readonly timezone: string;
  readonly minimum: number;
  readonly hourly: number;
  readonly daily: number;
  readonly save: (v: {
    timezone: string;
    schedule_minimum_minutes: number;
    trigger_runs_per_hour: number;
    trigger_runs_per_day: number;
  }) => Promise<void>;
}) {
  useUiTranslation();
  const id = useId();
  const form = useAppForm({
    resolver: zodResolver(automationPreferencesSchema),
    defaultValues: {
      timezone,
      schedule_minimum_minutes: minimum,
      trigger_runs_per_hour: hourly,
      trigger_runs_per_day: daily,
    },
  });
  const [error, setError] = useState<string>();
  const submit = form.handleSubmit(async (v) => {
    setError(undefined);
    try {
      await save(v);
      form.reset(v);
    } catch (e) {
      setError(message(e));
    }
  });
  return (
    <form className="space-y-6" onSubmit={submit}>
      <GroupedList
        title={translateUi("Timezone and automation budgets")}
        footer={
          error && (
            <p role="alert" className="text-destructive">
              {translateUi(error)}
            </p>
          )
        }
      >
        <TimezoneSelect
          grouped
          describedBy={`${id}-timezone-error`}
          invalid={Boolean(form.formState.errors.timezone)}
          value={form.watch("timezone")}
          onChange={(value) => form.setValue("timezone", value)}
        />
        {(
          [
            [
              "schedule_minimum_minutes",
              "Minimum interval (minutes)",
              DEFAULT_SCHEDULE_MINIMUM_MINUTES,
              SCHEDULE_MINIMUM_MINUTES_LIMIT,
            ],
            [
              "trigger_runs_per_hour",
              "Runs per hour",
              1,
              TRIGGER_RUNS_PER_HOUR_LIMIT,
            ],
            [
              "trigger_runs_per_day",
              "Runs per day",
              1,
              TRIGGER_RUNS_PER_DAY_LIMIT,
            ],
          ] as const
        ).map(([key, label, min, max]) => (
          <GroupedRow key={key} asChild>
            <Label>
              <span className="flex-1">{label}</span>
              <GroupedInput
                className="w-20 text-right"
                type="number"
                min={min}
                max={max}
                aria-describedby={`${id}-${key}-error`}
                aria-invalid={Boolean(form.formState.errors[key])}
                {...form.register(key, { valueAsNumber: true, min, max })}
              />
            </Label>
          </GroupedRow>
        ))}
        <GroupedFooter>
          {Object.entries(form.formState.errors).map(([field, validation]) => (
            <p key={field} id={`${id}-${field}-error`} role="alert" className="text-destructive">
              {validation.message}
            </p>
          ))}
        </GroupedFooter>
      </GroupedList>
      <GroupedFormActions
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "timezone", label: "Timezone" }, { key: "schedule_minimum_minutes", label: "Minimum interval (minutes)" }, { key: "trigger_runs_per_hour", label: "Runs per hour" }, { key: "trigger_runs_per_day", label: "Runs per day" }])}
        label={translateUi("automation preferences")}
        dirty={form.formState.isDirty}
        pending={form.formState.isSubmitting}
        save={submit}
        discard={() => {
          form.reset();
          setError(undefined);
        }}
        saveLabel={translateUi("Save preferences")}
      />
    </form>
  );
}
