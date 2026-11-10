import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { formatCapacityWait } from "@/lib/assistant/capacity-wait";
import { storedError } from "@/lib/assistant/nyxagent-transport";
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
  const { t } = useUiTranslation();
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
          t("Agent"))
      : delivery.type === "agent"
        ? t("Device agent")
        : delivery.type === "notification"
          ? t("Notification")
          : t("Webhook");
  };
  function actions(row: TriggerResponse) {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={t("Actions for {{label}}", { label: row.label })}
          >
            <MoreVertical className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => setEditing(row)}>{t("Edit")}</DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => {
              void update
                .mutateAsync({
                  id: row.id,
                  data: {
                    status: row.status === "active" ? "disabled" : "active",
                  },
                })
                .catch((e: unknown) => toast.error(translateErrorMessage(message(e))));
            }}
          >
            {row.status === "active" ? t("Pause") : t("Resume")}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={row.status !== "active" || run.isPending}
            onClick={() => {
              void run
                .mutateAsync(row.id)
                .then(() => toast.success(translateUi("Run queued")))
                .catch((e: unknown) => toast.error(translateErrorMessage(message(e))));
            }}
          >{t("Run now")}</DropdownMenuItem>
          <DropdownMenuItem onClick={() => setHistory(row)}>{t("Run history")}</DropdownMenuItem>
          <DropdownMenuItem
            className="text-destructive"
            onClick={() => setDeleting(row)}
          >{t("Delete")}</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
  }
  return (
    <div className="space-y-6">
      <PageHeader
        title={t("Automations")}
        description={t("Scheduled work, reminders and webhook events for your agents.")}
        actions={
          <AddCtaButton
            label={t("Create automation")}
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
        <Skeleton className="h-36 w-full" aria-label={t("Loading automations")} />
      ) : rows.length === 0 ? (
        <p className="text-13 text-muted-foreground">{t("No automations yet. Ask NyxBot to schedule a task or create one here.")}</p>
      ) : (
        <>
          <div className="hidden overflow-hidden rounded-xl border border-border/50 bg-card md:block">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("Automation")}</TableHead>
                  <TableHead>{t("Target")}</TableHead>
                  <TableHead>{t("Next run")}</TableHead>
                  <TableHead>{t("Last run")}</TableHead>
                  <TableHead>{t("Status")}</TableHead>
                  <TableHead>{t("Actions")}</TableHead>
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
                        <span className="block text-11 text-muted-foreground">
                          {row.source ?? t("webhook")}
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
                        {row.status === "active" ? t("Active") : t("Paused")}
                      </Badge>
                      {row.pause_reason && (
                        <span className="block text-11">
                          {automationPauseReason(row.pause_reason)}
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
                <p className="text-12 text-muted-foreground">
                  {target(row)} ·{" "}
                  {row.status === "active" ? t("Active") : t("Paused")}
                </p>
                <p className="text-12">{t("Next: ")}{formatAutomationTime(row.next_run_at, zone)}
                </p>
                <p className="text-12 capitalize">{t("Last run: ")}{row.last_run?.outcome ?? "—"}
                </p>
                {row.pause_reason && (
                  <p className="text-12">
                    {automationPauseReason(row.pause_reason)}
                  </p>
                )}
              </div>
            ))}
          </div>
        </>
      )}
      <p className="text-12 text-muted-foreground">
        <Link className="underline" to="/triggers">{t("Webhook secrets and delivery replay")}</Link>
      </p>
      {setup.error && <ErrorBanner message={setup.error.message} />}
      {search.setup && setup.isPending && (
        <Skeleton className="h-24 w-full" aria-label={t("Loading setup")} />
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
        title={t("Automation secrets")}
        description={t("Save these secrets now. They are shown only once.")}
      />
      <Dialog
        open={Boolean(deleting)}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Delete ")}{deleting?.label}?</DialogTitle>
            <DialogDescription>{t("Future runs stop. Existing run threads remain available.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleting(null)}>{t("Cancel")}</Button>
            <Button
              variant="destructive"
              disabled={remove.isPending}
              onClick={() => {
                if (deleting)
                  void remove
                    .mutateAsync(deleting.id)
                    .then(() => setDeleting(null))
                    .catch((e: unknown) => toast.error(translateErrorMessage(message(e))));
              }}
            >{t("Delete")}</Button>
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
  const { t } = useUiTranslation();
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
      <label className="block space-y-1 text-12" key={name}>
        {t(label)}
        <Input
          type={type}
          disabled={Boolean(row) && name === "signature_header"}
          {...form.register(name, { valueAsNumber: name === "amount" })}
        />
        {form.formState.errors[name] && (
          <span role="alert" className="text-destructive">
            {translateUi(form.formState.errors[name]?.message ?? "")}
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
      <div className="block space-y-1 text-12">
        {t(label)}
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
          <SelectTrigger aria-label={t(label)}>
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
            {translateUi(form.formState.errors[name]?.message ?? "")}
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
        label={t(label)}
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
            {row ? t("Edit automation") : t("Create automation")}
          </DialogTitle>
          <DialogDescription>{t("Choose when to run, what to do and where the answer arrives.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="space-y-4" onSubmit={form.handleSubmit(save)}>
            {input("label", t("Label"))}
            {select(
              "source",
              t("Source"),
              [
                ["schedule", t("Schedule")],
                ["webhook", t("Webhook")],
              ],
              Boolean(row),
            )}
            {values.source === "schedule" ? (
              <div className="space-y-3 rounded-lg border border-border p-3">
                <TimezoneSelect
                  label={t("IANA timezone")}
                  value={values.timezone}
                  onChange={(zone) => form.setValue("timezone", zone)}
                />
                {select("kind", t("Schedule type"), [
                  ["cron", t("Calendar / cron")],
                  ["every", t("Every interval")],
                  ["at", t("Once")],
                ])}
                {values.kind === "cron" ? (
                  <>
                    <div className="space-y-1 text-12">
                      <span>{t("Preset")}</span>
                      <Select
                        onValueChange={(value) =>
                          form.setValue("expression", value)
                        }
                      >
                        <SelectTrigger aria-label={t("Preset")}>
                          <SelectValue placeholder={t("Choose a preset…")} />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="0 8 * * 1-5">{t("Weekdays at 8:00")}</SelectItem>
                          <SelectItem value="0 9 * * *">{t("Daily at 9:00")}</SelectItem>
                          <SelectItem value="0 17 * * 5">{t("Friday at 17:00")}</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                    {input("expression", t("Cron expression"))}
                    <p className="text-11 text-muted-foreground">{t("Minute · hour · day · month · weekday. Missing DST times run once at the first valid time after the gap; repeated times use the earlier instant.")}</p>
                  </>
                ) : values.kind === "every" ? (
                  <>
                    {input("amount", t("Interval amount"), "number")}
                    {select("unit", t("Interval unit"), [
                      ["minutes", t("Minutes")],
                      ["hours", t("Hours")],
                      ["days", t("Days")],
                    ])}
                    {dateTime("anchor", t("Anchor"), false)}
                  </>
                ) : (
                  dateTime("at", t("Run at"), false)
                )}
                <div aria-live="polite" className="text-12">
                  <strong>{t("Next runs")}</strong>
                  {preview.isFetching ? (
                    <Skeleton
                      className="h-12 w-full"
                      aria-label={t("Calculating next runs")}
                    />
                  ) : preview.error ? (
                    <p role="alert" className="text-destructive">
                      {translateUi(preview.error.message ?? "")}
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
                  <summary className="cursor-pointer text-12">{t("Start, end and run limits")}</summary>
                  <div className="mt-3 space-y-3">
                    {dateTime("start", t("Start"), true)}
                    {dateTime("end", t("End"), true)}
                    {input("max_runs", t("Maximum occurrences"), "number")}
                    {input(
                      "grace_seconds",
                      t("Grace window in seconds (optional)"),
                      "number",
                    )}
                  </div>
                </details>
              </div>
            ) : (
              <>
                {select(
                  "verification_mode",
                  t("Inbound verification"),
                  [
                    ["bearer", t("Bearer token")],
                    ["query", t("Query token")],
                    ["hmac", t("HMAC-SHA256")],
                  ],
                  Boolean(row),
                )}
                {values.verification_mode === "hmac" &&
                  input("signature_header", t("Signature header"))}
              </>
            )}
            {select("delivery_type", t("Delivery"), [
              ["assistant", t("NyxBot / specialist")],
              ["notification", t("Notification reminder")],
              ["webhook", t("Webhook")],
              ["agent", t("Device agent")],
            ])}
            {values.delivery_type === "assistant" ? (
              <>
                {select("agent_id", t("Target agent"), [
                  ["", t("Choose an agent…")],
                  ...(agents.data?.agents
                    .filter((a) => a.status !== "destroyed")
                    .map((a) => [a.id, a.display_name ?? a.name] as const) ??
                    []),
                ])}
                {values.source === "webhook" && (
                  <>
                    {select("confirmation_policy", t("Confirm webhook actions"), [
                      ["changes", t("Every changing action (recommended)")],
                      ["destructive", t("Only destructive actions")],
                    ])}
                    {values.confirmation_policy === "destructive" && (
                      <p role="alert" className="text-12 text-warning">{t("Untrusted webhook content can cause changes using your account without confirmation. Choose this only for event sources you trust.")}</p>
                    )}
                  </>
                )}
                {select("thread_policy", t("Thread"), [
                  [
                    "default",
                    values.source === "webhook"
                      ? t("Default (dedicated thread)")
                      : t("Default (NyxBot home, specialist dedicated)"),
                  ],
                  ["home", t("Home thread")],
                  ["dedicated", t("One thread for this automation")],
                  ["new", t("New thread each run")],
                ])}
                {values.source === "webhook" &&
                  values.thread_policy === "home" && (
                    <p role="alert" className="text-12 text-warning">{t("Untrusted webhook content will remain in your home thread and can influence later owner turns, including private channel chats, with full account authority. Webhook confirmations do not protect those later turns. Choose this only if you accept that risk.")}</p>
                  )}
                <label className="block space-y-1 text-12">{t("Instruction")}<Textarea {...form.register("instruction")} />
                  {form.formState.errors.instruction && (
                    <span role="alert" className="text-destructive">
                      {translateUi(form.formState.errors.instruction.message ?? "")}
                    </span>
                  )}
                </label>
                {select("deliver_type", t("Send result to"), [
                  ["thread", t("Web thread")],
                  ["chat", t("Channel chat")],
                  ["notification", t("Notification")],
                ])}
                {values.deliver_type === "chat" &&
                  select("chat_id", t("Chat (posting must be allowed)"), [
                    ["", t("Choose a chat…")],
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
              input("webhook_url", t("Delivery HTTPS URL"))
            ) : values.delivery_type === "agent" ? (
              input("conversation_id", t("Device conversation ID"))
            ) : null}
            {select("overlap", t("If the previous run is still active"), [
              ["skip", t("Skip")],
              ["queue", t("Queue one")],
            ])}
            {error && (
              <p role="alert" className="text-12 text-destructive">
                {translateUi(error)}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={onClose}>{t("Cancel")}</Button>
              <Button
                type="submit"
                variant="primary"
                disabled={
                  create.isPending ||
                  update.isPending ||
                  (Boolean(row) && !form.formState.isDirty)
                }
              >{t("Save automation")}</Button>
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
  const { t } = useUiTranslation();
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
          <DialogTitle>{row.label}{t(": run history")}</DialogTitle>
          <DialogDescription>{t("Up to 1,000 recent runs are retained for 90 days.")}</DialogDescription>
        </DialogHeader>
        {history.error ? (
          <ErrorBanner message={history.error.message} />
        ) : history.isPending ? (
          <Skeleton className="h-24 w-full" aria-label={t("Loading runs")} />
        ) : !runs.length ? (
          <p>{t("No runs yet.")}</p>
        ) : (
          <ul className="space-y-3">
            {runs.map((run) => (
              <li
                key={run.id}
                className="rounded-lg border border-border p-3 text-12"
              >
                <strong className="capitalize">{run.outcome}</strong> ·{" "}{t("Scheduled ")}{formatAutomationTime(run.scheduled_at, zone, true)}
                {run.started_at && <p>{t("Started ")}{formatAutomationTime(run.started_at, zone, true)}</p>}
                {run.capacity_wait_ms != null && run.capacity_wait_ms > 0 && <p>{t("waited ")}{formatCapacityWait(run.capacity_wait_ms)}{t(" for capacity")}</p>}
                {run.missed ? (
                  <p>{t("Missed ")}{run.missed.count}{t(" occurrences between")}{" "}
                    {formatAutomationTime(run.missed.from, zone)}{t(" and")}{" "}
                    {formatAutomationTime(run.missed.through, zone)}.
                  </p>
                ) : (
                  run.reason && <p>{run.reason.replaceAll("_", " ")}</p>
                )}
                {run.duration_ms !== null && (
                  <p>{(run.duration_ms / 1000).toFixed(1)}{t(" seconds")}</p>
                )}
                {run.thread_id && (
                  <Link
                    className="text-primary-text underline"
                    to="/assistant"
                    search={{ c: run.thread_id }}
                  >{t("Open thread")}</Link>
                )}
              </li>
            ))}
          </ul>
        )}
        {history.hasNextPage && (
          <Button
            disabled={history.isFetchingNextPage}
            onClick={() => void history.fetchNextPage()}
          >{t("Load older runs")}</Button>
        )}
      </DialogContent>
    </Dialog>
  );
}

function automationPauseReason(reason: string): string {
  if (["insufficient_credits", "wallet_suspended", "plan_entitlement_required", "credential_invalid", "model_unavailable", "model_not_configured"].includes(reason)) return storedError(reason);
  return reason.replaceAll("_", " ");
}
