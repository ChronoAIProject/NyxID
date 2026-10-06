import { AutomationPreferences } from "./automation-preferences";
import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { AlertTriangle, Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ChannelBotsManager } from "@/components/assistant/nyxbot-channels";
import {
  useNyxBotAgents,
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import {
  nyxAgentSettingsFormSchema,
  type NyxAgentSettings,
  type NyxAgentSettingsForm,
  type NyxAgentSettingsUpdate,
} from "@/schemas/assistant-nyxagent";

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

/** Header gear that opens NyxBot's settings. */
export function NyxBotSettingsButton() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        aria-label="NyxBot settings"
        onClick={() => setOpen(true)}
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors hover:border-hairline-strong hover:text-muted-foreground focus-visible:outline-none"
      >
        <Settings2 className="h-[14px] w-[14px]" />
      </button>
      {open ? <NyxBotSettingsDialog onClose={() => setOpen(false)} /> : null}
    </>
  );
}

export function NyxBotSettingsDialog({ onClose }: { readonly onClose: () => void }) {
  const settings = useNyxBotSettings();
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent scrollMode="body" className="md:max-w-lg">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>NyxBot settings</DialogTitle>
          <DialogDescription>
            NyxBot runs with full access to your connected services and account; specialists use
            only what you or NyxBot grant them. These settings apply to all of your agents.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-6 pb-1">
          {settings.isPending ? (
            <p className="text-12 text-text-tertiary">Loading settings...</p>
          ) : settings.error ? (
            <ErrorBanner
              message={`Could not load settings. ${settings.error.message}`}
              onRetry={() => void settings.refetch()}
            />
          ) : (
            <SettingsForm settings={settings.data} />
          )}
          <AutomationPreferences />
          <ChannelBotsSection />
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

function formValues(settings: NyxAgentSettings): NyxAgentSettingsForm {
  return {
    max_auto_continuations: settings.max_auto_continuations ?? 8,
    confirm_destructive: !settings.skip_destructive_confirmation,
    max_live_subagents: settings.max_live_subagents,
    max_concurrent_subagent_turns: settings.max_concurrent_subagent_turns,
    max_group_handoffs: settings.max_group_handoffs,
    max_group_handoffs_per_hour: settings.max_group_handoffs_per_hour,
  };
}

function SettingsForm({ settings }: { readonly settings: NyxAgentSettings }) {
  const update = useUpdateNyxBotSettings();
  const form = useAppForm<NyxAgentSettingsForm>({
    resolver: zodResolver(nyxAgentSettingsFormSchema(settings)),
    defaultValues: formValues(settings),
  });
  const [error, setError] = useState<string>();
  const confirm = form.watch("confirm_destructive");

  async function save(values: NyxAgentSettingsForm) {
    setError(undefined);
    const dirty = form.formState.dirtyFields;
    // Send only what changed, so a concurrent edit elsewhere is not overwritten.
    const body: NyxAgentSettingsUpdate = {
      ...(dirty.max_auto_continuations ? { max_auto_continuations: values.max_auto_continuations } : {}),
      ...(dirty.confirm_destructive
        ? { skip_destructive_confirmation: !values.confirm_destructive }
        : {}),
      ...(dirty.max_live_subagents ? { max_live_subagents: values.max_live_subagents } : {}),
      ...(dirty.max_concurrent_subagent_turns
        ? { max_concurrent_subagent_turns: values.max_concurrent_subagent_turns }
        : {}),
      ...(dirty.max_group_handoffs ? { max_group_handoffs: values.max_group_handoffs } : {}),
      ...(dirty.max_group_handoffs_per_hour
        ? { max_group_handoffs_per_hour: values.max_group_handoffs_per_hour }
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
          name="confirm_destructive"
          render={({ field }) => (
            <FormItem className="space-y-2">
              <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-4">
                <div className="space-y-1">
                  <FormLabel>Confirm destructive actions</FormLabel>
                  <FormDescription className="text-12">
                    NyxBot asks before deleting keys, bots, services or nodes.
                  </FormDescription>
                </div>
                <FormControl>
                  <Switch checked={field.value} onCheckedChange={field.onChange} />
                </FormControl>
              </div>
              {!confirm ? (
                <div
                  role="alert"
                  className="flex items-start gap-2 rounded-xl border border-warning/15 bg-warning/[0.04] px-3 py-2.5 text-12 text-warning"
                >
                  <AlertTriangle aria-hidden="true" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span>
                    NyxBot will delete keys, channel bots, services and nodes without asking you
                    first. Deletions cannot be undone.
                  </span>
                </div>
              ) : null}
            </FormItem>
          )}
        />
        <FormField control={form.control} name="max_auto_continuations" render={({ field }) => (
          <FormItem>
            <FormLabel>Automatic task continuations</FormLabel>
            <FormControl><Input type="number" inputMode="numeric" min={0} max={32} step={1} {...field} value={Number.isNaN(field.value) ? "" : field.value} onChange={event => field.onChange(event.target.valueAsNumber)} /></FormControl>
            <FormDescription className="text-11">Resume long tasks automatically when the assistant reaches a tool or time budget, keeping its context. Default 8; 0 turns this off, up to 32. Stop always ends the task. Continuing uses credits at the usual rates.</FormDescription>
            <FormMessage />
          </FormItem>
        )} />
        <div className="grid gap-4 sm:grid-cols-2">
          <FormField
            control={form.control}
            name="max_live_subagents"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Live specialists</FormLabel>
                <FormControl>
                  <Input
                    type="number"
                    inputMode="numeric"
                    min={0}
                    max={settings.max_live_subagents_limit}
                    step={1}
                    {...field}
                    value={Number.isNaN(field.value) ? "" : field.value}
                    onChange={(event) => field.onChange(event.target.valueAsNumber)}
                  />
                </FormControl>
                <FormDescription className="text-11">
                  0 to {settings.max_live_subagents_limit}. 0 stops new specialists.
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="max_concurrent_subagent_turns"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Specialists working at once</FormLabel>
                <FormControl>
                  <Input
                    type="number"
                    inputMode="numeric"
                    min={1}
                    max={settings.max_concurrent_subagent_turns_limit}
                    step={1}
                    {...field}
                    value={Number.isNaN(field.value) ? "" : field.value}
                    onChange={(event) => field.onChange(event.target.valueAsNumber)}
                  />
                </FormControl>
                <FormDescription className="text-11">
                  1 to {settings.max_concurrent_subagent_turns_limit}.
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          <FormField
            control={form.control}
            name="max_group_handoffs"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Group hand-offs per message</FormLabel>
                <FormControl>
                  <Input
                    type="number"
                    inputMode="numeric"
                    min={0}
                    max={settings.max_group_handoffs_limit}
                    step={1}
                    {...field}
                    value={Number.isNaN(field.value) ? "" : field.value}
                    onChange={(event) => field.onChange(event.target.valueAsNumber)}
                  />
                </FormControl>
                <FormDescription className="text-11">
                  How many times agents in a group may pass work to each other after one of your
                  messages. 0 to {settings.max_group_handoffs_limit}; 0 turns hand-offs off.
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="max_group_handoffs_per_hour"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Group hand-offs per hour</FormLabel>
                <FormControl>
                  <Input
                    type="number"
                    inputMode="numeric"
                    min={0}
                    max={settings.max_group_handoffs_per_hour_limit}
                    step={1}
                    {...field}
                    value={Number.isNaN(field.value) ? "" : field.value}
                    onChange={(event) => field.onChange(event.target.valueAsNumber)}
                  />
                </FormControl>
                <FormDescription className="text-11">
                  Across all your groups. 0 to {settings.max_group_handoffs_per_hour_limit}.
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
        </div>
        <p className="text-11 text-muted-foreground">
          Every specialist turn and group hand-off is billed to your credits like any other NyxBot
          turn, so higher limits can spend credits faster.
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
    <section aria-labelledby="nyxbot-channel-bots" className="space-y-3">
      <div className="space-y-1">
        <h3 id="nyxbot-channel-bots" className="text-13 font-semibold text-foreground">
          Channel bots
        </h3>
        <p className="text-12 text-muted-foreground">
          Each channel bot reaches one agent: NyxBot or a specialist. The agent only replies to
          the chat account you verify with a one-time link.
        </p>
      </div>
      <ChannelBotsManager agents={agents.data?.agents ?? []} />
    </section>
  );
}
