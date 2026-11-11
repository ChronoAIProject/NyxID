import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { Label } from "@/components/ui/label";
import { describeChanges, labelValue } from "@/lib/grouped-form-changes";
import { TEXT_STYLES } from "@/lib/typography";
import { formatCapacityWait } from "@/lib/assistant/capacity-wait";
import { storedError } from "@/lib/assistant/nyxagent-transport";
import { Link, useSearch } from "@tanstack/react-router";
import { parseAutomationSearch } from "@/lib/automation-search";
import { browserTimezone, formatAutomationTime } from "@/lib/automation-time";
import { TimezoneSelect } from "@/components/shared/timezone-select";
import { AutomationDateTime } from "@/components/shared/automation-date-time";
import {
  Select,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
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
  useCancelAutomationRun,
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
import { WorkspacePageHeader } from "@/components/assistant/workspace-page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  OneTimeSecretDialog,
  type OneTimeSecretValue,
} from "@/components/shared/one-time-secret-dialog";
import { Button } from "@/components/ui/button";
import { Form, useAppForm } from "@/components/ui/form";
import {
  Dialog,
  DialogContent,
  DialogBody,
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
  GroupedList,
  GroupedRow,
  GroupedInput,
  GroupedTextarea,
  GroupedSelectTrigger,
  GroupedDisclosure,
  GroupedFooter,
} from "@/components/ui/grouped-list";
import { GroupedDialogActions } from "@/components/ui/grouped-form";
import {
  GroupedPage,
  GroupedDialog,
  GroupedDialogContent,
} from "@/components/ui/grouped-dialog";
import {
  SegmentedControl,
  SegmentedControlList,
  SegmentedControlTrigger,
} from "@/components/ui/segmented-control";

const message = (e: unknown) =>
  e instanceof Error ? e.message : "Could not save automation";

export function AutomationsPage() {
  useUiTranslation();
  return (
    <GroupedPage>
      <AutomationsContent />
    </GroupedPage>
  );
}

function AutomationsContent() {
  useUiTranslation();
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
          translateUi("Agent"))
      : delivery.type === "agent"
        ? translateUi("Device agent")
        : delivery.type === "notification"
          ? translateUi("Notification")
          : translateUi("Webhook");
  };
  function actions(row: TriggerResponse) {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={translateUi("Actions for {{value1}}", { value1: row.label })}
          >
            <MoreVertical className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => setEditing(row)}>{translateUi("Edit")}</DropdownMenuItem>
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
            {row.status === "active" ? translateUi("Pause") : translateUi("Resume")}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={row.status !== "active" || run.isPending}
            onClick={() => {
              void run
                .mutateAsync(row.id)
                .then(() => toast.success(translateUi("Run queued")))
                .catch((e: unknown) => toast.error(translateErrorMessage(message(e))));
            }}
          >{translateUi("Run now")}</DropdownMenuItem>
          <DropdownMenuItem onClick={() => setHistory(row)}>{translateUi("Run history")}</DropdownMenuItem>
          <DropdownMenuItem
            className="text-destructive"
            onClick={() => setDeleting(row)}
          >{translateUi("Delete")}</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
  }
  return (
    <div className="shell-page-stack">
      <WorkspacePageHeader
        column
        title={translateUi("Automations")}
        description={translateUi("Scheduled work, reminders and webhook events for your agents.")}
        actions={
          <AddCtaButton
            label={translateUi("Create automation")}
            onClick={() => setEditing("new")}
          />
        }
      />
      <AutomationPreferences title={translateUi("Settings")} />
      {triggers.error && (
        <ErrorBanner
          message={triggers.error.message}
          onRetry={() => void triggers.refetch()}
        />
      )}
      {triggers.isPending ? (
        <Skeleton className="h-36 w-full" aria-label={translateUi("Loading automations")} />
      ) : rows.length === 0 ? (
        <GroupedList
          className="max-w-2xl"
          footer={
            <Link className="underline" to="/triggers">{translateUi("Webhook secrets and delivery replay")}</Link>
          }
        >
          <GroupedRow
            title={translateUi("No automations yet")}
            description={translateUi("Ask NyxBot to schedule a task or create one here.")}
          />
        </GroupedList>
      ) : (
        <GroupedList
          title={translateUi("Your automations")}
          className="max-w-2xl"
          footer={
            <Link className="underline" to="/triggers">{translateUi("Webhook secrets and delivery replay")}</Link>
          }
        >
          {rows.map((row) => (
            <GroupedRow key={row.id}>
              <button
                type="button"
                aria-label={row.label}
                className="min-w-0 flex-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                onClick={() => setHistory(row)}
              >
                <span data-trigger-title="" className={`block break-words ${TEXT_STYLES.rowTitle}`}>
                  {row.label}
                </span>
                <span className="mt-0.5 block type-secondary text-text-tertiary">
                  {target(row)} ·{" "}
                  {row.source === "schedule" ? translateUi("Schedule") : translateUi("Webhook")} ·{" "}
                  <span
                    className={
                      row.status === "active" ? "text-success" : undefined
                    }
                  >
                    {row.status === "active" ? translateUi("Active") : translateUi("Paused")}
                  </span>
                </span>
                <span className="mt-1 block type-secondary text-text-tertiary">
                  {row.source === "webhook"
                    ? translateUi("Runs on webhook events")
                    : row.status !== "active"
                      ? translateUi("Schedule paused")
                      : row.next_run_at
                        ? translateUi("Next: {{value1}}", { value1: formatAutomationTime(row.next_run_at, zone) })
                        : translateUi("No run scheduled")} · {row.last_run?.outcome
                    ? translateUi("Last run: {{value1}}", { value1: row.last_run.outcome.replaceAll("_", " ") })
                    : translateUi("Not run yet")}
                </span>
                {row.last_run?.notification_delivery?.message && (
                  <span className="mt-1 block type-secondary text-warning">
                    {row.last_run.execution_outcome === "completed" ? translateUi("Task completed. ") : ""}{row.last_run.notification_delivery.message}
                  </span>
                )}
                {row.pause_reason && (
                  <span className="mt-1 block type-secondary text-warning">
                    {automationPauseReason(row.pause_reason)}
                  </span>
                )}
              </button>
              {actions(row)}
            </GroupedRow>
          ))}
        </GroupedList>
      )}
      {setup.error && <ErrorBanner message={setup.error.message} />}
      {search.setup && setup.isPending && (
        <Skeleton className="h-24 w-full" aria-label={translateUi("Loading setup")} />
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
        title={translateUi("Automation secrets")}
        description={translateUi("Save these secrets now. They are shown only once.")}
      />
      <Dialog
        open={Boolean(deleting)}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{translateUi("Delete ")}{deleting?.label}?</DialogTitle>
            <DialogDescription>{translateUi("Future runs stop. Existing run threads remain available.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleting(null)}>{translateUi("Cancel")}</Button>
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
            >{translateUi("Delete")}</Button>
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
  useUiTranslation();
  const settings = useNyxBotSettings();
  const saveSettings = useUpdateNyxBotSettings();
  const agents = useNyxBotAgents();
  const create = useCreateTrigger();
  const update = useUpdateTrigger();
  const [error, setError] = useState<string>();
  const [advanced, setAdvanced] = useState(false);
  const [prefillDirty, setPrefillDirty] = useState(Boolean(!row && prefill));
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
    mode: "onChange",
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
  function fieldError(name: keyof AutomationForm) {
    const error = form.formState.errors[name];
    return error ? (
      <GroupedFooter>
        <p
          id={`automation-${name}-error`}
          role="alert"
          className="text-destructive"
        >
          {error.message}
        </p>
      </GroupedFooter>
    ) : null;
  }
  function input(name: keyof AutomationForm, label: string, type = "text") {
    return (
      <GroupedRow key={name} asChild className="block space-y-0.5">
        <Label>
          <span className="text-text-tertiary">
            {translateUi(label)}
          </span>
          <GroupedInput
            type={type}
            aria-invalid={Boolean(form.formState.errors[name])}
            aria-describedby={`automation-${name}-error`}
            disabled={Boolean(row) && name === "signature_header"}
            {...form.register(name, { valueAsNumber: name === "amount" })}
          />
          {fieldError(name)}
        </Label>
      </GroupedRow>
    );
  }
  function select(
    name: keyof AutomationForm,
    label: string,
    options: readonly (readonly [string, string])[],
    disabled = false,
  ) {
    return (
      <>
        <GroupedRow
          title={translateUi(label)}
          accessory={
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
              <GroupedSelectTrigger
                aria-label={translateUi(label)}
                aria-describedby={`automation-${name}-error`}
                aria-invalid={Boolean(form.formState.errors[name])}
              >
                <SelectValue />
              </GroupedSelectTrigger>
              <SelectContent>
                {options.map(([value, text]) => (
                  <SelectItem key={value || "__empty"} value={value || "__empty"}>
                    {text}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        {fieldError(name)}
      </>
    );
  }
  function segmented(
    name: "source" | "kind",
    label: string,
    options: readonly (readonly [string, string])[],
    disabled = false,
  ) {
    return (
      <GroupedRow className="block space-y-2">
        <Label asChild><p>{label}</p></Label>
        <SegmentedControl
          value={values[name]}
          onValueChange={(value) =>
            form.setValue(name, value as AutomationForm[typeof name])
          }
        >
          <SegmentedControlList aria-label={label}>
            {options.map(([value, text]) => (
              <SegmentedControlTrigger
                key={value}
                value={value}
                disabled={disabled}
              >
                {text}
              </SegmentedControlTrigger>
            ))}
          </SegmentedControlList>
        </SegmentedControl>
      </GroupedRow>
    );
  }
  function dateTime(
    name: "at" | "anchor" | "start" | "end",
    label: string,
    optional: boolean,
  ) {
    return (
      <>
        <AutomationDateTime
          grouped
          errorId={`automation-${name}-error`}
          invalid={Boolean(form.formState.errors[name])}
          timezone={values.timezone}
          label={translateUi(label)}
          optional={optional}
          value={values[name]}
          onChange={(value) => form.setValue(name, value)}
        />
        {fieldError(name)}
      </>
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
      toast.success(translateUi(row ? "Automation updated" : "Automation created"));
      onClose();
    } catch (e) {
      setError(message(e));
    }
  }
  const busy = create.isPending || update.isPending || saveSettings.isPending;
  return (
    <GroupedDialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <GroupedDialogContent scrollMode="body" className="md:max-w-xl">
        <DialogHeader className="shrink-0 pr-6">
          <DialogTitle>
            {row ? translateUi("Edit automation") : translateUi("Create automation")}
          </DialogTitle>
          <DialogDescription>{translateUi("Choose when to run, what to do and where the answer arrives.")}</DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form className="min-h-0" onSubmit={form.handleSubmit(save)}>
            <DialogBody className="space-y-6 pb-4">
              <GroupedList title={translateUi("What")}>
                {input("label", translateUi("Label"))}
                {values.delivery_type === "assistant" && (
                  <GroupedRow asChild className="block space-y-0.5">
                    <Label>
                      <span className="text-text-tertiary">{translateUi("Instruction")}</span>
                      <GroupedTextarea
                        {...form.register("instruction")}
                        aria-invalid={Boolean(
                          form.formState.errors.instruction,
                        )}
                        aria-describedby="automation-instruction-error"
                      />
                      {fieldError("instruction")}
                    </Label>
                  </GroupedRow>
                )}
              </GroupedList>
              <GroupedList
                title={translateUi("When")}
                footer={
                  values.source === "schedule" && values.kind === "cron"
                    ? translateUi("Minute · hour · day · month · weekday. Missing DST times run once at the first valid time after the gap; repeated times use the earlier instant.")
                    : undefined
                }
              >
                {segmented(
                  "source",
                  "Source",
                  [
                    ["schedule", "Schedule"],
                    ["webhook", "Webhook"],
                  ],
                  Boolean(row),
                )}
                {values.source === "schedule" ? (
                  <>
                    {segmented("kind", "Schedule type", [
                      ["cron", "Calendar"],
                      ["every", "Every"],
                      ["at", "Once"],
                    ])}
                    {values.kind === "cron" ? (
                      <>
                        <GroupedRow
                          title={translateUi("Preset")}
                          accessory={
                            <Select
                              onValueChange={(value) =>
                                form.setValue("expression", value)
                              }
                            >
                              <GroupedSelectTrigger aria-label={translateUi("Preset")}>
                                <SelectValue placeholder={translateUi("Choose a preset…")} />
                              </GroupedSelectTrigger>
                              <SelectContent>
                                <SelectItem value="0 8 * * 1-5">{translateUi("Weekdays at 8:00")}</SelectItem>
                                <SelectItem value="0 9 * * *">{translateUi("Daily at 9:00")}</SelectItem>
                                <SelectItem value="0 17 * * 5">{translateUi("Friday at 17:00")}</SelectItem>
                              </SelectContent>
                            </Select>
                          }
                        />
                        {input("expression", translateUi("Cron expression"))}
                      </>
                    ) : values.kind === "every" ? (
                      <>
                        <GroupedRow
                          title={translateUi("Interval")}
                          accessory={
                            <div className="flex max-w-full flex-wrap items-center gap-2">
                              <GroupedInput
                                type="number"
                                className="w-16"
                                aria-label={translateUi("Interval amount")}
                                aria-invalid={Boolean(
                                  form.formState.errors.amount,
                                )}
                                aria-describedby="automation-amount-error"
                                {...form.register("amount", {
                                  valueAsNumber: true,
                                })}
                              />
                              <Select
                                value={values.unit}
                                onValueChange={(value) =>
                                  form.setValue(
                                    "unit",
                                    value as AutomationForm["unit"],
                                  )
                                }
                              >
                                <GroupedSelectTrigger
                                  aria-label={translateUi("Interval unit")}
                                  aria-invalid={Boolean(
                                    form.formState.errors.unit,
                                  )}
                                  aria-describedby="automation-unit-error"
                                >
                                  <SelectValue />
                                </GroupedSelectTrigger>
                                <SelectContent>
                                  {["minutes", "hours", "days"].map((unit) => (
                                    <SelectItem key={unit} value={unit}>
                                      {unit.charAt(0).toUpperCase() +
                                        unit.slice(1)}
                                    </SelectItem>
                                  ))}
                                </SelectContent>
                              </Select>
                            </div>
                          }
                        />
                        {fieldError("amount")}
                        {fieldError("unit")}
                        {dateTime("anchor", translateUi("Anchor"), false)}
                      </>
                    ) : (
                      dateTime("at", translateUi("Run at"), false)
                    )}
                    <TimezoneSelect
                      grouped
                      label={translateUi("IANA timezone")}
                      describedBy="automation-timezone-error"
                      invalid={Boolean(form.formState.errors.timezone)}
                      value={values.timezone}
                      onChange={(zone) => form.setValue("timezone", zone)}
                    />
                    {fieldError("timezone")}
                    <GroupedDisclosure
                      aria-expanded={advanced}
                      onClick={() => setAdvanced(!advanced)}
                    >{translateUi("Start, end and run limits")}</GroupedDisclosure>
                  </>
                ) : (
                  <>
                    {select(
                      "verification_mode",
                      translateUi("Inbound verification"),
                      [
                        ["bearer", "Bearer token"],
                        ["query", "Query token"],
                        ["hmac", "HMAC-SHA256"],
                      ],
                      Boolean(row),
                    )}
                    {values.verification_mode === "hmac" &&
                      input("signature_header", translateUi("Signature header"))}
                  </>
                )}
              </GroupedList>
              {values.source === "schedule" && advanced && (
                <GroupedList title={translateUi("Start, end and run limits")}>
                  {dateTime("start", translateUi("Start"), true)}
                  {dateTime("end", translateUi("End"), true)}
                  {input("max_runs", translateUi("Maximum occurrences"), "number")}
                  {input(
                    "grace_seconds",
                    translateUi("Grace window in seconds (optional)"),
                    "number",
                  )}
                </GroupedList>
              )}
              {values.source === "schedule" && (
                <GroupedList
                  title={translateUi("Next runs")}
                  aria-live="polite"
                  footer={
                    preview.error ? (
                      <p role="alert" className="text-destructive">
                        {preview.error.message}
                      </p>
                    ) : (
                      preview.data?.description
                    )
                  }
                >
                  {preview.isFetching ? (
                    <GroupedRow>
                      <Skeleton
                        className="h-8 w-full"
                        aria-label={translateUi("Calculating next runs")}
                      />
                    </GroupedRow>
                  ) : preview.data?.next_runs.length ? (
                    preview.data.next_runs.map((time) => (
                      <GroupedRow
                        key={time}
                        title={formatAutomationTime(
                          time,
                          settings.data?.timezone ?? values.timezone,
                        )}
                      />
                    ))
                  ) : (
                    <GroupedRow title={translateUi("Choose a valid schedule to preview its next runs")} />
                  )}
                </GroupedList>
              )}
              {values.delivery_type === "assistant" && (
                <GroupedList
                  title={translateUi("Who")}
                  footer={
                    <>
                      {values.source === "webhook" &&
                        values.confirmation_policy === "destructive" && (
                          <p role="alert" className="text-warning">{translateUi("Untrusted webhook content can cause changes using your account without confirmation. Choose this only for event sources you trust.")}</p>
                        )}
                      {values.source === "webhook" &&
                        values.thread_policy === "home" && (
                          <p role="alert" className="text-warning">{translateUi("Untrusted webhook content will remain in your home thread and can influence later owner turns, including private channel chats, with full account authority. Webhook confirmations do not protect those later turns. Choose this only if you accept that risk.")}</p>
                        )}
                    </>
                  }
                >
                  {select("agent_id", translateUi("Target agent"), [
                    ["", "Choose an agent…"],
                    ...(agents.data?.agents
                      .filter((a) => a.status !== "destroyed")
                      .map((a) => [a.id, a.display_name ?? a.name] as const) ??
                      []),
                  ])}
                  {select("thread_policy", translateUi("Thread"), [
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
                    select("confirmation_policy", translateUi("Confirm webhook actions"), [
                      ["changes", "Every changing action (recommended)"],
                      ["destructive", "Only destructive actions"],
                    ])}
                </GroupedList>
              )}
              <GroupedList title={translateUi("Deliver to")} footer={values.delivery_type === "assistant" && values.deliver_type === "notification" ? translateUi("Mobile push requires an enabled, registered device in the NyxID app. For Telegram reports, choose Channel chat; linking Telegram notifications does not enable push.") : undefined}>
                {select("delivery_type", translateUi("Delivery"), [
                  ["assistant", "NyxBot / specialist"],
                  ["notification", "Notification reminder"],
                  ["webhook", "Webhook"],
                  ["agent", "Device agent"],
                ])}
                {values.delivery_type === "assistant" ? (
                  <>
                    {select("deliver_type", translateUi("Send result to"), [
                      ["thread", "Web thread"],
                      ["chat", "Channel chat"],
                      ["notification", "Mobile push"],
                    ])}
                    {values.deliver_type === "chat" &&
                      select("chat_id", translateUi("Chat (posting must be allowed)"), [
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
                  input("webhook_url", translateUi("Delivery HTTPS URL"))
                ) : values.delivery_type === "agent" ? (
                  input("conversation_id", translateUi("Device conversation ID"))
                ) : null}
              </GroupedList>
              <GroupedList
                title={translateUi("If still running")}
                footer={
                  error && (
                    <p role="alert" className="text-destructive">
                      {translateUi(error)}
                    </p>
                  )
                }
              >
                {select("overlap", translateUi("If the previous run is still active"), [
                  ["skip", "Skip"],
                  ["queue", "Queue one"],
                ])}
              </GroupedList>
            </DialogBody>
            <GroupedDialogActions
              changes={() => describeChanges(form.formState.defaultValues, form.getValues(), [{ key: "label", label: "Label" }, { key: "instruction", label: "Instruction" }, { key: "source", label: "Source", format: labelValue({ schedule: "Schedule", webhook: "Webhook" }) }, { key: "kind", label: "Schedule type", format: labelValue({ cron: "Calendar", every: "Every", at: "Once" }) }, { key: "expression", label: "Cron expression" }, { key: "timezone", label: "Timezone" }, { key: "amount", label: "Interval" }, { key: "unit", label: "Interval unit", format: labelValue({ minutes: "Minutes", hours: "Hours", days: "Days" }) }, { key: "anchor", label: "Anchor" }, { key: "at", label: "Run at" }, { key: "start", label: "Start" }, { key: "end", label: "End" }, { key: "max_runs", label: "Maximum runs" }, { key: "grace_seconds", label: "Grace period (seconds)" }, { key: "delivery_type", label: "Delivery type", format: labelValue({ assistant: "Assistant", notification: "Notification", webhook: "Webhook", agent: "Device agent" }) }, { key: "agent_id", label: "Target agent", format: (value) => { const agent = agents.data?.agents.find((agent) => agent.id === value); return agent?.display_name || agent?.name || String(value || "None"); } }, { key: "thread_policy", label: "Thread", format: labelValue({ default: values.source === "webhook" ? "Default (dedicated thread)" : "Default (NyxBot home, specialist dedicated)", home: "Home thread", dedicated: "One thread for this automation", new: "New thread each run" }) }, { key: "confirmation_policy", label: "Confirmation policy", format: labelValue({ changes: "Every changing action (recommended)", destructive: "Only destructive actions" }) }, { key: "deliver_type", label: "Send result to", format: labelValue({ thread: "Web thread", chat: "Channel chat", notification: "Mobile push" }) }, { key: "chat_id", label: "Chat", format: (value) => { const chat = chats.data?.find((chat) => chat.id === value); return chat ? `${chat.bot_label}: ${chat.title ?? chat.kind ?? "Chat"}` : value ? "Saved chat" : "None"; } }, { key: "overlap", label: "If still running", format: labelValue({ skip: "Skip", queue: "Queue one" }) }, { key: "webhook_url", label: "Webhook URL", format: (value) => value === form.formState.defaultValues?.webhook_url ? "Saved destination (hidden)" : "New destination (hidden)" }, { key: "conversation_id", label: "Device conversation" }, { key: "verification_mode", label: "Verification method", format: labelValue({ bearer: "Bearer token", query: "Query token", hmac: "HMAC signature" }) }, { key: "signature_header", label: "Signature header" }])}
              intent={row ? "edit" : "create"}
              disabled={!form.formState.isValid}
              label={translateUi("automation")}
              dirty={form.formState.isDirty || prefillDirty}
              pending={busy}
              save={form.handleSubmit(save)}
              discard={() => {
                form.reset();
                setPrefillDirty(false);
                setError(undefined);
                if (prefillDirty) onClose();
              }}
              saveLabel={translateUi("Save automation")}
            />
          </form>
        </Form>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}

function AutomationHistory({
  row,
  onClose,
}: {
  readonly row: TriggerResponse;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const settings = useNyxBotSettings();
  const zone = settings.data?.timezone ?? browserTimezone();
  const history = useAutomationRuns(row.id);
  const cancelRun = useCancelAutomationRun();
  const [cancelling, setCancelling] = useState<string | null>(null);
  const runs = history.data?.pages.flatMap((page) => page.runs) ?? [];
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="bg-background sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{row.label}{translateUi(": run history")}</DialogTitle>
          <DialogDescription>{translateUi("Up to 1,000 recent runs are retained for 90 days.")}</DialogDescription>
        </DialogHeader>
        <Dialog open={cancelling !== null} onOpenChange={(open) => { if (!open) setCancelling(null); }}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{translateUi("Cancel this automation run?")}</DialogTitle>
              <DialogDescription>{translateUi("Stop this run and its delegated work. Completed actions remain completed.")}</DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setCancelling(null)}>{translateUi("Keep run")}</Button>
              <Button variant="destructive" disabled={cancelRun.isPending} onClick={() => {
                if (cancelling) cancelRun.mutate({ triggerId: row.id, runId: cancelling });
                setCancelling(null);
              }}>{translateUi("Confirm cancellation")}</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
        {cancelRun.error && <ErrorBanner message={cancelRun.error.message} />}
        {history.error ? (
          <ErrorBanner message={history.error.message} />
        ) : history.isPending ? (
          <Skeleton className="h-24 w-full" aria-label={translateUi("Loading runs")} />
        ) : !runs.length ? (
          <GroupedList>
            <GroupedRow title={translateUi("No runs yet.")} />
          </GroupedList>
        ) : (
          <div className="space-y-6">
            {runs.map((run) => (
              <GroupedList
                key={run.id}
                title={run.outcome.replaceAll("_", " ")}
                footer={
                  <>
                    {run.started_at && (
                      <p>{translateUi("Started")}{" "}
                        {formatAutomationTime(run.started_at, zone, true)}
                      </p>
                    )}
                    {run.capacity_wait_ms != null &&
                      run.capacity_wait_ms > 0 && (
                        <p>{translateUi("waited ")}{formatCapacityWait(run.capacity_wait_ms)}{translateUi(" for capacity")}</p>
                      )}
                    {run.notification_delivery && (
                      <>
                        {run.notification_delivery.message && <p>{run.notification_delivery.message}</p>}
                        {run.notification_delivery.retry_at && <p>{translateUi("Next delivery attempt: ")}{formatAutomationTime(run.notification_delivery.retry_at, zone, true)}</p>}
                        <p>{run.notification_delivery.attempts}{translateUi(" push ")}{run.notification_delivery.attempts === 1 ? translateUi("attempt") : translateUi("attempts")}{run.notification_delivery.http_status != null ? translateUi(" · Provider HTTP {{value1}}", { value1: run.notification_delivery.http_status }) : ""}</p>
                      </>
                    )}
                    {run.missed ? (
                      <p>{translateUi("Missed ")}{run.missed.count}{translateUi(" occurrences between")}{" "}
                        {formatAutomationTime(run.missed.from, zone)}{translateUi(" and")}{" "}
                        {formatAutomationTime(run.missed.through, zone)}.
                      </p>
                    ) : run.failure_reason ? (
                      <p>
                        {run.failure_reason.stage === "model_proxy"
                          ? translateUi("Model request failed")
                          : translateUi("Assistant request failed")}
                        {": "}
                        {run.failure_reason.code}
                        {run.failure_reason.http_status != null &&
                          ` (HTTP ${run.failure_reason.http_status})`}
                      </p>
                    ) : (
                      run.reason && run.reason !== run.notification_delivery?.reason && <p>{run.reason === "coordinator_timeout"
                        ? translateUi("Coordinator deadline expired (coordinator_timeout).")
                        : run.outcome === "waiting" && run.wait_kind === "coordinator"
                          ? translateUi("Waiting for delegated work to finish")
                          : run.reason.replaceAll("_", " ")}</p>
                    )}
                    {run.duration_ms !== null && (
                      <p>{(run.duration_ms / 1000).toFixed(1)}{translateUi(" seconds")}</p>
                    )}
                  </>
                }
              >
                <GroupedRow
                  title={run.scheduled_at ? translateUi("Scheduled") : translateUi("On demand")}
                  accessory={run.scheduled_at ? formatAutomationTime(run.scheduled_at, zone, true) : "Started by an event or request"}
                />
                {run.execution_outcome && <GroupedRow title={translateUi("Task execution")} accessory={run.execution_outcome} />}
                {run.notification_delivery && <GroupedRow title={translateUi("Mobile push delivery")} accessory={run.notification_delivery.state === "accepted" ? "Accepted by push provider" : run.notification_delivery.state} />}
                {run.outcome === "waiting" && run.coordinator_deadline && (
                  <GroupedRow
                    title={translateUi("Completion deadline")}
                    accessory={formatAutomationTime(run.coordinator_deadline, zone, true)}
                  />
                )}
                {run.can_cancel && (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={cancelRun.isPending}
                    onClick={() => setCancelling(run.id)}
                  >{translateUi("Cancel run")}</Button>
                )}
                {run.thread_id && (
                  <GroupedRow asChild interactive>
                    <Link
                      className="text-primary-text underline"
                      to="/assistant"
                      search={{ c: run.thread_id }}
                    >{translateUi("Open thread")}</Link>
                  </GroupedRow>
                )}
              </GroupedList>
            ))}
          </div>
        )}
        {history.hasNextPage && (
          <GroupedList>
            <GroupedRow asChild interactive>
              <button
                type="button"
                disabled={history.isFetchingNextPage}
                onClick={() => void history.fetchNextPage()}
              >{translateUi("Load older runs")}</button>
            </GroupedRow>
          </GroupedList>
        )}
      </DialogContent>
    </Dialog>
  );
}

function automationPauseReason(reason: string): string {
  if (["insufficient_credits", "wallet_suspended", "plan_entitlement_required", "credential_invalid", "model_unavailable", "model_not_configured"].includes(reason)) return storedError(reason);
  return reason.replaceAll("_", " ");
}
