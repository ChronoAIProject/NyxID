import { Link, useSearch } from "@tanstack/react-router";
import { parseAutomationSearch } from "@/lib/automation-search";
import { browserTimezone, formatAutomationTime } from "@/lib/automation-time";
import { TimezoneSelect } from "@/components/shared/timezone-select";
import { AutomationDateTime } from "@/components/shared/automation-date-time";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  useAutomationSetup,
  type AutomationSetup,
} from "@/hooks/use-automations";
import { AutomationPreferences } from "@/components/assistant/automation-preferences";
import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { MoreVertical } from "lucide-react";
import { toast } from "sonner";
import {
  useCreateTrigger,
  useDeleteTrigger,
  useTriggers,
  useUpdateTrigger,
} from "@/hooks/use-triggers";
import {
  useAutomationChats,
  useAutomationRuns,
  useRunAutomation,
  useSchedulePreview,
} from "@/hooks/use-automations";
import {
  useNyxBotAgents,
  useNyxBotSettings,
  useUpdateNyxBotSettings,
} from "@/hooks/use-nyxbot-agents";
import {
  automationDefaults,
  automationFormSchema,
  automationRequest,
  formSchedule,
  type AutomationForm,
  type ScheduleSpec,
} from "@/schemas/automations";
import type { TriggerResponse } from "@/schemas/triggers";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  OneTimeSecretDialog,
  type OneTimeSecretValue,
} from "@/components/shared/one-time-secret-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Form, useAppForm } from "@/components/ui/form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const message = (e: unknown) =>
  e instanceof Error ? e.message : "Could not save automation";

export function AutomationsPage() {
  const search = parseAutomationSearch(useSearch({ strict: false }));
  const setup = useAutomationSetup(search.setup);
  const settings = useNyxBotSettings();
  const zone = settings.data?.timezone ?? browserTimezone();
  const triggers = useTriggers();
  const agents = useNyxBotAgents();
  const remove = useDeleteTrigger();
  const update = useUpdateTrigger();
  const run = useRunAutomation();
  const [editing, setEditing] = useState<TriggerResponse | "new" | null>(() =>
    search.setup ? "new" : null,
  );
  const [history, setHistory] = useState<TriggerResponse | null>(null);
  const [deleting, setDeleting] = useState<TriggerResponse | null>(null);
  const [secrets, setSecrets] = useState<readonly OneTimeSecretValue[]>([]);
  const filterAgent = search.agent;
  const rows = (triggers.data?.triggers ?? []).filter(
    (row) =>
      !filterAgent ||
      (row.delivery.type === "assistant" &&
        row.delivery.agent_id === filterAgent),
  );
  const target = (row: TriggerResponse) => {
    const delivery = row.delivery;
    return delivery.type === "assistant"
      ? (agents.data?.agents.find((a) => a.id === delivery.agent_id)?.name ??
          "Agent")
      : delivery.type === "agent"
        ? "Device agent"
        : delivery.type === "notification"
          ? "Notification"
          : "Webhook";
  };
  function actions(row: TriggerResponse) {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Actions for ${row.label}`}
          >
            <MoreVertical className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => setEditing(row)}>
            Edit
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => {
              void update
                .mutateAsync({
                  id: row.id,
                  data: {
                    status: row.status === "active" ? "disabled" : "active",
                  },
                })
                .catch((e: unknown) => toast.error(message(e)));
            }}
          >
            {row.status === "active" ? "Pause" : "Resume"}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={row.status !== "active" || run.isPending}
            onClick={() => {
              void run
                .mutateAsync(row.id)
                .then(() => toast.success("Run queued"))
                .catch((e: unknown) => toast.error(message(e)));
            }}
          >
            Run now
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setHistory(row)}>
            Run history
          </DropdownMenuItem>
          <DropdownMenuItem
            className="text-destructive"
            onClick={() => setDeleting(row)}
          >
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
  }
  return (
    <div className="space-y-6">
      <PageHeader
        title="Automations"
        description="Scheduled work, reminders and webhook events for your agents."
        actions={
          <AddCtaButton
            label="Create automation"
            onClick={() => setEditing("new")}
          />
        }
      />
      <AutomationPreferences />
      {triggers.error && (
        <ErrorBanner
          message={triggers.error.message}
          onRetry={() => void triggers.refetch()}
        />
      )}
      {triggers.isPending ? (
        <Skeleton className="h-36 w-full" aria-label="Loading automations" />
      ) : rows.length === 0 ? (
        <p className="text-[13px] text-muted-foreground">
          No automations yet. Ask NyxBot to schedule a task or create one here.
        </p>
      ) : (
        <>
          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Automation</TableHead>
                  <TableHead>Target</TableHead>
                  <TableHead>Next run</TableHead>
                  <TableHead>Last run</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((row) => (
                  <TableRow key={row.id}>
                    <TableCell>
                      <button
                        className="text-left"
                        onClick={() => setHistory(row)}
                      >
                        {row.label}
                        <span className="block text-[11px] text-muted-foreground">
                          {row.source ?? "webhook"}
                        </span>
                      </button>
                    </TableCell>
                    <TableCell>{target(row)}</TableCell>
                    <TableCell>
                      {formatAutomationTime(row.next_run_at, zone)}
                    </TableCell>
                    <TableCell className="capitalize">
                      {row.last_run?.outcome ?? "—"}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          row.status === "active" ? "success" : "secondary"
                        }
                      >
                        {row.status === "active" ? "Active" : "Paused"}
                      </Badge>
                      {row.pause_reason && (
                        <span className="block text-[11px]">
                          {row.pause_reason.replaceAll("_", " ")}
                        </span>
                      )}
                    </TableCell>
                    <TableCell>{actions(row)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
          <div className="space-y-3 md:hidden">
            {rows.map((row) => (
              <div
                key={row.id}
                className="rounded-xl border border-border/50 bg-card p-4"
              >
                <div className="flex items-center justify-between">
                  <button onClick={() => setHistory(row)}>{row.label}</button>
                  {actions(row)}
                </div>
                <p className="text-[12px] text-muted-foreground">
                  {target(row)} ·{" "}
                  {row.status === "active" ? "Active" : "Paused"}
                </p>
                <p className="text-[12px]">
                  Next: {formatAutomationTime(row.next_run_at, zone)}
                </p>
                <p className="text-[12px] capitalize">
                  Last run: {row.last_run?.outcome ?? "—"}
                </p>
                {row.pause_reason && (
                  <p className="text-[12px]">
                    {row.pause_reason.replaceAll("_", " ")}
                  </p>
                )}
              </div>
            ))}
          </div>
        </>
      )}
      <p className="text-[12px] text-muted-foreground">
        <Link className="underline" to="/triggers">
          Webhook secrets and delivery replay
        </Link>
      </p>
      {setup.error && <ErrorBanner message={setup.error.message} />}
      {search.setup && setup.isPending && (
        <Skeleton className="h-24 w-full" aria-label="Loading setup" />
      )}
      {editing && (!search.setup || setup.data) && (
        <AutomationEditor
          row={editing === "new" ? undefined : editing}
          prefill={setup.data}
          watchId={search.setup}
          agentId={search.agent}
          onClose={() => setEditing(null)}
          onSecrets={setSecrets}
        />
      )}
      {history && (
        <AutomationHistory row={history} onClose={() => setHistory(null)} />
      )}
      <OneTimeSecretDialog
        open={secrets.length > 0}
        onOpenChange={(open) => {
          if (!open) setSecrets([]);
        }}
        values={secrets}
        title="Automation secrets"
        description="Save these secrets now. They are shown only once."
      />
      <Dialog
        open={Boolean(deleting)}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {deleting?.label}?</DialogTitle>
            <DialogDescription>
              Future runs stop. Existing run threads remain available.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleting(null)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => {
                if (deleting)
                  void remove
                    .mutateAsync(deleting.id)
                    .then(() => setDeleting(null))
                    .catch((e: unknown) => toast.error(message(e)));
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export function AutomationEditor({
  row,
  prefill,
  watchId,
  agentId,
  onClose,
  onSecrets,
}: {
  readonly row?: TriggerResponse;
  readonly prefill?: AutomationSetup;
  readonly watchId?: string;
  readonly agentId?: string;
  readonly onClose: () => void;
  readonly onSecrets: (values: readonly OneTimeSecretValue[]) => void;
}) {
  const settings = useNyxBotSettings();
  const saveSettings = useUpdateNyxBotSettings();
  const agents = useNyxBotAgents();
  const create = useCreateTrigger();
  const update = useUpdateTrigger();
  const [error, setError] = useState<string>();
  const [defaults] = useState(() => {
    const value = automationDefaults(
      settings.data?.timezone ?? browserTimezone(),
      row,
    );
    if (!row) {
      value.agent_id = agentId ?? "";
      if (prefill) {
        value.source = "webhook";
        value.label = prefill.label;
        value.agent_id = prefill.agent_id;
        value.instruction = prefill.instruction;
        value.confirmation_policy = prefill.confirmation_policy;
        value.thread_policy = prefill.thread_policy ?? "default";
      }
    }
    return value;
  });
  const form = useAppForm<AutomationForm>({
    resolver: zodResolver(automationFormSchema),
    defaultValues: defaults,
  });
  const ownerTimezone = settings.data?.timezone;
  const hasCalendarTimezone = row?.schedule?.kind === "cron";
  useEffect(() => {
    if (
      ownerTimezone &&
      !hasCalendarTimezone &&
      !form.getFieldState("timezone").isDirty
    ) {
      form.setValue("timezone", ownerTimezone, {
        shouldDirty: false,
        shouldTouch: false,
        shouldValidate: false,
      });
    }
  }, [form, ownerTimezone, hasCalendarTimezone]);
  const values = form.watch();
  const chats = useAutomationChats(values.deliver_type === "chat");
  const [spec, setSpec] = useState<ScheduleSpec>();
  const serialized = JSON.stringify(values);
  useEffect(() => {
    const timer = setTimeout(() => {
      try {
        const v = JSON.parse(serialized) as AutomationForm;
        setSpec(v.source === "schedule" ? formSchedule(v) : undefined);
      } catch {
        setSpec(undefined);
      }
    }, 350);
    return () => clearTimeout(timer);
  }, [serialized]);
  const preview = useSchedulePreview(spec);
  function input(name: keyof AutomationForm, label: string, type = "text") {
    return (
      <label className="block space-y-1 text-[12px]" key={name}>
        {label}
        <Input
          type={type}
          disabled={Boolean(row) && name === "signature_header"}
          {...form.register(name, { valueAsNumber: name === "amount" })}
        />
        {form.formState.errors[name] && (
          <span role="alert" className="text-destructive">
            {form.formState.errors[name]?.message}
          </span>
        )}
      </label>
    );
  }
  function select(
    name: keyof AutomationForm,
    label: string,
    options: readonly (readonly [string, string])[],
    disabled = false,
  ) {
    return (
      <div className="block space-y-1 text-[12px]">
        {label}
        <Select
          value={String(form.watch(name)) || "__empty"}
          onValueChange={(value) =>
            form.setValue(
              name,
              value === "__empty" ? "" : (value as AutomationForm[typeof name]),
            )
          }
          disabled={disabled}
        >
          <SelectTrigger aria-label={label}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {options.map(([value, text]) => (
              <SelectItem key={value || "__empty"} value={value || "__empty"}>
                {text}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {form.formState.errors[name] && (
          <span role="alert" className="text-destructive">
            {form.formState.errors[name]?.message}
          </span>
        )}
      </div>
    );
  }
  function dateTime(
    name: "at" | "anchor" | "start" | "end",
    label: string,
    optional: boolean,
  ) {
    return (
      <AutomationDateTime
        timezone={values.timezone}
        label={label}
        optional={optional}
        value={values[name]}
        onChange={(value) => form.setValue(name, value)}
      />
    );
  }
  async function save(v: AutomationForm) {
    try {
      setError(undefined);
      if (!settings.data?.timezone)
        await saveSettings.mutateAsync({
          timezone: v.timezone || browserTimezone(),
        });
      const request = automationRequest(v);
      if (row) {
        const result = await update.mutateAsync({
          id: row.id,
          data: {
            label: request.label,
            delivery: request.delivery,
            schedule: request.schedule,
            overlap: request.overlap,
          },
        });
        if (result.delivery_signing_secret)
          onSecrets([
            {
              label: "Delivery signing secret",
              value: result.delivery_signing_secret,
            },
          ]);
      } else {
        const result = await create.mutateAsync({
          ...request,
          ...(watchId ? { watch_id: watchId } : {}),
        });
        const secrets: OneTimeSecretValue[] = [];
        if (result.trigger.inbound_url)
          secrets.push({
            label: "Inbound URL",
            value: result.trigger.inbound_url,
          });
        if (result.secret)
          secrets.push({ label: "Inbound secret", value: result.secret });
        if (result.delivery_signing_secret)
          secrets.push({
            label: "Delivery signing secret",
            value: result.delivery_signing_secret,
          });
        if (secrets.length) onSecrets(secrets);
      }
      toast.success(row ? "Automation updated" : "Automation created");
      onClose();
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {row ? "Edit automation" : "Create automation"}
          </DialogTitle>
          <DialogDescription>
            Choose when to run, what to do and where the answer arrives.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-4" onSubmit={form.handleSubmit(save)}>
            {input("label", "Label")}
            {select(
              "source",
              "Source",
              [
                ["schedule", "Schedule"],
                ["webhook", "Webhook"],
              ],
              Boolean(row),
            )}
            {values.source === "schedule" ? (
              <div className="space-y-3 rounded-lg border border-border p-3">
                <TimezoneSelect
                  label="IANA timezone"
                  value={values.timezone}
                  onChange={(zone) => form.setValue("timezone", zone)}
                />
                {select("kind", "Schedule type", [
                  ["cron", "Calendar / cron"],
                  ["every", "Every interval"],
                  ["at", "Once"],
                ])}
                {values.kind === "cron" ? (
                  <>
                    <div className="space-y-1 text-[12px]">
                      <span>Preset</span>
                      <Select
                        onValueChange={(value) =>
                          form.setValue("expression", value)
                        }
                      >
                        <SelectTrigger aria-label="Preset">
                          <SelectValue placeholder="Choose a preset…" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="0 8 * * 1-5">
                            Weekdays at 8:00
                          </SelectItem>
                          <SelectItem value="0 9 * * *">
                            Daily at 9:00
                          </SelectItem>
                          <SelectItem value="0 17 * * 5">
                            Friday at 17:00
                          </SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    {input("expression", "Cron expression")}
                    <p className="text-[11px] text-muted-foreground">
                      Minute · hour · day · month · weekday. Missing DST times
                      run once at the first valid time after the gap; repeated
                      times use the earlier instant.
                    </p>
                  </>
                ) : values.kind === "every" ? (
                  <>
                    {input("amount", "Interval amount", "number")}
                    {select("unit", "Interval unit", [
                      ["minutes", "Minutes"],
                      ["hours", "Hours"],
                      ["days", "Days"],
                    ])}
                    {dateTime("anchor", "Anchor", false)}
                  </>
                ) : (
                  dateTime("at", "Run at", false)
                )}
                <div aria-live="polite" className="text-[12px]">
                  <strong>Next runs</strong>
                  {preview.isFetching ? (
                    <Skeleton
                      className="h-12 w-full"
                      aria-label="Calculating next runs"
                    />
                  ) : preview.error ? (
                    <p role="alert" className="text-destructive">
                      {preview.error.message}
                    </p>
                  ) : (
                    <>
                      <p>{preview.data?.description}</p>
                      <ul>
                        {preview.data?.next_runs.map((time) => (
                          <li key={time}>
                            {formatAutomationTime(
                              time,
                              settings.data?.timezone ?? values.timezone,
                            )}
                          </li>
                        ))}
                      </ul>
                    </>
                  )}
                </div>
                <details>
                  <summary className="cursor-pointer text-[12px]">
                    Start, end and run limits
                  </summary>
                  <div className="mt-3 space-y-3">
                    {dateTime("start", "Start", true)}
                    {dateTime("end", "End", true)}
                    {input("max_runs", "Maximum occurrences", "number")}
                    {input(
                      "grace_seconds",
                      "Grace window in seconds (optional)",
                      "number",
                    )}
                  </div>
                </details>
              </div>
            ) : (
              <>
                {select(
                  "verification_mode",
                  "Inbound verification",
                  [
                    ["bearer", "Bearer token"],
                    ["query", "Query token"],
                    ["hmac", "HMAC-SHA256"],
                  ],
                  Boolean(row),
                )}
                {values.verification_mode === "hmac" &&
                  input("signature_header", "Signature header")}
              </>
            )}
            {select("delivery_type", "Delivery", [
              ["assistant", "NyxBot / specialist"],
              ["notification", "Notification reminder"],
              ["webhook", "Webhook"],
              ["agent", "Device agent"],
            ])}
            {values.delivery_type === "assistant" ? (
              <>
                {select("agent_id", "Target agent", [
                  ["", "Choose an agent…"],
                  ...(agents.data?.agents
                    .filter((a) => a.status !== "destroyed")
                    .map((a) => [a.id, a.display_name ?? a.name] as const) ??
                    []),
                ])}
                {values.source === "webhook" && (
                  <>
                    {select("confirmation_policy", "Confirm webhook actions", [
                      ["changes", "Every changing action (recommended)"],
                      ["destructive", "Only destructive actions"],
                    ])}
                    {values.confirmation_policy === "destructive" && (
                      <p role="alert" className="text-[12px] text-warning">
                        Untrusted webhook content can cause changes using your
                        account without confirmation. Choose this only for event
                        sources you trust.
                      </p>
                    )}
                  </>
                )}
                {select("thread_policy", "Thread", [
                  [
                    "default",
                    values.source === "webhook"
                      ? "Default (dedicated thread)"
                      : "Default (NyxBot home, specialist dedicated)",
                  ],
                  ["home", "Home thread"],
                  ["dedicated", "One thread for this automation"],
                  ["new", "New thread each run"],
                ])}
                {values.source === "webhook" &&
                  values.thread_policy === "home" && (
                    <p role="alert" className="text-[12px] text-warning">
                      Untrusted webhook content will remain in your home thread
                      and can influence later owner turns, including private
                      channel chats, with full account authority. Webhook
                      confirmations do not protect those later turns. Choose
                      this only if you accept that risk.
                    </p>
                  )}
                <label className="block space-y-1 text-[12px]">
                  Instruction
                  <Textarea {...form.register("instruction")} />
                  {form.formState.errors.instruction && (
                    <span role="alert" className="text-destructive">
                      {form.formState.errors.instruction.message}
                    </span>
                  )}
                </label>
                {select("deliver_type", "Send result to", [
                  ["thread", "Web thread"],
                  ["chat", "Channel chat"],
                  ["notification", "Notification"],
                ])}
                {values.deliver_type === "chat" &&
                  select("chat_id", "Chat (posting must be allowed)", [
                    ["", "Choose a chat…"],
                    ...(chats.data?.map(
                      (chat) =>
                        [
                          chat.id,
                          `${chat.bot_label}: ${chat.title ?? chat.kind ?? "Chat"}`,
                        ] as const,
                    ) ?? []),
                  ])}
              </>
            ) : values.delivery_type === "webhook" ? (
              input("webhook_url", "Delivery HTTPS URL")
            ) : values.delivery_type === "agent" ? (
              input("conversation_id", "Device conversation ID")
            ) : null}
            {select("overlap", "If the previous run is still active", [
              ["skip", "Skip"],
              ["queue", "Queue one"],
            ])}
            {error && (
              <p role="alert" className="text-[12px] text-destructive">
                {error}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={
                  create.isPending ||
                  update.isPending ||
                  (Boolean(row) && !form.formState.isDirty)
                }
              >
                Save automation
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}

function AutomationHistory({
  row,
  onClose,
}: {
  readonly row: TriggerResponse;
  readonly onClose: () => void;
}) {
  const settings = useNyxBotSettings();
  const zone = settings.data?.timezone ?? browserTimezone();
  const history = useAutomationRuns(row.id);
  const runs = history.data?.pages.flatMap((page) => page.runs) ?? [];
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{row.label}: run history</DialogTitle>
          <DialogDescription>
            Up to 1,000 recent runs are retained for 90 days.
          </DialogDescription>
        </DialogHeader>
        {history.error ? (
          <ErrorBanner message={history.error.message} />
        ) : history.isPending ? (
          <Skeleton className="h-24 w-full" aria-label="Loading runs" />
        ) : !runs.length ? (
          <p>No runs yet.</p>
        ) : (
          <ul className="space-y-3">
            {runs.map((run) => (
              <li
                key={run.id}
                className="rounded-lg border border-border p-3 text-[12px]"
              >
                <strong className="capitalize">{run.outcome}</strong> ·{" "}
                {formatAutomationTime(run.scheduled_at, zone)}
                {run.missed ? (
                  <p>
                    Missed {run.missed.count} occurrences between{" "}
                    {formatAutomationTime(run.missed.from, zone)} and{" "}
                    {formatAutomationTime(run.missed.through, zone)}.
                  </p>
                ) : (
                  run.reason && <p>{run.reason.replaceAll("_", " ")}</p>
                )}
                {run.duration_ms !== null && (
                  <p>{(run.duration_ms / 1000).toFixed(1)} seconds</p>
                )}
                {run.thread_id && (
                  <Link
                    className="text-primary underline"
                    to="/assistant"
                    search={{ c: run.thread_id }}
                  >
                    Open thread
                  </Link>
                )}
              </li>
            ))}
          </ul>
        )}
        {history.hasNextPage && (
          <Button
            disabled={history.isFetchingNextPage}
            onClick={() => void history.fetchNextPage()}
          >
            Load older runs
          </Button>
        )}
      </DialogContent>
    </Dialog>
  );
}
