import { useEffect, useRef, useState } from "react";
import { Alert, Switch, View } from "react-native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as Clipboard from "expo-clipboard";
import type { RootStackParamList } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { automationsApi } from "../../lib/api/automationsApi";
import { getCurrentUserProfileRequest } from "../../lib/api/http";
import type { AssistantAgent } from "../agents/agentContracts";
import type {
  Automation,
  AutomationChat,
  AutomationSettings,
  AutomationSetup,
} from "./automationContracts";
import {
  automationDefaults,
  automationFormSchema,
  automationRequest,
  automationUpdateRequest,
  formSchedule,
  type AutomationForm,
} from "./automationForm";
import { deviceTimezone, formatAutomationTime } from "./automationTime";
import { ownedAutomationAgents } from "./automationPresentation";
import {
  AUTOMATIONS,
  AUTOMATION_SETTINGS,
  AutomationLoading,
  AutomationSheet,
  ChoiceField,
  automationError,
  automationStyles as s,
  useAutomationRuntime,
  useAutomationWrite,
} from "./AutomationUI";
import { useTheme } from "../../theme/ThemeContext";

type Props = NativeStackScreenProps<RootStackParamList, "AutomationEditor">;
type SecretValue = { label: string; value: string };
const steps = ["Task", "When", "Delivery", "Review"];

export function AutomationEditorScreen({ navigation, route }: Props) {
  const runtime = useAutomationRuntime();
  const id = route.params?.id;
  const setupId = route.params?.setup;
  // A consumed setup watch must not unmount the completion UI and its secrets.
  const loaded = useRef<
    | {
        settings: AutomationSettings;
        row?: Automation;
        setup?: AutomationSetup;
        agents: AssistantAgent[];
        userId: string;
      }
    | undefined
  >(undefined);
  const settings = useQuery({
    queryKey: AUTOMATION_SETTINGS,
    queryFn: ({ signal }) => automationsApi.settings(signal),
    enabled: runtime.active,
    retry: false,
  });
  const detail = useQuery({
    queryKey: [...AUTOMATIONS, "detail", id],
    queryFn: ({ signal }) => automationsApi.detail(id!, signal),
    enabled: runtime.active && !!id,
    retry: false,
  });
  const setup = useQuery({
    queryKey: [...AUTOMATIONS, "setup", setupId],
    queryFn: ({ signal }) => automationsApi.setup(setupId!, signal),
    enabled: runtime.active && !!setupId && !id && !loaded.current,
    retry: false,
    staleTime: 0,
  });
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: runtime.active,
    retry: false,
  });
  const profile = useQuery({
    queryKey: ["account", "profile"],
    queryFn: ({ signal }) => getCurrentUserProfileRequest(signal),
    enabled: runtime.active,
    retry: false,
  });
  const error =
    settings.error ??
    (id ? detail.error : setupId ? setup.error : undefined) ??
    agents.error ??
    profile.error;
  if (
    !loaded.current &&
    settings.data &&
    agents.data &&
    profile.data &&
    (!id || detail.data) &&
    (!setupId || id || setup.data)
  ) {
    loaded.current = {
      settings: settings.data,
      row: id ? detail.data : undefined,
      setup: setupId && !id ? setup.data : undefined,
      agents: agents.data.agents,
      userId: profile.data.id,
    };
  }
  if (error && !loaded.current)
    return (
      <WorkspacePage title="Automation setup" showHeading={false}>
        <Notice tone="error" message={automationError(error)} />
        {setupId && (
          <Label muted>
            This setup link may have expired, been used, or no longer belong to
            this account. Your original chat remains available through Back.
          </Label>
        )}
        <ActionButton
          label="Retry setup"
          variant="secondary"
          onPress={() => {
            void settings.refetch();
            void agents.refetch();
            void profile.refetch();
            if (id) void detail.refetch();
            else if (setupId) void setup.refetch();
          }}
        />
      </WorkspacePage>
    );
  if (!loaded.current)
    return (
      <WorkspacePage title="Automation setup" showHeading={false}>
        <AutomationLoading />
      </WorkspacePage>
    );
  return (
    <AutomationEditor
      key={route.key}
      navigation={navigation}
      row={loaded.current.row}
      setup={loaded.current.setup}
      setupId={setupId}
      agentId={route.params?.agentId}
      settings={loaded.current.settings}
      agents={ownedAutomationAgents(
        agents.data?.agents ?? loaded.current.agents,
        loaded.current.userId,
      )}
    />
  );
}

function AutomationEditor({
  navigation,
  row: suppliedRow,
  setup,
  setupId,
  agentId,
  settings,
  agents,
}: {
  navigation: Props["navigation"];
  row?: Automation;
  setup?: AutomationSetup;
  setupId?: string;
  agentId?: string;
  settings: AutomationSettings;
  agents: AssistantAgent[];
}) {
  const { colors } = useTheme();
  const client = useQueryClient();
  const write = useAutomationWrite();
  const [row] = useState(suppliedRow);
  const [values, setValues] = useState<AutomationForm>(() => {
    const v = automationDefaults(
      settings.timezone ?? deviceTimezone(),
      suppliedRow,
    );
    if (!suppliedRow) {
      v.agent_id = agentId ?? agents.find((a) => a.kind === "nyxbot")?.id ?? "";
      if (setup)
        Object.assign(v, {
          source: "webhook",
          label: setup.label,
          instruction: setup.instruction,
          agent_id: setup.agent_id,
          confirmation_policy: setup.confirmation_policy,
          thread_policy: setup.thread_policy ?? "default",
        });
    }
    return v;
  });
  const initial = useRef(JSON.stringify(values));
  const [step, setStep] = useState(0);
  const [advanced, setAdvanced] = useState(false);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [reviewAcknowledged, setReviewAcknowledged] = useState(false);
  const [secrets, setSecrets] = useState<SecretValue[]>([]);
  const [savedId, setSavedId] = useState<string>();
  const [exitTo, setExitTo] = useState<"back" | "detail">();
  const [copied, setCopied] = useState<string>();
  const [previewSpec, setPreviewSpec] =
    useState<ReturnType<typeof formSchedule>>();
  const [scheduleError, setScheduleError] = useState<string>();
  const target = agents.find((a) => a.id === values.agent_id);
  const changed = initial.current !== JSON.stringify(values);
  const preview = useQuery({
    queryKey: [...AUTOMATIONS, "preview", previewSpec],
    queryFn: ({ signal }) => automationsApi.preview(previewSpec!, signal),
    enabled: write.active && values.source === "schedule" && !!previewSpec,
    staleTime: 30_000,
    retry: false,
  });
  const chats = useQuery({
    queryKey: [...AUTOMATIONS, "chats"],
    queryFn: ({ signal }) => automationsApi.chats(signal),
    enabled:
      write.active &&
      values.delivery_type === "assistant" &&
      values.deliver_type === "chat",
    retry: false,
  });
  const devices = useQuery({
    queryKey: [...AUTOMATIONS, "devices"],
    queryFn: ({ signal }) => automationsApi.deviceConversations(signal),
    enabled: write.active && values.delivery_type === "agent",
    retry: false,
  });
  const serialized = JSON.stringify(values);
  let previewMatches = values.source !== "schedule";
  try {
    if (values.source === "schedule")
      previewMatches =
        JSON.stringify(previewSpec) ===
        JSON.stringify(formSchedule(values, row));
  } catch {
    /* Invalid calendar input is shown inline after the debounce. */
  }
  useEffect(() => {
    const timer = setTimeout(() => {
      try {
        setPreviewSpec(
          values.source === "schedule" ? formSchedule(values, row) : undefined,
        );
        setScheduleError(undefined);
      } catch (cause) {
        setPreviewSpec(undefined);
        setScheduleError(automationError(cause));
      }
    }, 350);
    return () => clearTimeout(timer);
  }, [serialized, row]);
  function set<K extends keyof AutomationForm>(
    field: K,
    value: AutomationForm[K],
  ) {
    setValues((v) => ({ ...v, [field]: value }));
    setFieldErrors((errors) => ({ ...errors, [field]: "" }));
    setReviewAcknowledged(false);
  }
  function leave() {
    if (step > 0 && !savedId) {
      setStep((v) => v - 1);
      return;
    }
    if (changed && !savedId)
      Alert.alert(
        "Discard automation changes?",
        "Your changes haven’t been saved.",
        [
          { text: "Keep editing", style: "cancel" },
          {
            text: "Discard",
            style: "destructive",
            onPress: () => setExitTo("back"),
          },
        ],
      );
    else navigation.goBack();
  }
  useWorkspaceBack({
    active: !exitTo && (step > 0 || changed || write.busy || !!savedId),
    defaultRole: "back",
    disabled: !exitTo && (write.busy || !!savedId),
    label: step > 0 ? "Previous step" : "Back",
    onBack: leave,
  });
  useEffect(() => {
    if (write.busy || !write.active) return;
    if (exitTo === "back") navigation.goBack();
    else if (exitTo === "detail" && savedId) {
      if (row) navigation.popTo("AutomationDetail", { id: savedId });
      else navigation.replace("AutomationDetail", { id: savedId });
    }
  }, [exitTo, navigation, savedId, row, write.busy, write.active]);
  function validate(all = false) {
    const result = automationFormSchema.safeParse(values);
    const stepFields =
      step === 0
        ? ["label", "agent_id", "instruction"]
        : step === 1
          ? [
              "kind",
              "expression",
              "timezone",
              "amount",
              "anchor",
              "at",
              "start",
              "end",
              "max_runs",
              "grace_seconds",
              "signature_header",
            ]
          : ["webhook_url", "conversation_id", "chat_id"];
    const errors = result.success
      ? []
      : result.error.issues.filter(
          (issue) => all || stepFields.includes(String(issue.path[0])),
        );
    const mapped: Record<string, string> = {};
    for (const issue of errors) mapped[String(issue.path[0])] = issue.message;
    if (values.delivery_type === "assistant" && !target && (step === 0 || all))
      mapped.agent_id = "Choose a live agent owned by your personal account.";
    if (
      (step === 2 || all) &&
      values.delivery_type === "assistant" &&
      values.deliver_type === "chat" &&
      !eligibleChats(chats.data ?? []).some(
        (chat) => chat.id === values.chat_id,
      )
    )
      mapped.chat_id = "Choose an available chat that this agent may post to.";
    if (
      (step === 2 || all) &&
      values.delivery_type === "agent" &&
      !(devices.data ?? []).some((chat) => chat.id === values.conversation_id)
    )
      mapped.conversation_id = "Choose an available device conversation.";
    setFieldErrors(mapped);
    return Object.keys(mapped).length === 0;
  }
  function field(
    name: keyof AutomationForm,
    label: string,
    options: {
      numeric?: boolean;
      multiline?: boolean;
      placeholder?: string;
    } = {},
  ) {
    return (
      <View style={s.field}>
        <Field
          label={label}
          value={String(values[name])}
          onChangeText={(v) =>
            set(
              name,
              (options.numeric && name === "amount"
                ? Number(v)
                : v) as AutomationForm[typeof name],
            )
          }
          editable={!write.busy}
          keyboardType={options.numeric ? "number-pad" : "default"}
          multiline={options.multiline}
          placeholder={options.placeholder}
        />
        {!!fieldErrors[name] && (
          <Label style={{ color: colors.danger }}>{fieldErrors[name]}</Label>
        )}
      </View>
    );
  }
  function choice(
    name: keyof AutomationForm,
    label: string,
    options: readonly (readonly [string, string, string?])[],
    disabled = false,
  ) {
    return (
      <View style={s.field}>
        <ChoiceField
          label={label}
          value={String(values[name])}
          disabled={write.busy || disabled}
          onChange={(value) => set(name, value as AutomationForm[typeof name])}
          options={options.map(([value, text, description]) => ({
            value,
            label: text,
            description,
          }))}
        />
        {!!fieldErrors[name] && (
          <Label style={{ color: colors.danger }}>{fieldErrors[name]}</Label>
        )}
      </View>
    );
  }
  function dateTime(
    name: "at" | "anchor" | "start" | "end",
    label: string,
    optional = false,
  ) {
    const [date = "", time = ""] = values[name].split("T");
    return (
      <View style={s.field}>
        <Label>
          {label}
          {optional ? " (optional)" : ""}
        </Label>
        <View style={s.row}>
          <View style={s.flex}>
            <Field
              label={`${label} date`}
              placeholder="YYYY-MM-DD"
              value={date}
              onChangeText={(v) =>
                set(name, v ? `${v}T${time || "09:00"}` : "")
              }
              editable={!write.busy}
            />
          </View>
          <View style={s.flex}>
            <Field
              label={`${label} time`}
              placeholder="HH:mm"
              value={time}
              onChangeText={(v) => set(name, `${date}T${v}`)}
              editable={!write.busy}
            />
          </View>
        </View>
        <Label muted style={s.small}>
          {values.timezone} · 24-hour time
        </Label>
        {optional && values[name] && (
          <ActionButton
            label={`Clear ${label.toLowerCase()}`}
            variant="secondary"
            disabled={write.busy}
            onPress={() => set(name, "")}
          />
        )}
        {!!fieldErrors[name] && (
          <Label style={{ color: colors.danger }}>{fieldErrors[name]}</Label>
        )}
      </View>
    );
  }
  async function save() {
    if (!validate(true) || !reviewAcknowledged) return;
    if (
      values.source === "schedule" &&
      (!previewMatches || !preview.data || preview.error || preview.isFetching)
    )
      return;
    await write.run(async (current) => {
      if (!settings.timezone) {
        await automationsApi.updatePreferences({
          timezone: values.timezone || deviceTimezone(),
        });
        if (!current() || !write.interactive()) return;
      }
      const result = row
        ? await automationsApi.update(
            row.id,
            automationUpdateRequest(values, row),
          )
        : await automationsApi.create({
            ...automationRequest(values),
            ...(setupId ? { watch_id: setupId } : {}),
          });
      if (!current()) return;
      const oneTime: SecretValue[] = [];
      if ("secret" in result && result.secret) {
        if (result.trigger.inbound_url)
          oneTime.push({
            label: "Inbound URL",
            value: result.trigger.inbound_url,
          });
        oneTime.push({ label: "Inbound secret", value: result.secret });
      }
      if (result.delivery_signing_secret)
        oneTime.push({
          label: "Delivery signing secret",
          value: result.delivery_signing_secret,
        });
      // Only the non-secret trigger enters the query cache.
      client.setQueryData(
        [...AUTOMATIONS, "detail", result.trigger.id],
        result.trigger,
      );
      void client.invalidateQueries({ queryKey: AUTOMATIONS });
      if (setupId)
        client.removeQueries({ queryKey: [...AUTOMATIONS, "setup", setupId] });
      setSavedId(result.trigger.id);
      initial.current = JSON.stringify(values);
      setSecrets(oneTime);
      if (!oneTime.length) setExitTo("detail");
    });
  }
  function finish() {
    if (!savedId) return;
    setSecrets([]);
    setExitTo("detail");
  }
  function dismissSecrets() {
    Alert.alert(
      "Saved these secrets?",
      "They won’t be shown again after closing.",
      [
        { text: "Keep open", style: "cancel" },
        { text: "I’ve saved them", onPress: finish },
      ],
    );
  }
  const defaultThread =
    values.source === "schedule" && target?.kind === "nyxbot"
      ? "Home"
      : "Dedicated";
  const resolvedThread =
    values.thread_policy === "default" ? defaultThread : values.thread_policy;
  return (
    <WorkspacePage
      key={step}
      title={row ? "Edit automation" : "Create automation"}
      showHeading={false}
    >
      <View style={s.spread}>
        <Label style={s.section}>{steps[step]}</Label>
        <Label muted>Step {step + 1} of 4</Label>
      </View>
      <View style={{ flexDirection: "row", gap: 5 }}>
        {steps.map((name, i) => (
          <View
            key={name}
            style={{
              height: 3,
              flex: 1,
              borderRadius: 3,
              backgroundColor: i <= step ? colors.primary : colors.border,
            }}
          />
        ))}
      </View>
      {!write.online && (
        <Notice message="You’re offline. Your edits are kept here; reconnect to preview and save." />
      )}
      {write.error && <Notice tone="error" message={write.error} />}
      {row?.status === "disabled" && (
        <Notice message="This automation is paused. Saving changes keeps it paused; upcoming times are a schedule preview." />
      )}
      {step === 0 && (
        <>
          {field("label", "Automation name", {
            placeholder: "Morning briefing",
          })}
          {choice(
            "source",
            "Start this task",
            [
              ["schedule", "On a schedule"],
              ["webhook", "When a webhook arrives"],
            ],
            !!row || !!setup,
          )}
          {choice("delivery_type", "Task type", [
            ["assistant", "Agent task", "Ask NyxBot or a specialist to act."],
            [
              "notification",
              "Reminder notification",
              "Send a reminder without running an agent.",
            ],
            [
              "webhook",
              "HTTPS webhook",
              "Deliver an event to an external endpoint.",
            ],
            [
              "agent",
              "Device agent",
              "Deliver to an active device conversation.",
            ],
          ])}
          {values.delivery_type === "assistant" && (
            <>
              {choice(
                "agent_id",
                "Agent",
                agents.map((a) => [
                  a.id,
                  a.display_name || a.name,
                  a.kind === "nyxbot"
                    ? "NyxBot · your personal account"
                    : "Specialist · existing permissions",
                ]),
              )}
              {!agents.length && (
                <Notice message="No personal agents are available. Return to Agents to check access." />
              )}
              {field("instruction", "What should the agent do?", {
                multiline: true,
                placeholder:
                  "Summarise my inbox and highlight anything urgent.",
              })}
              <Label muted style={s.small}>
                Up to 8 KB. This task uses the agent’s existing permissions.
              </Label>
            </>
          )}
        </>
      )}
      {step === 1 &&
        (values.source === "schedule" ? (
          <>
            {choice("kind", "Repeat", [
              ["cron", "Calendar schedule"],
              ["every", "Every interval"],
              ["at", "Once"],
            ])}
            {field("timezone", "Timezone", { placeholder: "Asia/Shanghai" })}
            <ActionButton
              label="Use device timezone"
              variant="secondary"
              disabled={write.busy}
              onPress={() => set("timezone", deviceTimezone())}
            />
            {values.kind === "cron" && (
              <>
                <ChoiceField
                  label="Calendar preset"
                  value={values.expression}
                  options={[
                    { value: "0 8 * * 1-5", label: "Weekdays at 08:00" },
                    { value: "0 9 * * *", label: "Every day at 09:00" },
                    { value: "0 17 * * 5", label: "Fridays at 17:00" },
                    ...(["0 8 * * 1-5", "0 9 * * *", "0 17 * * 5"].includes(
                      values.expression,
                    )
                      ? []
                      : [
                          {
                            value: values.expression,
                            label: "Custom schedule",
                          },
                        ]),
                  ]}
                  onChange={(v) => set("expression", v)}
                  disabled={write.busy}
                />
                {field("expression", "Cron expression", {
                  placeholder: "0 8 * * 1-5",
                })}
                <Label muted style={s.small}>
                  Five fields: minute, hour, day of month, month, day of week.
                </Label>
              </>
            )}
            {values.kind === "every" && (
              <>
                {field("amount", "Every", { numeric: true })}
                {choice("unit", "Interval unit", [
                  ["minutes", "Minutes"],
                  ["hours", "Hours"],
                  ["days", "Days"],
                ])}
                {dateTime("anchor", "First anchor")}
              </>
            )}
            {values.kind === "at" && dateTime("at", "Run once")}
            <Label muted style={s.small}>
              Minimum interval: {settings.schedule_minimum_minutes} minutes.
              Preview is calculated by the server.
            </Label>
            <ActionButton
              label={
                advanced ? "Hide schedule limits" : "Schedule limits (optional)"
              }
              variant="secondary"
              onPress={() => setAdvanced(!advanced)}
            />
            {advanced && (
              <>
                {dateTime("start", "Start", true)}
                {dateTime("end", "End", true)}
                {field("max_runs", "Maximum occurrences", {
                  numeric: true,
                  placeholder: "No limit",
                })}
                {field("grace_seconds", "Grace period (seconds)", {
                  numeric: true,
                  placeholder: "Server default",
                })}
              </>
            )}
            <SchedulePreview
              preview={previewMatches ? preview.data : undefined}
              loading={!previewMatches || preview.isFetching}
              error={
                scheduleError ??
                (preview.error ? automationError(preview.error) : undefined)
              }
              zone={values.timezone}
            />
            {preview.error && (
              <ActionButton
                label="Retry schedule preview"
                variant="secondary"
                onPress={() => void preview.refetch()}
              />
            )}
          </>
        ) : (
          <>
            <Card>
              <Label style={s.title}>Incoming webhook</Label>
              <Label muted>
                The inbound URL and secret are generated after saving. Save them
                then; they are shown only once.
              </Label>
            </Card>
            {choice(
              "verification_mode",
              "Verify incoming events",
              [
                ["bearer", "Bearer token"],
                ["query", "Query token"],
                ["hmac", "HMAC-SHA256 signature"],
              ],
              !!row,
            )}
            {values.verification_mode === "hmac" &&
              (row ? (
                <Card>
                  <Label>Signature header: {values.signature_header}</Label>
                  <Label muted>
                    Verification cannot be changed after creation.
                  </Label>
                </Card>
              ) : (
                field("signature_header", "Signature header", {
                  placeholder: "X-Signature",
                })
              ))}
          </>
        ))}
      {step === 2 && (
        <>
          {values.delivery_type === "assistant" && (
            <>
              {choice("thread_policy", "Run in", [
                [
                  "default",
                  `Default · ${defaultThread}`,
                  "Home for scheduled NyxBot; Dedicated for specialists and webhooks.",
                ],
                ["home", "Home thread"],
                ["dedicated", "Dedicated automation thread"],
                ["new", "New thread for each run"],
              ])}
              {values.source === "webhook" &&
                values.thread_policy === "home" && (
                  <Notice
                    tone="error"
                    message="Untrusted webhook content remains in the home thread and may affect later owner turns, including private channel chats. The webhook confirmation policy does not protect those later turns."
                  />
                )}
              {choice("deliver_type", "Send the result to", [
                ["thread", "Thread only"],
                ["notification", "Notification"],
                ["chat", "Channel chat"],
              ])}
              {values.deliver_type === "chat" && (
                <>
                  {chats.isPending && <AutomationLoading />}
                  {chats.error && (
                    <>
                      <Notice
                        tone="error"
                        message={automationError(chats.error)}
                      />
                      <ActionButton
                        label="Retry channel chats"
                        variant="secondary"
                        onPress={() => void chats.refetch()}
                      />
                    </>
                  )}
                  {choice(
                    "chat_id",
                    "Channel chat",
                    eligibleChats(chats.data ?? []).map((chat) => [
                      chat.id,
                      chat.title || chat.bot_label,
                      `${chat.platform} · ${chat.bot_label}`,
                    ]),
                  )}
                  {!chats.isPending &&
                    !eligibleChats(chats.data ?? []).length && (
                      <Notice message="No chats allow this agent to post. Configure its channel access on Web, or choose Thread or Notification." />
                    )}
                </>
              )}
              {values.source === "webhook" && (
                <>
                  {choice("confirmation_policy", "Require confirmation for", [
                    ["changes", "Every changing action (recommended)"],
                    ["destructive", "Only destructive actions"],
                  ])}
                  {values.confirmation_policy === "destructive" && (
                    <Notice
                      tone="error"
                      message="Ordinary changing actions may run without confirmation. Only destructive actions will wait for your approval."
                    />
                  )}
                </>
              )}
            </>
          )}
          {values.delivery_type === "webhook" && (
            <>
              {field("webhook_url", "Delivery URL", {
                placeholder: "https://example.com/events",
              })}
              <Notice message="The outgoing delivery signing secret will be shown once after saving. This URL is separate from incoming webhook verification." />
            </>
          )}
          {values.delivery_type === "agent" && (
            <>
              {devices.isPending && <AutomationLoading />}
              {devices.error && (
                <>
                  <Notice
                    tone="error"
                    message={automationError(devices.error)}
                  />
                  <ActionButton
                    label="Retry device conversations"
                    variant="secondary"
                    onPress={() => void devices.refetch()}
                  />
                </>
              )}
              {choice(
                "conversation_id",
                "Device conversation",
                (devices.data ?? []).map((chat) => [
                  chat.id,
                  chat.platform_conversation_id,
                ]),
              )}
              {!devices.isPending && !devices.data?.length && (
                <Notice message="No active device conversations are available. Connect a device on Web, or choose another task type." />
              )}
            </>
          )}
          {values.delivery_type === "notification" && (
            <Card>
              <Label style={s.title}>Reminder notification</Label>
              <Label muted>
                The automation name becomes the reminder. No agent runs.
                Notification delivery follows your account and device
                preferences.
              </Label>
            </Card>
          )}
          {choice("overlap", "If a previous run is still active", [
            ["skip", "Skip this occurrence"],
            [
              "queue",
              "Queue one run",
              "Keep at most one pending run, not an unlimited backlog.",
            ],
          ])}
        </>
      )}
      {step === 3 && (
        <>
          <Card>
            <Label style={s.title}>{values.label}</Label>
            <Label>Source: {values.source}</Label>
            <Label>
              Task:{" "}
              {values.delivery_type === "assistant"
                ? target?.display_name || target?.name || values.agent_id
                : values.delivery_type === "notification"
                  ? "Reminder notification"
                  : values.delivery_type === "agent"
                    ? "Device agent"
                    : "HTTPS webhook"}
            </Label>
            <Label muted>Owner: Your personal account</Label>
            {values.delivery_type === "assistant" && (
              <>
                <Label selectable>{values.instruction}</Label>
                <Label>Thread: {resolvedThread}</Label>
                <Label>
                  Result:{" "}
                  {values.deliver_type === "chat"
                    ? chats.data?.find((c) => c.id === values.chat_id)?.title ||
                      "Channel chat"
                    : values.deliver_type}
                </Label>
              </>
            )}
            {values.delivery_type === "webhook" && (
              <Label selectable>{values.webhook_url}</Label>
            )}
            <Label muted>
              Overlap:{" "}
              {values.overlap === "queue"
                ? "Queue one run"
                : "Skip overlapping runs"}
            </Label>
          </Card>
          {values.source === "schedule" && (
            <SchedulePreview
              preview={previewMatches ? preview.data : undefined}
              loading={!previewMatches || preview.isFetching}
              error={
                scheduleError ??
                (preview.error ? automationError(preview.error) : undefined)
              }
              zone={values.timezone}
            />
          )}
          {values.source === "webhook" && (
            <Notice message="Save the inbound URL and secret after creating. They are displayed once. Incoming events must pass the selected verification." />
          )}
          {values.delivery_type === "assistant" && (
            <Notice message="Runs use your identity, normal usage and billing, and the agent’s existing permissions. Creating this automation does not grant additional access. Scheduled runs keep normal owner confirmations." />
          )}
          {values.source === "webhook" &&
            values.delivery_type === "assistant" && (
              <Notice
                tone={
                  values.confirmation_policy === "destructive" ||
                  values.thread_policy === "home"
                    ? "error"
                    : "info"
                }
                message={`Webhook confirmation: ${values.confirmation_policy === "changes" ? "every changing action" : "only destructive actions; ordinary changes can run without a card"}.${values.thread_policy === "home" ? " Untrusted content persists into later owner turns. Those turns are not protected by this webhook policy." : ""}`}
              />
            )}
          {Object.values(fieldErrors).some(Boolean) && (
            <Notice
              tone="error"
              message={Object.values(fieldErrors).filter(Boolean).join("\n")}
            />
          )}
          <View style={s.row}>
            <Switch
              accessibilityLabel="I have reviewed this automation"
              value={reviewAcknowledged}
              onValueChange={setReviewAcknowledged}
              disabled={write.busy}
              trackColor={{ true: colors.primary }}
            />
            <Label style={s.flex}>
              I have reviewed the task, timing and permissions.
            </Label>
          </View>
        </>
      )}
      {!savedId &&
        (step < 3 ? (
          <ActionButton
            label="Continue"
            disabled={write.busy}
            onPress={() => {
              if (validate()) setStep(step + 1);
            }}
          />
        ) : (
          <ActionButton
            label={row ? "Save changes" : "Create automation"}
            loading={write.busy}
            disabled={
              !write.online ||
              !reviewAcknowledged ||
              (values.source === "schedule" &&
                (!previewMatches ||
                  !preview.data ||
                  !!preview.error ||
                  !!scheduleError ||
                  preview.isFetching))
            }
            onPress={() => void save()}
          />
        ))}
      {savedId && !secrets.length && (
        <Card>
          <Label style={s.title}>Automation saved</Label>
          <Label muted>
            {row
              ? "Your changes are saved."
              : "Your automation is active. Open its details to view the next run or pause it."}
          </Label>
          <ActionButton label="View automation" onPress={finish} />
        </Card>
      )}
      <AutomationSheet
        title="Save automation secrets"
        visible={!!secrets.length && write.active}
        onClose={dismissSecrets}
      >
        <Notice message="These values are shown only once. Save them securely before closing." />
        {secrets.map((secret) => (
          <Card key={secret.label}>
            <Label>{secret.label}</Label>
            <Label selectable mono>
              {secret.value}
            </Label>
            <ActionButton
              label={
                copied === secret.label
                  ? `Copied ${secret.label.toLowerCase()}`
                  : `Copy ${secret.label.toLowerCase()}`
              }
              variant="secondary"
              onPress={() => {
                void Clipboard.setStringAsync(secret.value)
                  .then(() => setCopied(secret.label))
                  .catch(() => setCopied(undefined));
              }}
            />
          </Card>
        ))}
        <ActionButton
          label="I’ve saved these secrets"
          onPress={dismissSecrets}
        />
      </AutomationSheet>
    </WorkspacePage>
  );
}

function eligibleChats(chats: AutomationChat[]) {
  // The server checks the agent's effective channel routing again on save.
  return chats.filter((chat) => chat.allow_posts);
}

function SchedulePreview({
  preview,
  loading,
  error,
  zone,
}: {
  preview?: { next_runs: string[]; description?: string };
  loading: boolean;
  error?: string;
  zone: string;
}) {
  return (
    <Card>
      <Label style={s.title}>Upcoming runs</Label>
      <Label muted style={s.small}>
        {zone}
      </Label>
      {loading && <AutomationLoading />}
      {error && <Notice tone="error" message={error} />}
      {!loading && !error && preview && (
        <>
          {preview.description && <Label>{preview.description}</Label>}
          {preview.next_runs.map((at) => (
            <Label key={at} mono style={s.small}>
              {formatAutomationTime(at, zone)}
            </Label>
          ))}
          {!preview.next_runs.length && (
            <Label muted>No future runs for these settings.</Label>
          )}
        </>
      )}
    </Card>
  );
}
