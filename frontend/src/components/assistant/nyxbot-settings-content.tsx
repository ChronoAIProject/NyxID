import { AutomationPreferences } from "./automation-preferences";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DetailSection } from "@/components/shared/detail-section";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
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

export function NyxBotSettingsContent() {
  const settings = useNyxBotSettings();
  return (
    <section aria-label="NyxBot settings" className="space-y-8">
      <DetailSection title="General">
        <div className="p-5">
          {settings.isPending ? (
            <p role="status" className="text-12 text-text-tertiary">
              Loading settings...
            </p>
          ) : settings.error ? (
            <ErrorBanner
              message={`Could not load settings. ${settings.error.message}`}
              onRetry={() => void settings.refetch()}
            />
          ) : (
            <SettingsForm settings={settings.data} />
          )}
        </div>
      </DetailSection>
      <DetailSection title="Automations">
        <div className="p-5">
          <AutomationPreferences />
        </div>
      </DetailSection>
      <DetailSection title="Channel bots">
        <div className="@container/channel-bots p-5">
          <ChannelBotsSection />
        </div>
      </DetailSection>
    </section>
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

function SettingsForm({ settings }: { readonly settings: NyxAgentSettings }) {
  const update = useUpdateNyxBotSettings();
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
      ...(dirty.specialist_access_requests
        ? { specialist_access_requests: values.specialist_access_requests }
        : {}),
    };
    try {
      form.reset(formValues(await update.mutateAsync(body)));
    } catch (cause) {
      setError(errorMessage(cause, "Could not save settings. Try again."));
    }
  }

  return (
    <Form {...form}>
      <form
        aria-label="NyxBot preferences"
        // Limits are validated by the schema so messages match the rest of the form.
        noValidate
        onSubmit={form.handleSubmit(save)}
        className="space-y-4"
      >
        <FormField
          control={form.control}
          name="access_mode"
          render={({ field }) => (
            <FormItem className="space-y-2 rounded-lg border border-border p-4">
              <FormLabel>Access mode</FormLabel>
              <FormDescription className="text-12">
                When NyxBot and your specialists ask you before acting. An agent can
                have its own mode in its details.
              </FormDescription>
              <Select value={field.value} onValueChange={field.onChange}>
                <FormControl>
                  <SelectTrigger aria-label="Access mode">
                    <SelectValue />
                  </SelectTrigger>
                </FormControl>
                <SelectContent>
                  {nyxAgentAccessModeSchema.options.map((option) => (
                    <SelectItem key={option} value={option}>
                      {ACCESS_MODE_LABEL[option]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-12 text-muted-foreground">{ACCESS_MODE_DESCRIPTION[mode]}</p>
              {mode === "autopilot" ? (
                <div
                  role="alert"
                  className="flex items-start gap-2 rounded-xl border border-warning/15 bg-warning/[0.04] px-3 py-2.5 text-12 text-warning"
                >
                  <AlertTriangle aria-hidden="true" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span>
                    Approvals are bypassed: NyxBot deletes keys, channel bots, services and
                    nodes and changes access without asking you first. Deletions cannot be
                    undone. Automation, machine, saved-login, publication and service-approval
                    confirmations are also bypassed and recorded in chat.
                  </span>
                </div>
              ) : null}
            </FormItem>
          )}
        />
        {(
          [
            {
              name: "max_auto_continuations",
              label: "Automatic task continuations",
              min: 0,
              max: 32,
              description:
                "Resume long tasks automatically when the assistant reaches a tool or time budget, keeping its context. Default 8; 0 turns this off, up to 32. Stop always ends the task. Continuing uses credits at the usual rates.",
            },
            {
              name: "max_live_subagents",
              label: "Live specialists",
              min: 0,
              max: settings.max_live_subagents_limit,
              description: `0 to ${settings.max_live_subagents_limit}. 0 stops new specialists.`,
            },
            {
              name: "max_concurrent_subagent_turns",
              label: "Specialists working at once",
              min: 1,
              max: settings.max_concurrent_subagent_turns_limit,
              description: `1 to ${settings.max_concurrent_subagent_turns_limit}.`,
            },
            {
              name: "max_group_handoffs",
              label: "Group hand-offs per message",
              min: 0,
              max: settings.max_group_handoffs_limit,
              description: `How many times agents in a group may pass work to each other after one of your messages. 0 to ${settings.max_group_handoffs_limit}; 0 turns hand-offs off.`,
            },
            {
              name: "max_group_handoffs_per_hour",
              label: "Group hand-offs per hour",
              min: 0,
              max: settings.max_group_handoffs_per_hour_limit,
              description: `Across all your groups. 0 to ${settings.max_group_handoffs_per_hour_limit}.`,
            },
          ] as const
        ).map(({ name, label, min, max, description }) => (
          <FormField
            key={name}
            control={form.control}
            name={name}
            render={({ field }) => (
              <FormItem className="grid gap-3 rounded-lg border border-border p-4 sm:grid-cols-[minmax(0,1fr)_8rem] sm:items-center">
                <div className="space-y-1">
                  <FormLabel>{label}</FormLabel>
                  <FormDescription className="text-12">
                    {description}
                  </FormDescription>
                </div>
                <FormControl>
                  <Input
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
        <FormField
          control={form.control}
          name="specialist_access_requests"
          render={({ field }) => (
            <FormItem className="space-y-2 rounded-lg border border-border p-4">
              <FormLabel>Specialist access requests</FormLabel>
              <FormDescription className="text-12">
                Only NyxBot and you can grant access; specialists cannot grant themselves or
                each other anything.
              </FormDescription>
              <Select value={field.value} onValueChange={field.onChange}>
                <FormControl>
                  <SelectTrigger aria-label="Specialist access requests">
                    <SelectValue />
                  </SelectTrigger>
                </FormControl>
                <SelectContent>
                  <SelectItem value="denied">Deny requests</SelectItem>
                  <SelectItem value="routed_to_nyxbot">Route to NyxBot</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-12 text-muted-foreground">
                {field.value === "denied"
                  ? "A specialist that needs more access tells NyxBot in its reply; you can still grant it in the specialist's details."
                  : "A specialist's request goes to NyxBot, which approves or denies it; you can decide it on its card instead."}
              </p>
            </FormItem>
          )}
        />
        <p className="text-11 text-muted-foreground">
          Every specialist turn and group hand-off is billed to your credits
          like any other NyxBot turn, so higher limits can spend credits faster.
        </p>
        {error ? (
          <p role="alert" className="text-12 text-destructive">
            {error}
          </p>
        ) : null}
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            isLoading={update.isPending}
            disabled={!form.formState.isDirty}
          >
            Save settings
          </Button>
        </div>
      </form>
    </Form>
  );
}

function ChannelBotsSection() {
  const agents = useNyxBotAgents();
  return (
    <section aria-label="Channel bots" className="space-y-3">
      <p className="text-12 text-muted-foreground">
        Each channel bot reaches one agent: NyxBot or a specialist. The agent
        only replies to the chat account you verify with a one-time link.
      </p>
      <ChannelBotsManager agents={agents.data?.agents ?? []} />
    </section>
  );
}
