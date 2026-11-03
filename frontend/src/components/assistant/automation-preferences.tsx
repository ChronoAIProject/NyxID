import { GroupedList, GroupedRow } from "@/components/ui/grouped-list";
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
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useTriggers } from "@/hooks/use-triggers";
import {
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import { automationPreferencesSchema } from "@/schemas/automations";
import { useAppForm } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
const message = (e: unknown) =>
  e instanceof Error ? e.message : "Could not save preferences";
export function AgentAutomations({ agentId }: { readonly agentId: string }) {
  const triggers = useTriggers();
  const settings = useNyxBotSettings();
  const rows =
    triggers.data?.triggers.filter(
      (t) => t.delivery.type === "assistant" && t.delivery.agent_id === agentId,
    ) ?? [];
  return (
    <GroupedList title="Automations">
      {rows.map((row) => (
        <GroupedRow key={row.id} asChild interactive>
          <Link to="/assistant/automations" search={{ agent: agentId }}>
            <span className="min-w-0 flex-1 break-words">{row.label}</span>
            <span className="text-11 text-text-tertiary">
              {row.status === "active"
                ? formatAutomationTime(
                    row.next_run_at,
                    settings.data?.timezone ?? browserTimezone(),
                  )
                : "Paused"}
            </span>
          </Link>
        </GroupedRow>
      ))}
      <GroupedRow asChild interactive>
        <Link to="/assistant/automations" search={{ agent: agentId }}>
          Manage automations ({rows.length})
        </Link>
      </GroupedRow>
    </GroupedList>
  );
}

export function AutomationPreferences() {
  const settings = useNyxBotSettings();
  const update = useUpdateNyxBotSettings();
  const [open, setOpen] = useState(false);
  return (
    <div className="text-12">
      <button className="text-primary-text underline" onClick={() => setOpen(!open)}>
        Timezone and automation budgets
      </button>
      {open && settings.data && (
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
  return (
    <form
      className="mt-3 grid gap-3 rounded-xl border border-border p-4 sm:grid-cols-2"
      onSubmit={form.handleSubmit(async (v) => {
        setError(undefined);
        try {
          await save(v);
        } catch (e) {
          setError(message(e));
        }
      })}
    >
      <TimezoneSelect
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
        <label key={key}>
          {label}
          <Input
            type="number"
            min={min}
            max={max}
            {...form.register(key, { valueAsNumber: true, min, max })}
          />
        </label>
      ))}
      {Object.entries(form.formState.errors).map(([field, validation]) => (
        <p key={field} role="alert" className="text-destructive">
          {validation.message}
        </p>
      ))}
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
      <Button
        type="submit"
        variant="primary"
        disabled={form.formState.isSubmitting}
      >
        Save preferences
      </Button>
    </form>
  );
}
