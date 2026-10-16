import { useEffect, useState } from "react";
import { Alert, Pressable, View } from "react-native";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import * as WebBrowser from "expo-web-browser";
import {
  ChevronRight,
  Clock3,
  MoreHorizontal,
  SlidersHorizontal,
} from "lucide-react-native";
import type {
  RootStackParamList,
  WorkspaceScreenProps,
} from "../../app/navigationTypes";
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
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { useTheme } from "../../theme/ThemeContext";
import { automationScheduleSummary } from "./automationForm";
import type { Automation, AutomationRunsCursor } from "./automationContracts";
import { deviceTimezone, formatAutomationTime } from "./automationTime";
import { validateBrowserUrl } from "../services/serviceContracts";
import {
  automationPauseReason,
  automationRunNotice,
  automationTarget,
  uniqueAutomationRuns,
} from "./automationPresentation";
import {
  AUTOMATIONS,
  AUTOMATION_SETTINGS,
  AutomationLoading,
  AutomationSheet,
  AutomationStatus,
  ChoiceField,
  automationError,
  automationStyles as s,
  useAutomationRuntime,
  useAutomationWrite,
} from "./AutomationUI";

export { AutomationEditorScreen } from "./AutomationEditorScreen";
export { AutomationPreferencesScreen } from "./AutomationPreferencesScreen";

export function AutomationsScreen({
  navigation,
  route,
}: WorkspaceScreenProps<"Automations">) {
  const runtime = useAutomationRuntime();
  const { colors } = useTheme();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState("all");
  const [agentId, setAgentId] = useState(route.params?.agentId ?? "");
  const list = useQuery({
    queryKey: [...AUTOMATIONS, "list"],
    queryFn: ({ signal }) => automationsApi.list(signal),
    enabled: runtime.active,
    retry: false,
    refetchInterval: runtime.active ? 15_000 : false,
  });
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: runtime.active,
    retry: false,
  });
  const settings = useQuery({
    queryKey: AUTOMATION_SETTINGS,
    queryFn: ({ signal }) => automationsApi.settings(signal),
    enabled: runtime.active,
    retry: false,
  });
  useEffect(() => {
    setAgentId(route.params?.agentId ?? "");
  }, [route.params?.agentId]);
  useEffect(() => {
    if (!route.params?.setup || !runtime.active) return;
    const setup = route.params.setup;
    navigation.setParams({ setup: undefined });
    navigation.push("AutomationEditor", {
      setup,
      agentId: route.params.agentId,
    });
  }, [route.params?.setup, route.params?.agentId, runtime.active, navigation]);
  const zone = settings.data?.timezone ?? deviceTimezone();
  const rows = (list.data?.triggers ?? []).filter(
    (row) =>
      (status === "all" || row.status === status) &&
      (!agentId ||
        (row.delivery.type === "assistant" &&
          row.delivery.agent_id === agentId)) &&
      `${row.label} ${automationTarget(row, agents.data?.agents)}`
        .toLowerCase()
        .includes(search.trim().toLowerCase()),
  );
  const agentOptions =
    agents.data?.agents
      .filter((a) => a.status !== "destroyed")
      .map((a) => ({ value: a.id, label: a.display_name || a.name })) ?? [];
  if (agentId && !agentOptions.some((a) => a.value === agentId))
    agentOptions.push({ value: agentId, label: "Selected agent" });
  return (
    <WorkspacePage title="Automations" showHeading={false}>
      <View style={s.spread}>
        <View style={s.flex}>
          <Label style={s.section}>Tasks that run for you</Label>
          <Label muted style={s.small}>
            On a schedule or when an event arrives.
          </Label>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Automation preferences"
          style={s.iconButton}
          onPress={() => navigation.push("AutomationPreferences")}
        >
          <SlidersHorizontal size={21} color={colors.textSecondary} />
        </Pressable>
      </View>
      <ActionButton
        label="Create automation"
        disabled={!runtime.online}
        onPress={() =>
          navigation.push("AutomationEditor", { agentId: agentId || undefined })
        }
      />
      <Field
        label="Search automations"
        placeholder="Search by name or agent"
        value={search}
        onChangeText={setSearch}
      />
      <View style={s.row}>
        <View style={s.flex}>
          <ChoiceField
            label="Status"
            value={status}
            onChange={setStatus}
            options={[
              { value: "all", label: "All statuses" },
              { value: "active", label: "Active" },
              { value: "disabled", label: "Paused" },
            ]}
          />
        </View>
        <View style={s.flex}>
          <ChoiceField
            label="Agent"
            value={agentId}
            onChange={setAgentId}
            options={[{ value: "", label: "All agents" }, ...agentOptions]}
          />
        </View>
      </View>
      {!runtime.online && (
        <Notice message="You’re offline. Reconnect to refresh or change automations." />
      )}
      {list.isPending && <AutomationLoading />}
      {list.error && (
        <>
          <Notice tone="error" message={automationError(list.error)} />
          <ActionButton
            label="Retry automations"
            variant="secondary"
            onPress={() => void list.refetch()}
          />
        </>
      )}
      {rows.map((row) => (
        <Pressable
          key={row.id}
          accessibilityRole="button"
          accessibilityLabel={`Open ${row.label}`}
          onPress={() => navigation.push("AutomationDetail", { id: row.id })}
        >
          <Card>
            <View style={s.spread}>
              <View style={s.row}>
                <Clock3 size={18} color={colors.primary} />
                <AutomationStatus value={row.status} />
              </View>
              <ChevronRight size={18} color={colors.textMuted} />
            </View>
            <Label style={s.title}>{row.label}</Label>
            <Label muted>
              {automationTarget(row, agents.data?.agents)} ·{" "}
              {(row.source ?? "webhook") === "schedule"
                ? "Scheduled"
                : "Webhook"}
            </Label>
            <Label muted style={s.small}>
              {automationScheduleSummary(row)}
            </Label>
            {row.source === "schedule" && (
              <Label style={s.small}>
                {row.status === "disabled"
                  ? "Paused · no new scheduled runs"
                  : `Next: ${formatAutomationTime(row.next_run_at, zone)} · ${zone}`}
              </Label>
            )}
            {row.last_run && (
              <Label muted style={s.small}>
                Last run: {row.last_run.outcome} ·{" "}
                {formatAutomationTime(row.last_run.scheduled_at, zone)}
              </Label>
            )}
            {row.pause_reason && (
              <Label style={s.small}>
                {automationPauseReason(row.pause_reason)}
              </Label>
            )}
          </Card>
        </Pressable>
      ))}
      {list.data && !rows.length && (
        <Card>
          <Label style={s.title}>
            {list.data.triggers.length
              ? "No matching automations"
              : "Your first automation"}
          </Label>
          <Label muted>
            {list.data.triggers.length
              ? "Try another search, agent or status."
              : "Ask NyxBot to help, or create a task and choose when it should run."}
          </Label>
          {!list.data.triggers.length && (
            <ActionButton
              label="Ask NyxBot"
              variant="secondary"
              onPress={() =>
                navigation.push("AssistantContext", { draft: true })
              }
            />
          )}
        </Card>
      )}
    </WorkspacePage>
  );
}

export function AutomationDetailScreen({
  navigation,
  route,
}: NativeStackScreenProps<RootStackParamList, "AutomationDetail">) {
  const write = useAutomationWrite();
  const { colors } = useTheme();
  const client = useQueryClient();
  const [menu, setMenu] = useState(false);
  const [notice, setNotice] = useState<string>();
  const [deleted, setDeleted] = useState(false);
  const detail = useQuery({
    queryKey: [...AUTOMATIONS, "detail", route.params.id],
    queryFn: ({ signal }) => automationsApi.detail(route.params.id, signal),
    enabled: write.active,
    retry: false,
    refetchInterval: write.active ? 5000 : false,
  });
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: write.active,
    retry: false,
  });
  const settings = useQuery({
    queryKey: AUTOMATION_SETTINGS,
    queryFn: ({ signal }) => automationsApi.settings(signal),
    enabled: write.active,
    retry: false,
  });
  const row = detail.data;
  const zone = settings.data?.timezone ?? deviceTimezone();
  const scheduleZone =
    row?.schedule?.kind === "cron" ? row.schedule.timezone : zone;
  useWorkspaceBack({
    active: write.busy,
    defaultRole: "back",
    disabled: write.busy,
    onBack: navigation.goBack,
  });
  useEffect(() => {
    if (deleted && !write.busy && write.active) navigation.goBack();
  }, [deleted, write.busy, write.active, navigation]);
  function mutate(action: "run" | "toggle" | "delete") {
    setMenu(false);
    if (!row) return;
    void write.run(async (current) => {
      setNotice(undefined);
      if (action === "delete") await automationsApi.remove(row.id);
      else if (action === "toggle")
        await automationsApi.update(row.id, {
          status: row.status === "active" ? "disabled" : "active",
        });
      else {
        const result = await automationsApi.run(row.id);
        if (current()) setNotice(automationRunNotice(result));
      }
      if (!current()) return;
      void client.invalidateQueries({ queryKey: AUTOMATIONS });
      if (action === "delete") setDeleted(true);
    });
  }
  return (
    <WorkspacePage title="Automation" showHeading={false}>
      {detail.isPending && <AutomationLoading />}
      {detail.error && (
        <>
          <Notice tone="error" message={automationError(detail.error)} />
          <ActionButton
            label="Retry automation"
            variant="secondary"
            onPress={() => void detail.refetch()}
          />
        </>
      )}
      {write.error && <Notice tone="error" message={write.error} />}
      {notice && <Notice message={notice} />}
      {row && (
        <>
          <View style={s.spread}>
            <AutomationStatus value={row.status} />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Automation actions"
              disabled={write.busy}
              onPress={() => setMenu(true)}
              style={s.iconButton}
            >
              <MoreHorizontal size={23} color={colors.textPrimary} />
            </Pressable>
          </View>
          <Label accessibilityRole="header" style={s.section}>
            {row.label}
          </Label>
          <Label muted>
            {automationTarget(row, agents.data?.agents)} ·{" "}
            {row.source ?? "webhook"}
          </Label>
          {row.pause_reason && (
            <Notice message={automationPauseReason(row.pause_reason)!} />
          )}
          {row.status === "disabled" && (
            <Notice message="Future scheduled runs are paused. Resuming calculates the next future occurrence; existing threads remain available." />
          )}
          <Card>
            <Label style={s.title}>Schedule</Label>
            <Label>{automationScheduleSummary(row)}</Label>
            <Label muted style={s.small}>
              Timezone:{" "}
              {row.schedule?.kind === "cron" ? row.schedule.timezone : zone}
            </Label>
            {row.status === "active" &&
              row.source === "schedule" &&
              (row.next_runs?.length
                ? row.next_runs
                : row.next_run_at
                  ? [row.next_run_at]
                  : []
              ).map((at) => (
                <Label key={at} mono style={s.small}>
                  {formatAutomationTime(at, scheduleZone)}
                </Label>
              ))}
            <Label muted style={s.small}>
              Overlap:{" "}
              {row.overlap === "queue"
                ? "Queue one run"
                : "Skip overlapping runs"}
            </Label>
          </Card>
          <AutomationDeliveryCard
            row={row}
            target={automationTarget(row, agents.data?.agents)}
          />
          {row.delivery.type === "assistant" && (
            <ActionButton
              label="Review agent permissions"
              variant="secondary"
              onPress={() => {
                if (row.delivery.type === "assistant")
                  navigation.push("AgentDetail", {
                    agentId: row.delivery.agent_id,
                  });
              }}
            />
          )}
          {row.last_run && (
            <Card>
              <View style={s.spread}>
                <Label style={s.title}>Latest run</Label>
                <AutomationStatus value={row.last_run.outcome} />
              </View>
              <Label muted>
                {formatAutomationTime(row.last_run.scheduled_at, zone)} · {zone}
              </Label>
              {row.last_run.reason && (
                <Label>{row.last_run.reason.replaceAll("_", " ")}</Label>
              )}
            </Card>
          )}
          <ActionButton
            label="View run history"
            variant="secondary"
            disabled={write.busy}
            onPress={() => navigation.push("AutomationRuns", { id: row.id })}
          />
          <ActionButton
            label="Edit automation"
            disabled={write.busy || !write.online}
            onPress={() => navigation.push("AutomationEditor", { id: row.id })}
          />
          {row.delivery.type === "assistant" && (
            <Label muted style={s.small}>
              Normal agent usage and billing apply. This automation uses
              existing agent permissions.
            </Label>
          )}
          <AutomationSheet
            title="Automation actions"
            visible={menu && write.active}
            onClose={() => setMenu(false)}
          >
            {row.source !== "schedule" && (
              <ActionButton
                label="Webhook secrets & delivery history on Web"
                variant="secondary"
                onPress={() => {
                  setMenu(false);
                  void write.run(async () => {
                    const url = new URL(
                      "/triggers",
                      validateBrowserUrl(NYXID_FRONTEND_URL),
                    );
                    await WebBrowser.openBrowserAsync(url.href);
                  });
                }}
              />
            )}
            <ActionButton
              label={
                row.status === "active"
                  ? "Pause automation"
                  : "Resume automation"
              }
              variant="secondary"
              disabled={!write.online || write.busy}
              onPress={() => mutate("toggle")}
            />
            <ActionButton
              label="Run now"
              variant="secondary"
              disabled={!write.online || write.busy || row.status !== "active"}
              onPress={() => mutate("run")}
            />
            <ActionButton
              label="Delete automation"
              variant="danger"
              disabled={!write.online || write.busy}
              onPress={() => {
                setMenu(false);
                Alert.alert(
                  `Delete ${row.label}?`,
                  "Future runs stop. Existing run threads remain available.",
                  [
                    { text: "Cancel", style: "cancel" },
                    {
                      text: "Delete",
                      style: "destructive",
                      onPress: () => mutate("delete"),
                    },
                  ],
                );
              }}
            />
          </AutomationSheet>
        </>
      )}
    </WorkspacePage>
  );
}

function AutomationDeliveryCard({
  row,
  target,
}: {
  row: Automation;
  target: string;
}) {
  const d = row.delivery;
  return (
    <Card>
      <Label style={s.title}>Task & delivery</Label>
      <Label>{target}</Label>
      {d.type === "assistant" ? (
        <>
          <Label selectable>{d.instruction}</Label>
          <Label muted>
            Thread: {d.thread_policy ?? "Default for this agent and source"}
          </Label>
          <Label muted>
            Result:{" "}
            {d.deliver_to.type === "chat"
              ? "Channel chat"
              : d.deliver_to.type === "notification"
                ? "Notification"
                : "Thread"}
          </Label>
          {row.source !== "schedule" && (
            <Label muted>
              Confirm:{" "}
              {d.confirmation_policy === "destructive"
                ? "Destructive actions"
                : "Every changing action"}
            </Label>
          )}
        </>
      ) : d.type === "webhook" ? (
        <Label selectable>{d.url}</Label>
      ) : d.type === "agent" ? (
        <Label mono>{d.conversation_id}</Label>
      ) : (
        <Label muted>Send a reminder notification.</Label>
      )}
    </Card>
  );
}

export function AutomationRunsScreen({
  navigation,
  route,
}: NativeStackScreenProps<RootStackParamList, "AutomationRuns">) {
  const runtime = useAutomationRuntime();
  const settings = useQuery({
    queryKey: AUTOMATION_SETTINGS,
    queryFn: ({ signal }) => automationsApi.settings(signal),
    enabled: runtime.active,
    retry: false,
  });
  const history = useInfiniteQuery({
    queryKey: [...AUTOMATIONS, route.params.id, "runs"],
    initialPageParam: undefined as AutomationRunsCursor | undefined,
    queryFn: ({ signal, pageParam }) =>
      automationsApi.runs(route.params.id, pageParam, signal),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: runtime.active,
    retry: false,
    refetchInterval: runtime.active ? 5000 : false,
  });
  const zone = settings.data?.timezone ?? deviceTimezone();
  const runs = uniqueAutomationRuns(history.data?.pages ?? []);
  return (
    <WorkspacePage title="Run history" showHeading={false}>
      <Label muted>Up to 1,000 recent runs are retained for 90 days.</Label>
      <Label muted style={s.small}>
        Times shown in {zone}
      </Label>
      {history.isPending && <AutomationLoading />}
      {history.error && (
        <>
          <Notice tone="error" message={automationError(history.error)} />
          <ActionButton
            label="Retry run history"
            variant="secondary"
            onPress={() => void history.refetch()}
          />
        </>
      )}
      {history.data && !runs.length && (
        <Card>
          <Label style={s.title}>No runs yet</Label>
          <Label muted>
            Scheduled, requested and event runs will appear here.
          </Label>
        </Card>
      )}
      {runs.map((run) => (
        <Card key={run.id}>
          <View style={s.spread}>
            <AutomationStatus value={run.outcome} />
            <Label muted style={s.small}>
              {run.duration_ms == null
                ? ""
                : `${(run.duration_ms / 1000).toFixed(1)} seconds`}
            </Label>
          </View>
          <Label>
            Scheduled: {formatAutomationTime(run.scheduled_at, zone)}
          </Label>
          {run.started_at && (
            <Label muted style={s.small}>
              Started: {formatAutomationTime(run.started_at, zone)}
            </Label>
          )}
          {run.finished_at && (
            <Label muted style={s.small}>
              Finished: {formatAutomationTime(run.finished_at, zone)}
            </Label>
          )}
          {run.reason && (
            <Label selectable>{run.reason.replaceAll("_", " ")}</Label>
          )}
          {run.outcome === "waiting" &&
            run.reason === "confirmation_required" && (
              <Notice message="Open the thread to review the required action card." />
            )}
          {run.missed && (
            <Label muted>
              Missed {run.missed.count} occurrences between{" "}
              {formatAutomationTime(run.missed.from, zone)} and{" "}
              {formatAutomationTime(run.missed.through, zone)}.
            </Label>
          )}
          {run.thread_id && (
            <ActionButton
              label="Open thread"
              variant="secondary"
              onPress={() =>
                navigation.push("AssistantContext", {
                  conversationId: run.thread_id!,
                })
              }
            />
          )}
        </Card>
      ))}
      {history.hasNextPage && (
        <ActionButton
          label="Load older runs"
          variant="secondary"
          disabled={!runtime.online}
          loading={history.isFetchingNextPage}
          onPress={() => void history.fetchNextPage()}
        />
      )}
    </WorkspacePage>
  );
}
