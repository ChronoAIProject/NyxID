import { useEffect, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type {
  WorkspaceContentParamList,
  WorkspaceScreenProps,
} from "../../app/navigationTypes";
import { useOpenWorkspaceChat } from "../../app/WorkspaceHeader";
import { newAssistantChatParams } from "../../app/assistantChatParams";
import { AgentAvatar } from "../../components/AgentAvatar";
import { SegmentControl } from "../../components/SegmentControl";
import {
  SettingsCheckRow,
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { automationsApi } from "../../lib/api/automationsApi";
import type {
  Automation,
  AutomationRun,
  AutomationRunsCursor,
} from "./automationContracts";
import { deviceTimezone, formatAutomationTime } from "./automationTime";
import {
  automationListDetail,
  automationPauseReason,
  automationReason,
  automationRunDetail,
  automationRunDuration,
  automationScheduleSummary,
  automationSourceLabel,
  automationStatusLabel,
  automationTarget,
  uniqueAutomationRuns,
} from "./automationPresentation";
import {
  AUTOMATIONS,
  AUTOMATION_SETTINGS,
  AutomationLoading,
  automationError,
  useAutomationRuntime,
  usePullRefresh,
} from "./AutomationUI";

const STATUSES = ["all", "active", "disabled"] as const;
const AGENTS_KEY = ["workspace", "agents", "list"] as const;

function useAgents(enabled: boolean) {
  return useQuery({
    queryKey: AGENTS_KEY,
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled,
    retry: false,
  });
}

function useAutomationZone(enabled: boolean) {
  const settings = useQuery({
    queryKey: AUTOMATION_SETTINGS,
    queryFn: ({ signal }) => automationsApi.settings(signal),
    enabled,
    retry: false,
  });
  return settings.data?.timezone ?? deviceTimezone();
}

function useAutomationRuns(id: string, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: [...AUTOMATIONS, id, "runs"],
    initialPageParam: undefined as AutomationRunsCursor | undefined,
    queryFn: ({ signal, pageParam }) =>
      automationsApi.runs(id, pageParam, signal),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled,
    retry: false,
    refetchInterval: enabled ? 5000 : false,
  });
}

/** A failed load: the message, then a tinted Retry row. */
function AutomationRetry({
  error,
  label,
  onRetry,
}: {
  error: unknown;
  label: string;
  onRetry: () => void;
}) {
  return (
    <>
      <Notice tone="error" message={automationError(error)} />
      <SettingsGroup inset={false}>
        <SettingsRow
          tone="action"
          label="Retry"
          accessibilityLabel={label}
          onPress={onRetry}
        />
      </SettingsGroup>
    </>
  );
}

export function AutomationsScreen({
  navigation,
  route,
}: WorkspaceScreenProps<"Automations">) {
  const runtime = useAutomationRuntime();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<(typeof STATUSES)[number]>("all");
  const [agentId, setAgentId] = useState(route.params?.agentId ?? "");
  const list = useQuery({
    queryKey: [...AUTOMATIONS, "list"],
    queryFn: ({ signal }) => automationsApi.list(signal),
    enabled: runtime.active,
    retry: false,
    refetchInterval: runtime.active ? 15_000 : false,
  });
  const agents = useAgents(runtime.active);
  const zone = useAutomationZone(runtime.active);
  const pull = usePullRefresh(() => list.refetch());
  useEffect(() => {
    setAgentId(route.params?.agentId ?? "");
  }, [route.params?.agentId]);
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
  const agent = agents.data?.agents.find((item) => item.id === agentId);
  const agentLabel = !agentId
    ? "All agents"
    : agent
      ? agent.display_name || agent.name
      : "Selected agent";
  return (
    <SettingsPage refreshing={pull.refreshing} onRefresh={pull.onRefresh}>
      <SegmentControl
        accessibilityLabel="Status"
        segments={[{ label: "All" }, { label: "Active" }, { label: "Paused" }]}
        activeIndex={STATUSES.indexOf(status)}
        onPress={(index) => setStatus(STATUSES[index] ?? "all")}
      />
      <SettingsGroup inset={false}>
        <SettingsTextField
          accessibilityLabel="Search automations"
          placeholder="Search by name or agent"
          value={search}
          onChangeText={setSearch}
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
        />
        <SettingsRow
          label="Agent"
          value={agentLabel}
          onPress={() =>
            navigation.push("AutomationAgentFilter", {
              agentId: agentId || undefined,
            })
          }
        />
      </SettingsGroup>
      {!runtime.online && (
        <Notice message="You’re offline. Reconnect to refresh automations." />
      )}
      {list.isPending && <AutomationLoading />}
      {list.error && (
        <AutomationRetry
          error={list.error}
          label="Retry automations"
          onRetry={() => void list.refetch()}
        />
      )}
      {list.data && (
        <SettingsGroup
          inset={false}
          footer={`Tasks that run for you on a schedule or when an event arrives. Times shown in ${zone}.`}
        >
          {rows.map((row) => {
            const value = automationStatusLabel(row.status);
            const detail = automationListDetail(
              row,
              automationTarget(row, agents.data?.agents),
              zone,
            );
            return (
              <SettingsRow
                key={row.id}
                label={row.label}
                detail={detail}
                value={value}
                accessibilityLabel={`${row.label}, ${value}. ${detail}`}
                accessibilityHint="Opens this automation"
                onPress={() =>
                  navigation.push("AutomationDetail", { id: row.id })
                }
              />
            );
          })}
          {!rows.length && (
            <SettingsRow
              label={
                list.data.triggers.length
                  ? "No matching automations"
                  : "No automations yet"
              }
              detail={
                list.data.triggers.length
                  ? "Try another search, agent or status."
                  : "Your configured automations will appear here."
              }
            />
          )}
        </SettingsGroup>
      )}
      <AutomationGuidance />
    </SettingsPage>
  );
}

/** Automations › Agent: a checkmark list; the choice filters the list it came from. */
export function AutomationAgentFilterScreen({
  navigation,
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "AutomationAgentFilter">) {
  const runtime = useAutomationRuntime();
  const agents = useAgents(runtime.active);
  const selected = route.params?.agentId ?? "";
  const choose = (agentId: string) =>
    navigation.popTo(
      "Automations",
      { agentId: agentId || undefined },
      { merge: true },
    );
  const live = (agents.data?.agents ?? []).filter(
    (agent) => agent.status !== "destroyed",
  );
  return (
    <SettingsPage>
      {agents.error && (
        <AutomationRetry
          error={agents.error}
          label="Retry agents"
          onRetry={() => void agents.refetch()}
        />
      )}
      <SettingsGroup>
        <SettingsCheckRow
          label="All agents"
          selected={!selected}
          onPress={() => choose("")}
        />
        {live.map((agent) => (
          <SettingsCheckRow
            key={agent.id}
            leading={<AgentAvatar agent={agent} size="sm" />}
            label={agent.display_name || agent.name}
            selected={agent.id === selected}
            onPress={() => choose(agent.id)}
          />
        ))}
        {selected && !live.some((agent) => agent.id === selected) ? (
          <SettingsCheckRow
            label="Selected agent"
            selected
            onPress={() => choose(selected)}
          />
        ) : null}
      </SettingsGroup>
      {agents.isPending && <AutomationLoading />}
    </SettingsPage>
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
  const agents = useAgents(runtime.active);
  const zone = useAutomationZone(runtime.active);
  const pull = usePullRefresh(() => detail.refetch());
  const row = detail.data;
  const target = row ? automationTarget(row, agents.data?.agents) : "";
  const nextRuns = row
    ? row.status === "active" && row.source === "schedule"
      ? row.next_runs?.length
        ? row.next_runs
        : row.next_run_at
          ? [row.next_run_at]
          : []
      : []
    : [];
  const pause = row
    ? [
        automationPauseReason(row.pause_reason),
        row.status === "disabled"
          ? "Future scheduled runs are paused. Existing run threads remain available."
          : undefined,
      ]
        .filter(Boolean)
        .join("\n\n")
    : "";
  return (
    <SettingsPage refreshing={pull.refreshing} onRefresh={pull.onRefresh}>
      {detail.isPending && <AutomationLoading />}
      {detail.error && (
        <AutomationRetry
          error={detail.error}
          label="Retry automation"
          onRetry={() => void detail.refetch()}
        />
      )}
      {row && (
        <>
          <SettingsGroup inset={false} footer={pause || undefined}>
            <SettingsRow label="Name" value={row.label} />
            <SettingsRow
              label="Status"
              value={automationStatusLabel(row.status)}
            />
            <SettingsRow label="Runs for" value={target} />
            <SettingsRow label="Starts" value={automationSourceLabel(row)} />
          </SettingsGroup>
          <SettingsGroup
            title="Schedule"
            inset={false}
            footer={`Times shown in ${zone}.`}
          >
            <SettingsRow
              label="Repeats"
              value={automationScheduleSummary(row)}
            />
            {row.schedule?.kind === "cron" && (
              <SettingsRow
                label="Calendar expression"
                value={row.schedule.expression}
              />
            )}
            {nextRuns.map((at, index) => (
              <SettingsRow
                key={at}
                label={index === 0 ? "Next run" : "Then"}
                value={formatAutomationTime(at, zone)}
              />
            ))}
            <SettingsRow
              label="Overlap"
              value={
                row.overlap === "queue"
                  ? "Queue one run"
                  : "Skip overlapping runs"
              }
            />
          </SettingsGroup>
          <AutomationDelivery row={row} />
          <SettingsGroup title="Runs" inset={false}>
            {row.last_run ? (
              <SettingsRow
                label="Latest run"
                detail={[
                  formatAutomationTime(row.last_run.scheduled_at, zone),
                  automationReason(row.last_run.reason),
                ]
                  .filter(Boolean)
                  .join("\n")}
                value={automationStatusLabel(row.last_run.outcome)}
                valueTone={
                  row.last_run.outcome === "failed" ? "danger" : "default"
                }
              />
            ) : null}
            <SettingsRow
              label="Run history"
              onPress={() => navigation.push("AutomationRuns", { id: row.id })}
            />
          </SettingsGroup>
          {row.delivery.type === "assistant" && (
            <SettingsGroup
              inset={false}
              footer="Normal agent usage and billing apply. This automation uses existing agent permissions."
            >
              <SettingsRow
                label="Agent permissions"
                onPress={() => {
                  if (row.delivery.type === "assistant")
                    navigation.push("AgentDetail", {
                      agentId: row.delivery.agent_id,
                    });
                }}
              />
            </SettingsGroup>
          )}
          <AutomationGuidance />
        </>
      )}
    </SettingsPage>
  );
}

/** Ask NyxBot: the one way to create, run or change an automation. */
function AutomationGuidance() {
  const openChat = useOpenWorkspaceChat();
  return (
    <SettingsGroup
      inset={false}
      footer="To create, run or change automations, ask NyxBot."
    >
      <SettingsRow
        tone="action"
        label="Ask NyxBot"
        onPress={() => openChat(newAssistantChatParams())}
      />
    </SettingsGroup>
  );
}

function AutomationDelivery({ row }: { row: Automation }) {
  const d = row.delivery;
  if (d.type === "assistant")
    return (
      <>
        <SettingsGroup title="Instruction" inset={false}>
          <SettingsRow label={d.instruction} selectable />
        </SettingsGroup>
        <SettingsGroup title="Delivery" inset={false}>
          <SettingsRow
            label="Thread"
            value={d.thread_policy ?? "Default for this agent and source"}
          />
          <SettingsRow
            label="Result"
            value={
              d.deliver_to.type === "chat"
                ? "Channel chat"
                : d.deliver_to.type === "notification"
                  ? "Notification"
                  : "Thread"
            }
          />
          <SettingsRow
            label="Confirmation"
            value={
              d.confirmation_policy === "destructive"
                ? "Destructive actions"
                : "Every changing action"
            }
          />
        </SettingsGroup>
      </>
    );
  return (
    <SettingsGroup title="Delivery" inset={false}>
      {d.type === "webhook" ? (
        <SettingsRow label={d.url} selectable />
      ) : d.type === "agent" ? (
        <SettingsRow label={d.conversation_id} mono selectable />
      ) : (
        <SettingsRow label="Send a reminder notification." />
      )}
    </SettingsGroup>
  );
}

export function AutomationRunsScreen({
  navigation,
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "AutomationRuns">) {
  const runtime = useAutomationRuntime();
  const zone = useAutomationZone(runtime.active);
  const history = useAutomationRuns(route.params.id, runtime.active);
  const pull = usePullRefresh(() => history.refetch());
  const runs = uniqueAutomationRuns(history.data?.pages ?? []);
  return (
    <SettingsPage refreshing={pull.refreshing} onRefresh={pull.onRefresh}>
      {history.isPending && <AutomationLoading />}
      {history.error && (
        <AutomationRetry
          error={history.error}
          label="Retry run history"
          onRetry={() => void history.refetch()}
        />
      )}
      {history.data && (
        <SettingsGroup
          inset={false}
          footer={`Up to 1,000 recent runs are retained for 90 days. Times shown in ${zone}.`}
        >
          {runs.map((run) => {
            const value = automationStatusLabel(run.outcome);
            const label = formatAutomationTime(run.scheduled_at, zone);
            const detail = automationRunDetail(run) || undefined;
            return (
              <SettingsRow
                key={run.id}
                label={label}
                detail={detail}
                value={value}
                valueTone={run.outcome === "failed" ? "danger" : "default"}
                accessibilityLabel={`Run scheduled ${label}, ${value}${detail ? `. ${detail}` : ""}`}
                accessibilityHint="Opens this run"
                onPress={() =>
                  navigation.push("AutomationRunDetail", {
                    id: route.params.id,
                    runId: run.id,
                  })
                }
              />
            );
          })}
          {!runs.length && (
            <SettingsRow
              label="No runs yet"
              detail="Scheduled, requested and event runs will appear here."
            />
          )}
          {history.hasNextPage && (
            <SettingsRow
              tone="action"
              label={
                history.isFetchingNextPage ? "Loading…" : "Load Older Runs"
              }
              accessibilityLabel="Load older runs"
              accessibilityState={{ busy: history.isFetchingNextPage }}
              disabled={!runtime.online || history.isFetchingNextPage}
              onPress={() => void history.fetchNextPage()}
            />
          )}
        </SettingsGroup>
      )}
    </SettingsPage>
  );
}

/** Run history › one run: its times, outcome and thread. */
export function AutomationRunDetailScreen({
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "AutomationRunDetail">) {
  const openChat = useOpenWorkspaceChat();
  const runtime = useAutomationRuntime();
  const zone = useAutomationZone(runtime.active);
  // Shares the run history cache, so the run opens without another request.
  const history = useAutomationRuns(route.params.id, runtime.active);
  const run: AutomationRun | undefined = uniqueAutomationRuns(
    history.data?.pages ?? [],
  ).find((item) => item.id === route.params.runId);
  if (!run)
    return (
      <SettingsPage>
        {history.isPending ? (
          <AutomationLoading />
        ) : history.error ? (
          <AutomationRetry
            error={history.error}
            label="Retry run"
            onRetry={() => void history.refetch()}
          />
        ) : (
          <Notice message="This run is no longer in the recent run history." />
        )}
      </SettingsPage>
    );
  const duration = automationRunDuration(run);
  const reason = automationReason(run.reason);
  return (
    <SettingsPage>
      <SettingsGroup
        inset={false}
        footer={
          run.outcome === "waiting" && run.reason === "confirmation_required"
            ? `Open the thread to review the required action card. Times shown in ${zone}.`
            : `Times shown in ${zone}.`
        }
      >
        <SettingsRow
          label="Outcome"
          value={automationStatusLabel(run.outcome)}
          valueTone={run.outcome === "failed" ? "danger" : "default"}
        />
        <SettingsRow
          label="Scheduled"
          value={formatAutomationTime(run.scheduled_at, zone)}
        />
        {run.started_at ? (
          <SettingsRow
            label="Started"
            value={formatAutomationTime(run.started_at, zone)}
          />
        ) : null}
        {run.finished_at ? (
          <SettingsRow
            label="Finished"
            value={formatAutomationTime(run.finished_at, zone)}
          />
        ) : null}
        {duration ? <SettingsRow label="Duration" value={duration} /> : null}
        {reason ? <SettingsRow label="Reason" value={reason} /> : null}
      </SettingsGroup>
      {run.missed && (
        <SettingsGroup title="Missed" inset={false}>
          <SettingsRow
            label={`Missed ${run.missed.count} occurrences between ${formatAutomationTime(run.missed.from, zone)} and ${formatAutomationTime(run.missed.through, zone)}.`}
          />
        </SettingsGroup>
      )}
      {run.thread_id && (
        <SettingsGroup inset={false}>
          <SettingsRow
            tone="action"
            label="Open Thread"
            onPress={() => openChat({ conversationId: run.thread_id! })}
          />
        </SettingsGroup>
      )}
    </SettingsPage>
  );
}
