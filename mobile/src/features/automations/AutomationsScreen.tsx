import type { ReactNode } from "react";
import { AgentAvatar } from "../../components/AgentAvatar";
import { useEffect, useState } from "react";
import { Pressable, View } from "react-native";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ChevronRight, Clock3 } from "lucide-react-native";
import type {
  WorkspaceContentParamList,
  WorkspaceScreenProps,
} from "../../app/navigationTypes";
import { useOpenWorkspaceChat } from "../../app/WorkspaceHeader";
import { newAssistantChatParams } from "../../app/assistantChatParams";
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
import { useTheme } from "../../theme/ThemeContext";
import type { Automation, AutomationRunsCursor } from "./automationContracts";
import { deviceTimezone, formatAutomationTime } from "./automationTime";
import {
  automationPauseReason,
  automationScheduleSummary,
  automationTarget,
  uniqueAutomationRuns,
} from "./automationPresentation";
import {
  AUTOMATIONS,
  AUTOMATION_SETTINGS,
  AutomationLoading,
  AutomationStatus,
  ChoiceField,
  automationError,
  automationStyles as s,
  useAutomationRuntime,
} from "./AutomationUI";

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
      .map((a) => ({
        value: a.id,
        label: a.display_name || a.name,
        leading: (<AgentAvatar agent={a} size="lg" />) as ReactNode,
      })) ?? [];
  if (agentId && !agentOptions.some((a) => a.value === agentId))
    agentOptions.push({
      value: agentId,
      label: "Selected agent",
      leading: undefined,
    });
  return (
    <WorkspacePage title="Automations" showHeading={false}>
      <View style={s.spread}>
        <View style={s.flex}>
          <Label style={s.section}>Tasks that run for you</Label>
          <Label muted style={s.small}>
            On a schedule or when an event arrives.
          </Label>
        </View>
      </View>
      <Label muted style={s.small}>
        Times shown in {zone}
      </Label>
      <AutomationGuidance />
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
        <Notice message="You’re offline. Reconnect to refresh automations." />
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
                  : `Next: ${formatAutomationTime(row.next_run_at, zone)}`}
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
              : "No automations yet"}
          </Label>
          <Label muted>
            {list.data.triggers.length
              ? "Try another search, agent or status."
              : "Your configured automations will appear here."}
          </Label>
        </Card>
      )}
    </WorkspacePage>
  );
}

export function AutomationDetailScreen({
  navigation,
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "AutomationDetail">) {
  const runtime = useAutomationRuntime();
  const detail = useQuery({
    queryKey: [...AUTOMATIONS, "detail", route.params.id],
    queryFn: ({ signal }) => automationsApi.detail(route.params.id, signal),
    enabled: runtime.active,
    retry: false,
    refetchInterval: runtime.active ? 5000 : false,
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
  const row = detail.data;
  const zone = settings.data?.timezone ?? deviceTimezone();
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
      {row && (
        <>
          <Label muted style={s.small}>
            Times shown in {zone}
          </Label>
          <AutomationStatus value={row.status} />
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
            <Notice message="Future scheduled runs are paused. Existing run threads remain available." />
          )}
          <Card>
            <Label style={s.title}>Schedule</Label>
            <Label>{automationScheduleSummary(row)}</Label>
            {row.schedule?.kind === "cron" && (
              <Label muted mono style={s.small}>
                Calendar expression: {row.schedule.expression}
              </Label>
            )}
            {row.status === "active" &&
              row.source === "schedule" &&
              (row.next_runs?.length
                ? row.next_runs
                : row.next_run_at
                  ? [row.next_run_at]
                  : []
              ).map((at) => (
                <Label key={at} mono style={s.small}>
                  {formatAutomationTime(at, zone)}
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
                {formatAutomationTime(row.last_run.scheduled_at, zone)}
              </Label>
              {row.last_run.reason && (
                <Label>{row.last_run.reason.replaceAll("_", " ")}</Label>
              )}
            </Card>
          )}
          <ActionButton
            label="View run history"
            variant="secondary"
            onPress={() => navigation.push("AutomationRuns", { id: row.id })}
          />
          <AutomationGuidance />
          {row.delivery.type === "assistant" && (
            <Label muted style={s.small}>
              Normal agent usage and billing apply. This automation uses
              existing agent permissions.
            </Label>
          )}
        </>
      )}
    </WorkspacePage>
  );
}

function AutomationGuidance() {
  const openChat = useOpenWorkspaceChat();
  return (
    <Card>
      <Label muted>To create, run or change automations, ask NyxBot.</Label>
      <ActionButton
        label="Ask NyxBot"
        variant="secondary"
        onPress={() => openChat(newAssistantChatParams())}
      />
    </Card>
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
          <Label muted>
            Confirmation:{" "}
            {d.confirmation_policy === "destructive"
              ? "Destructive actions"
              : "Every changing action"}
          </Label>
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
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "AutomationRuns">) {
  const openChat = useOpenWorkspaceChat();
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
              onPress={() => openChat({ conversationId: run.thread_id! })}
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
