import type { TriggerResponse } from "@/schemas/triggers";
import { ErrorBanner } from "@/components/shared/error-banner";
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
import { AUTOMATION_BUDGET_ROWS, AUTOMATION_TIMEZONE_LABEL } from "@/lib/assistant/nyxbot-settings-fields";
import {
  DEFAULT_SCHEDULE_MINIMUM_MINUTES,
  DEFAULT_TRIGGER_RUNS_PER_HOUR,
  DEFAULT_TRIGGER_RUNS_PER_DAY,
} from "@/lib/automation-limits";
import { useContext, useEffect, useId, useState, type ReactNode } from "react";
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
      {rows.map(row => <AutomationRow key={row.id} row={row} timezone={settings.data?.timezone} agentId={agentId} />)}
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

/** Shared saved status and destination for global and per-agent lists. */
export function AutomationRow({ row, timezone, agentId }: { readonly row: TriggerResponse; readonly timezone?: string | null; readonly agentId?: string }) {
  useUiTranslation();
  return <GroupedRow search={{ id: `automation-${row.id}`, title: row.label, breadcrumb: "Overview › Automations", keywords: `${row.source} ${row.status}` }} asChild interactive>
    <Link to="/assistant/automations" search={agentId ? { agent: agentId } : {}}>
      <span className="min-w-0 flex-1 break-words">{row.label}</span>
      <span className="text-13 text-text-tertiary">
        {row.status === "active"
          ? row.source === "webhook" ? translateUi("Runs on webhook events") : row.next_run_at
            ? formatAutomationTime(row.next_run_at, timezone ?? browserTimezone())
            : translateUi("No run scheduled")
          : translateUi("Paused")}
      </span>
    </Link>
  </GroupedRow>;
}

export function AutomationPreferences({
  title = "Automations",
  detail = false,
}: {
  readonly title?: string;
  readonly detail?: boolean;
}) {
  useUiTranslation();
  const settings = useNyxBotSettings();
  const update = useUpdateNyxBotSettings();
  const [open, setOpen] = useState(false);
  const guard = useGroupedDraftGuard();
  const owner = useContext(GroupedDraftOwnerContext);
  const loadError = settings.error ? <ErrorBanner message={translateUi("Could not load settings. {{value1}}", { value1: settings.error.message })} onRetry={() => void settings.refetch()} /> : undefined;
  return (
    <div className="max-w-2xl space-y-6 text-14">
      {!detail && (
        <GroupedList title={translateUi(title)}>
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
          >
            {translateUi("Timezone and automation budgets")}
          </GroupedDisclosure>
        </GroupedList>
      )}
      {(detail || open) && settings.isPending && (
        <GroupedList>
          <GroupedRow
            role="status"
            title={translateUi("Loading settings...")}
          />
        </GroupedList>
      )}
      {(detail || open) && settings.error && !settings.data && <GroupedList footer={loadError} />}
      {(detail || open) && settings.data && (
        <GroupedDraftOwnerContext
          value={detail ? owner : "automation-preferences"}
        >
          <AutomationPreferencesForm loadError={loadError}
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
              if (!detail) setOpen(false);
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
  loadError,
}: {
  readonly loadError?: ReactNode;
  readonly timezone: string;
  readonly minimum: number;
  readonly hourly: number;
  readonly daily: number;
  readonly save: (v: Partial<{
    timezone: string;
    schedule_minimum_minutes: number;
    trigger_runs_per_hour: number;
    trigger_runs_per_day: number;
  }>) => Promise<void>;
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
  const { reset } = form;
  useEffect(() => {
    reset({ timezone, schedule_minimum_minutes: minimum, trigger_runs_per_hour: hourly, trigger_runs_per_day: daily }, { keepDirtyValues: true });
  }, [timezone, minimum, hourly, daily, reset]);
  const submit = form.handleSubmit(async (v) => {
    setError(undefined);
    try {
      await save(Object.fromEntries(Object.entries(v).filter(([key]) => form.formState.dirtyFields[key as keyof typeof v])));
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
          <>{loadError}{error && (
            <p role="alert" className="text-destructive">
              {translateUi(error)}
            </p>
          )}</>
        }
      >
        <TimezoneSelect
          label={AUTOMATION_TIMEZONE_LABEL}
          grouped
          describedBy={`${id}-timezone-error`}
          invalid={Boolean(form.formState.errors.timezone)}
          value={form.watch("timezone")}
          onChange={(value) => form.setValue("timezone", value)}
        />
        {AUTOMATION_BUDGET_ROWS.map(([key, label, min, max]) => (
          <GroupedRow key={key} asChild>
            <Label>
              <span className="flex-1">{translateUi(label)}</span>
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
          changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "timezone", label: AUTOMATION_TIMEZONE_LABEL }, ...AUTOMATION_BUDGET_ROWS.map(([key, label]) => ({ key, label }))])}
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
