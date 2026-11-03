import { useEffect, useState } from "react";
import { Pressable, StyleSheet, TextInput, View } from "react-native";
import {
  Bell,
  CalendarClock,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Repeat,
  Search,
  Sparkles,
  Webhook,
  X,
} from "lucide-react-native";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type {
  WorkspaceContentParamList,
  WorkspaceScreenProps,
} from "../../app/navigationTypes";
import {
  useOpenWorkspaceChat,
  useWorkspaceSheet,
} from "../../app/WorkspaceHeader";
import { workspaceChatNavigationAction } from "../../app/workspaceChatNavigation";
import {
  assistantDraftKey,
  assistantDraftStore,
} from "../assistant/assistantDraftStore";
import { newAssistantChatParams } from "../../app/assistantChatParams";
import { AgentAvatar } from "../../components/AgentAvatar";
import {
  SettingsCheckRow,
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  usePopupMenu,
} from "../../components/settings/SettingsList";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { automationsApi } from "../../lib/api/automationsApi";
import type { AssistantAgent } from "../agents/agentContracts";
import { radius, spacing, typeScale, fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
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

// One blank, at the end: the reader finishes the sentence with when and what.
export const AUTOMATION_SETUP_PROMPT =
  "Hi NyxBot, I'd like help setting up an automation that runs every ";

/**
 * Open a new NyxBot chat as the main chat (not a page pushed over this one)
 * with its composer already holding a starter message the owner completes.
 */
function useStartNyxBotChat(navigation: {
  getParent: () => { getState: () => any } | undefined;
  dispatch: (action: any) => void;
}) {
  const sheet = useWorkspaceSheet();
  return (prompt: string) => {
    const params = newAssistantChatParams();
    assistantDraftStore.set(
      `text:${assistantDraftKey(undefined, undefined, params.draftId)}`,
      prompt,
    );
    if (sheet) {
      sheet.onOpenChat(params);
      return;
    }
    const action = workspaceChatNavigationAction(
      navigation.getParent()?.getState(),
      { kind: "new", params },
    );
    if (action) navigation.dispatch(action);
  };
}

const STATUS_OPTIONS = [
  { value: "all", label: "All" },
  { value: "active", label: "Active" },
  { value: "disabled", label: "Paused" },
] as const;

export function AutomationsScreen({
  navigation,
  route,
}: WorkspaceScreenProps<"Automations">) {
  const { colors } = useTheme();
  const runtime = useAutomationRuntime();
  const startChat = useStartNyxBotChat(navigation);
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<(typeof STATUSES)[number]>("all");
  const [agentId, setAgentId] = useState(route.params?.agentId ?? "");
  const statusMenu = usePopupMenu({
    options: STATUS_OPTIONS,
    value: status,
    onChange: setStatus,
  });
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
  const statusLabel =
    STATUS_OPTIONS.find((option) => option.value === status)?.label ?? "All";
  return (
    <SettingsPage refreshing={pull.refreshing} onRefresh={pull.onRefresh}>
      <AskNyxBotCard
        colors={colors}
        onPress={() => startChat(AUTOMATION_SETUP_PROMPT)}
      />
      <View style={styles.filters}>
        <View style={[styles.search, { backgroundColor: colors.segmentTrack }]}>
          <Search size={17} color={colors.textMuted} accessible={false} />
          <TextInput
            accessibilityLabel="Search automations"
            placeholder="Search by name or agent"
            placeholderTextColor={colors.textMuted}
            selectionColor={colors.primary}
            value={search}
            onChangeText={setSearch}
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="search"
            style={[styles.searchInput, { color: colors.textPrimary }]}
          />
          {search ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Clear search"
              onPress={() => setSearch("")}
              style={styles.clear}
            >
              <X size={16} color={colors.textMuted} />
            </Pressable>
          ) : null}
        </View>
        <View style={styles.chips}>
          <View ref={statusMenu.anchor} collapsable={false}>
            <FilterChip
              colors={colors}
              label={status === "all" ? "Status" : statusLabel}
              accessibilityLabel={`Status, ${statusLabel}`}
              active={status !== "all"}
              menu
              onPress={statusMenu.open}
            />
          </View>
          <FilterChip
            colors={colors}
            label={agentLabel}
            accessibilityLabel={`Agent, ${agentLabel}`}
            active={!!agentId}
            onPress={() =>
              navigation.push("AutomationAgentFilter", {
                agentId: agentId || undefined,
              })
            }
          />
        </View>
        {statusMenu.popup}
      </View>
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
        <View style={styles.cards}>
          {rows.map((row) => (
            <AutomationCard
              key={row.id}
              row={row}
              colors={colors}
              zone={zone}
              agent={
                row.delivery.type === "assistant"
                  ? agents.data?.agents.find(
                      (item) =>
                        row.delivery.type === "assistant" &&
                        item.id === row.delivery.agent_id,
                    )
                  : undefined
              }
              target={automationTarget(row, agents.data?.agents)}
              onPress={() => navigation.push("AutomationDetail", { id: row.id })}
            />
          ))}
          {!rows.length && (
            <View style={[styles.card, { backgroundColor: colors.groupedFill }]}>
              <Text style={[styles.cardTitle, { color: colors.textPrimary }]}>
                {list.data.triggers.length
                  ? "No matching automations"
                  : "No automations yet"}
              </Text>
              <Text style={[styles.cardMeta, { color: colors.textMuted }]}>
                {list.data.triggers.length
                  ? "Try another search, agent or status."
                  : "Ask NyxBot to run something for you on a schedule or when an event arrives."}
              </Text>
            </View>
          )}
          <Text style={[styles.footnote, { color: colors.textMuted }]}>
            {`Times shown in ${zone}.`}
          </Text>
        </View>
      )}
    </SettingsPage>
  );
}

/** Creation happens in chat, so the way in sits first on the page. */
function AskNyxBotCard({
  colors,
  onPress,
}: {
  colors: ThemeColors;
  onPress: () => void;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Ask NyxBot"
      accessibilityHint="Opens a new NyxBot chat with a message to finish describing your automation"
      onPress={onPress}
      style={({ pressed }) => [
        styles.ask,
        {
          backgroundColor: colors.primaryTone.bg,
          borderColor: colors.primaryTone.border,
        },
        pressed && { opacity: 0.7 },
      ]}
    >
      <View style={[styles.askIcon, { backgroundColor: colors.primary }]}>
        <Sparkles size={18} color={colors.onPrimary} />
      </View>
      <View style={styles.askText}>
        <Text style={[styles.cardTitle, { color: colors.textPrimary }]}>
          Ask NyxBot
        </Text>
        <Text style={[styles.cardMeta, { color: colors.textSecondary }]}>
          Start a chat to set up an automation. Just finish the sentence with
          when it runs and what it does.
        </Text>
      </View>
      <ChevronRight size={18} color={colors.textMuted} />
    </Pressable>
  );
}

function FilterChip({
  colors,
  label,
  accessibilityLabel,
  active,
  menu = false,
  onPress,
}: {
  colors: ThemeColors;
  label: string;
  accessibilityLabel: string;
  active: boolean;
  menu?: boolean;
  onPress: () => void;
}) {
  const Glyph = menu ? ChevronDown : ChevronRight;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityHint={menu ? "Opens a menu" : "Opens the agent list"}
      onPress={onPress}
      style={({ pressed }) => [
        styles.chip,
        {
          backgroundColor: active ? colors.primaryTone.bg : colors.segmentTrack,
          borderColor: active ? colors.primaryTone.border : "transparent",
        },
        pressed && { opacity: 0.7 },
      ]}
    >
      <Text
        numberOfLines={1}
        style={[
          styles.chipLabel,
          { color: active ? colors.primaryOnTint : colors.textPrimary },
        ]}
      >
        {label}
      </Text>
      <Glyph
        size={14}
        color={active ? colors.primaryOnTint : colors.textSecondary}
      />
    </Pressable>
  );
}

/** One automation as a card: who runs it, its state, when it runs next. */
function AutomationCard({
  row,
  colors,
  zone,
  agent,
  target,
  onPress,
}: {
  row: Automation;
  colors: ThemeColors;
  zone: string;
  agent?: AssistantAgent;
  target: string;
  onPress: () => void;
}) {
  const status = automationStatusLabel(row.status);
  const active = row.status === "active";
  // The page footnote names the zone once; cards drop the repeat.
  const schedule = automationScheduleSummary(row).replace(` (${zone})`, "");
  const next =
    row.source === "schedule"
      ? active
        ? `Next ${formatAutomationTime(row.next_run_at, zone)}`
        : "No new scheduled runs while paused"
      : undefined;
  const pause = automationPauseReason(row.pause_reason);
  const detail = automationListDetail(row, target, zone);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${row.label}, ${status}. ${detail}`}
      accessibilityHint="Opens this automation"
      onPress={onPress}
      style={({ pressed }) => [
        styles.card,
        { backgroundColor: colors.groupedFill },
        pressed && { backgroundColor: colors.navActive },
      ]}
    >
      <View style={styles.cardHead}>
        {agent ? (
          <AgentAvatar agent={agent} size="sm" />
        ) : (
          <View style={[styles.cardIcon, { backgroundColor: colors.segmentTrack }]}>
            {row.delivery.type === "notification" ? (
              <Bell size={15} color={colors.textSecondary} />
            ) : (
              <Webhook size={15} color={colors.textSecondary} />
            )}
          </View>
        )}
        <View style={styles.cardHeadText}>
          <Text
            numberOfLines={2}
            style={[styles.cardTitle, { color: colors.textPrimary }]}
          >
            {row.label}
          </Text>
          <Text
            numberOfLines={1}
            style={[styles.cardMeta, { color: colors.textMuted }]}
          >
            {`${target} · ${automationSourceLabel(row)}`}
          </Text>
        </View>
        <View
          style={[
            styles.status,
            { backgroundColor: active ? colors.successSoft : colors.segmentTrack },
          ]}
        >
          <View
            style={[
              styles.statusDot,
              { backgroundColor: active ? colors.success : colors.textMuted },
            ]}
          />
          <Text
            numberOfLines={1}
            style={[
              styles.statusLabel,
              { color: active ? colors.success : colors.textSecondary },
            ]}
          >
            {status}
          </Text>
        </View>
      </View>
      <View style={[styles.cardRule, { backgroundColor: colors.borderSoft }]} />
      <View style={styles.cardLine}>
        <Repeat size={15} color={colors.textMuted} />
        <Text style={[styles.cardLineText, { color: colors.textSecondary }]}>
          {schedule}
        </Text>
      </View>
      {next ? (
        <View style={styles.cardLine}>
          <CalendarClock size={15} color={colors.textMuted} />
          <Text style={[styles.cardLineText, { color: colors.textSecondary }]}>
            {next}
          </Text>
        </View>
      ) : null}
      {pause ? (
        <View style={styles.cardLine}>
          <CircleAlert size={15} color={colors.warning} />
          <Text style={[styles.cardLineText, { color: colors.warning }]}>
            {pause}
          </Text>
        </View>
      ) : null}
    </Pressable>
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
          <AutomationGuidance label={row.label} navigation={navigation} />
        </>
      )}
    </SettingsPage>
  );
}

/** Ask NyxBot: the one way to create, run or change an automation. */
/** Details are read-only here; changes go through NyxBot. */
function AutomationGuidance({
  label,
  navigation,
}: {
  label: string;
  navigation: Parameters<typeof useStartNyxBotChat>[0];
}) {
  const startChat = useStartNyxBotChat(navigation);
  return (
    <SettingsGroup
      inset={false}
      footer="To run or change this automation, ask NyxBot."
    >
      <SettingsRow
        tone="action"
        label="Ask NyxBot"
        onPress={() =>
          startChat(`Hi NyxBot, I'd like to change my automation "${label}": `)
        }
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

const styles = StyleSheet.create({
  filters: { gap: spacing.md },
  search: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm - 2,
    borderRadius: radius.md + 2,
    paddingLeft: spacing.md,
    paddingRight: spacing.xs,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 40,
    paddingVertical: spacing.sm,
    ...typeScale.label,
  },
  clear: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    minHeight: 36,
    maxWidth: 260,
    paddingHorizontal: spacing.md + 2,
    borderRadius: 18,
    borderWidth: 1,
  },
  chipLabel: { ...typeScale.small, fontSize: 15, lineHeight: 20, flexShrink: 1 },
  cards: { gap: spacing.md },
  card: {
    borderRadius: radius.lg,
    padding: spacing.lg + 2,
    gap: spacing.sm,
  },
  cardHead: { flexDirection: "row", alignItems: "center", gap: spacing.md },
  cardIcon: {
    width: 28,
    height: 28,
    borderRadius: 14,
    alignItems: "center",
    justifyContent: "center",
  },
  cardHeadText: { flex: 1, minWidth: 0, gap: 2 },
  cardTitle: {
    ...typeScale.label,
    fontFamily: fonts.bodySemi,
    fontWeight: "600",
  },
  cardMeta: { ...typeScale.small },
  cardRule: { height: StyleSheet.hairlineWidth, marginVertical: spacing.xs },
  cardLine: { flexDirection: "row", alignItems: "flex-start", gap: spacing.sm },
  cardLineText: { ...typeScale.small, flex: 1, minWidth: 0 },
  status: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    borderRadius: 10,
    paddingHorizontal: spacing.sm,
    paddingVertical: 3,
  },
  statusDot: { width: 7, height: 7, borderRadius: 4 },
  statusLabel: { ...typeScale.small, fontSize: 13, lineHeight: 17 },
  footnote: { ...typeScale.small, paddingHorizontal: spacing.xxl },
  ask: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    borderRadius: radius.lg,
    borderWidth: 1,
    padding: spacing.lg + 2,
  },
  askIcon: {
    width: 36,
    height: 36,
    borderRadius: 18,
    alignItems: "center",
    justifyContent: "center",
  },
  askText: { flex: 1, minWidth: 0, gap: 2 },
});
