import { useContext } from "react";
import { WorkspaceCornerSensorContext } from "./WorkspaceLayoutContext";
import { workspaceAgentPickerSections } from "./workspaceAgentPickerModel";
import { recentUnread, type WorkspaceRecent } from "./workspaceSidebarModel";
import {
  workspaceCrewLabel,
  workspaceDisclosureLabel,
  workspaceRecentLabel,
  workspaceThreadLabel,
} from "./workspaceAccessibilityLabels";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import {
  sidebarAgentOwner,
  sidebarOrderedAgents,
} from "./workspaceSidebarModel";
import { AgentAvatar, AgentAvatarStack } from "../AgentAvatar";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  Text,
  ActivityIndicator,
  AppState,
  Pressable,
  RefreshControl,
  ScrollView,
  type ScrollViewProps,
  StyleSheet,
  View,
} from "react-native";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronRight,
  Plus,
  Users
} from "lucide-react-native";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import type { AssistantAgent } from "../../features/agents/agentContracts";
import type { AssistantConversation } from "../../features/assistant/assistantTypes";
import type { Group } from "../../features/groups/groupContracts";
import { chatAgentLabel } from "../../features/assistant/assistantAgentContext";
import {
  EMPTY_NOTIFICATION_MARKERS,
  type NotificationMarkers,
} from "../../features/notifications/notificationPresentation";
import { formatRelativeTimeFromMs } from "../../lib/time";
import { useGroupsLive } from "../../features/groups/useGroupsLive";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { groupKeys } from "../../features/groups/groupLiveRefresh";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { agentsApi } from "../../lib/api/agentsApi";
import { assistantApi } from "../../lib/api/assistantApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { SkeletonRows } from "../Skeleton";
import { WorkspaceChatBrowser } from "./WorkspaceChatBrowser";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import { SidebarSectionHeader as SectionHeading } from "./SidebarSectionHeader";
import {
  boundedDrawerPreview,
  DRAWER_PREVIEW_LIMIT,
  drawerAgentConversations,
  orderedDrawerAgents,
  splitChannelThreads,
  channelPlatformName,
  threadTitle,
  threadStatus,
} from "./workspaceChatListModel";

export type WorkspaceChatListProps = {
  mode?: "drawer" | "picker";
  disabled?: boolean;
  recentRows?: readonly WorkspaceRecent[];
  scrollFooter?: ReactNode;
  scrollHeader?: ReactNode;
  scrollProps?: ScrollViewProps;
  visible: boolean;
  current?: WorkspaceChatTarget;
  /** Unread notification counts per thread and crew from the one summary
   * the sidebar already holds; rows draw their markers from it. */
  unread?: NotificationMarkers;
  onSelect: (target: WorkspaceChatTarget) => void;
  onCreateAgent: () => void;
  onCreateGroup: () => void;
};
type BrowseRow =
  | { id: string; agent: AssistantAgent }
  | { id: string; group: Group }
  | { id: string; chat: AssistantConversation };
type Browser =
  |
  { kind: "agents" | "groups" }
  | { kind: "chats"; fromAgents: boolean };

export function WorkspaceChatList({
  mode = "drawer",
  disabled = false,
  recentRows = [],
  scrollFooter,
  scrollHeader,
  scrollProps,
  visible,
  current,
  unread: markers = EMPTY_NOTIFICATION_MARKERS,
  onSelect,
  onCreateAgent,
  onCreateGroup,
}: WorkspaceChatListProps) {
  useAppLanguage();
  const { colors } = useTheme();
  const epoch = getAuthEpoch();
  const { isConnected } = useNetworkStatus();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [sections, setSections] = useState({
    agents: current?.kind !== "group",
    groups: current?.kind === "group",
  });
  // A single disclosure bounds the height even when the account has many agents.
  const [expandedAgent, setExpandedAgent] = useState<string | null>();
  const [browser, setBrowser] = useState<Browser | null>(null);
  const [browserClosing, setBrowserClosing] = useState(0);
  const browserDismissalSequence = useRef(0);
  const browserClosingRef = useRef(false);
  const pendingBrowserSelection = useRef<(() => void) | undefined>(undefined);
  const onSelectRef = useRef(onSelect);
  onSelectRef.current = onSelect;
  const active = visible && foreground && isConnected;
  const activeRef = useRef(active);
  activeRef.current = active;
  const refreshLock = useRef(false);
  const [pullRefreshing, setPullRefreshing] = useState(false);
  const available = () =>
    visible &&
    AppState.currentState === "active" &&
    isConnected &&
    epoch === getAuthEpoch();
  const live = useGroupsLive(epoch, active, available);
  const [channelDisclosures, setChannelDisclosures] = useState<
    Record<string, { open: boolean; active?: string; shown: number }>
  >({});
  const [pastThreads, setPastThreads] = useState(false);
  const sticky = useRef({ bottom: Infinity, headerHeight: 0, scrollY: 0 });
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => {
      activeRef.current = false;
      subscription.remove();
    };
  }, []);
  useEffect(() => {
    if (!visible) {
      pendingBrowserSelection.current = undefined;
      browserClosingRef.current = false;
      setBrowser(null);
      setBrowserClosing(0);
    }
  }, [visible]);
  const agents = useQuery({
    queryKey: ["workspace", "agents", "drawer", epoch],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: active,
    // Reopening/resuming enables a stale read immediately; events and manual
    // refresh can update it sooner than this foreground fallback interval.
    staleTime: 0,
    refetchInterval: active ? 30_000 : false,
    retry: false,
  });
  // Only the person's own refresh spins; the 30s poll and event refetches
  // update rows silently.
  const refreshingAgents = pullRefreshing;
  const refreshAgents = async () => {
    if (
      refreshLock.current ||
      !activeRef.current ||
      AppState.currentState !== "active" ||
      epoch !== getAuthEpoch()
    )
      return;
    refreshLock.current = true;
    setPullRefreshing(true);
    try {
      await agents.refetch({ cancelRefetch: false });
    } finally {
      refreshLock.current = false;
      setPullRefreshing(false);
    }
  };
  const groups = useQuery({
    queryKey: [...groupKeys(epoch), "list"],
    queryFn: ({ signal }) => groupsApi.list(signal),
    enabled: active && mode === "drawer",
    refetchInterval: livePollInterval(5000, live),
    retry: false,
  });
  const indexedAgents = (
    mode === "drawer" ? sidebarOrderedAgents : orderedDrawerAgents
  )(agents.data?.agents ?? []);
  const allAgents = indexedAgents.filter(
    (agent) =>
      agent.status !== "destroyed" &&
      (mode !== "picker" || agent.can_use === true),
  );
  const allGroups = [...(groups.data ?? [])].sort((a, b) =>
    (b.last_message_at || b.created_at).localeCompare(
      a.last_message_at || a.created_at,
    ),
  );
  const cornerWindow = useContext(WorkspaceCornerSensorContext);
  const picker = workspaceAgentPickerSections(
    allAgents,
    mode === "picker" && cornerWindow ? recentRows : [],
  );
  const roster = mode === "picker" ? picker.roster : allAgents;
  const currentAgentId =
    current?.kind === "agent" ? current.agentId : undefined;
  // A current agent shown under Recent has no inline roster row to expand.
  const currentInRecent = picker.recent.some(
    (entry) => entry.agent.id === currentAgentId,
  );
  const openId =
    expandedAgent !== undefined
      ? expandedAgent
      : currentInRecent
        ? undefined
        : currentAgentId;
  // Retain historical context for an open chat/browser without adding deleted
  // agents back to the current roster or its count.
  const agent = (mode === "drawer" ? indexedAgents : allAgents).find(
    (row) => row.id === openId,
  );
  useEffect(() => {
    sticky.current.bottom = Infinity;
    setPastThreads(false);
  }, [openId, sections.agents]);
  const agentRef = useRef(agent);
  agentRef.current = agent;
  const canStartChat =
    !!agent && agent.status !== "destroyed" && agent.can_use !== false;
  const history = useInfiniteQuery({
    queryKey: ["workspace", "drawer-chats", epoch, agent?.id],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam, signal }) =>
      assistantApi.list(pageParam, agent!.id, signal),
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled:
      active &&
      !!agent &&
      agent.can_use !== false &&
      (sections.agents || browser?.kind === "chats"),
    refetchInterval: livePollInterval(5000, live),
    retry: false,
  });
  const chats =
    agent && agent.can_use !== false
      ? drawerAgentConversations(history.data?.pages ?? [], agent)
      : [];
  const selectedChatId =
    current?.kind === "agent" &&
    (current.agentId === agent?.id ||
      (!current.agentId &&
        chats.some((row) => row.id === current.conversationId)))
      ? current.conversationId
      : undefined;
  const missingCurrent =
    !!selectedChatId && !chats.some((row) => row.id === selectedChatId);
  const threads = splitChannelThreads(chats);
  const previewChats = boundedDrawerPreview(
    threads.own,
    selectedChatId,
    DRAWER_PREVIEW_LIMIT - (missingCurrent ? 1 : 0),
  );
  const dismissBrowser = (selection?: () => void) => {
    if (browserClosingRef.current) return;
    pendingBrowserSelection.current = selection;
    browserClosingRef.current = true;
    setBrowserClosing(++browserDismissalSequence.current);
  };
  const finishBrowserDismissal = (dismissalId: number) => {
    if (
      !browserClosingRef.current ||
      dismissalId !== browserDismissalSequence.current
    )
      return;
    browserClosingRef.current = false;
    const select = pendingBrowserSelection.current;
    pendingBrowserSelection.current = undefined;
    setBrowser(null);
    setBrowserClosing(0);
    select?.();
  };
  const selectAfterBrowser = (select: () => void) => {
    if (browserClosingRef.current) return;
    if (browser) dismissBrowser(select);
    else select();
  };
  const openChat = (conversationId?: string) => {
    const selectedAgentId = agent?.id;
    const allowed = () => {
      const latest = agentRef.current;
      return (
        latest &&
        latest.id === selectedAgentId &&
        latest.can_use !== false &&
        (!!conversationId || latest.status !== "destroyed")
      );
    };
    if (!allowed()) return;
    selectAfterBrowser(() => {
      // Permissions may change while the native browser animates out.
      if (allowed())
        onSelectRef.current({
          kind: "agent",
          agentId: selectedAgentId,
          conversationId,
        });
    });
  };
  const openGroup = (group: Group) => {
    selectAfterBrowser(() =>
      onSelectRef.current({ kind: "group", groupId: group.id }),
    );
  };
  const agentDetail = (row: AssistantAgent) =>
    row.status === "destroyed"
      ? translateApp("Past agent · chat history")
      : row.can_use === false
        ? translateApp("View only")
        : row.owner_kind === "org"
          ? row.owner_name || translateApp("Shared agent")
          : row.kind === "nyxbot"
            ? translateApp("Personal · Your personal agent")
            : translateApp("Personal");
  const agentRowDetail = (row: AssistantAgent) =>
    [
      row.pending_acknowledgements
        ? translateApp("Needs you")
        : row.status === "running"
          ? translateApp("Running")
          : undefined,
      agentDetail(row),
    ]
      .filter(Boolean)
      .join(" · ");
  const groupedAgentSubtitle = (row: AssistantAgent) =>
    [
      row.pending_acknowledgements
        ? translateApp("Needs you")
        : row.status === "running"
          ? translateApp("Running")
          : undefined,
      row.status === "destroyed"
        ? translateApp("Past agent · chat history")
        : row.can_use === false
          ? translateApp("View only")
          : row.kind === "nyxbot"
            ? translateApp("Your personal agent")
            : row.description,
    ]
      .filter(Boolean)
      .join(" · ");
  const chatRow = (row: AssistantConversation) => (
    <ChatRow
      disabled={disabled}
      key={row.id}
      label={threadTitle(row)}
      detail={formatRelativeTimeFromMs(Date.parse(row.last_message_at))}
      status={threadStatus(row)}
      selected={selectedChatId === row.id}
      unread={markers.conversations[row.id]}
      onPress={() => openChat(row.id)}
      accessibilityLabel={workspaceThreadLabel(
        threadTitle(row), agent ? chatAgentLabel(agent) : translateApp("Agent"),
        threadStatus(row), selectedChatId === row.id, markers.conversations[row.id],
      )}
    />
  );
  const groupRow = (row: Group) => (
    <GroupChatRow
      disabled={disabled}
      key={row.id}
      group={row}
      selected={current?.kind === "group" && current.groupId === row.id}
      unread={markers.groups[row.id]}
      onPress={() => openGroup(row)}
    />
  );
  // The full browser shows its own first-load skeleton in place of the list.
  const historyStatus = (showLoading: boolean) => (
    <>
      {agent?.can_use === false ? (
        <DrawerMessage text={translateApp(
            "View only · chat access isn’t available for this agent.",
          )}
        />
      ) : (
        <>
          {showLoading && history.isPending ? (
            <SkeletonRows
              rows={3}
              icon={false}
              accessibilityLabel={translateApp("Loading chats")}
            />
          ) : null}
          {history.isError ? (
            <DrawerMessage
              text={
                history.data
                  ? translateApp("Couldn’t refresh chats.")
                  : translateApp("Couldn’t load chats.")
              }
              retry={() => void history.refetch()}
              disabled={!active}
            />
          ) : null}
        </>
      )}
    </>
  );
  const startChat =
    canStartChat && agent ? (
      <ChatRow
        disabled={disabled}
        label={translateApp("New chat with {{label}}", {
          label: chatAgentLabel(agent),
        })}
        create
        selected={
          current?.kind === "agent" &&
          current.agentId === agent.id &&
          !current.conversationId
        }
        onPress={() => openChat()}
        accessibilityLabel={translateApp("New chat with {{label}}", {
          label: chatAgentLabel(agent),
        })}
      />
    ) : null;
  const currentRow = missingCurrent && agent?.can_use !== false ? (
    <ChatRow
      disabled={disabled}
      label={translateApp("Current chat")}
      selected
      onPress={() => openChat(selectedChatId)}
    />
  ) : null;
  const previewAgents = boundedDrawerPreview(
    roster,
    openId || currentAgentId
  );
  const ownerCounts = new Map<string, { total: number; shown: number }>();
  for (const row of allAgents) {
    const key = sidebarAgentOwner(row).key;
    const counts = ownerCounts.get(key) ?? { total: 0, shown: 0 };
    counts.total += 1;
    ownerCounts.set(key, counts);
  }
  for (const row of previewAgents)
    ownerCounts.get(sidebarAgentOwner(row).key)!.shown += 1;
  const rows: ReactNode[] = [<View key="workspace">{scrollHeader}</View>];
  if (mode === "picker" && picker.recent.length) {
    rows.push(
      <Text
        key="recent-heading"
        accessibilityRole="header"
        style={[styles.ownerHeading, { color: colors.textMuted }]}
      >
        {translateApp("Recent")}
      </Text>,
    );
    // Recent opens the agent's latest thread, exactly like a sidebar Recents row.
    for (const { agent: recentAgent, row } of picker.recent) {
      const label = chatAgentLabel(recentAgent);
      const unread = recentUnread(row, markers);
      rows.push(
        <ChatRow
          key={`recent:${recentAgent.id}`}
          agent={recentAgent}
          label={label}
          detail={formatRelativeTimeFromMs(Date.parse(row.activityAt))}
          selected={
            current?.kind === "agent" &&
            row.target.kind === "agent" &&
            current.conversationId === row.target.conversationId
          }
          unread={unread}
          status={row.status}
          accessibilityLabel={workspaceRecentLabel(label, row.status, unread)}
          onPress={() =>
            selectAfterBrowser(() => onSelectRef.current(row.target))
          }
        />,
      );
    }
    if (roster.length)
      rows.push(
        <Text
          key="roster-heading"
          accessibilityRole="header"
          style={[styles.ownerHeading, { color: colors.textMuted }]}
        >
          {translateApp("Agents")}
        </Text>,
      );
  }
  if (mode === "drawer")
    rows.push(
      <SectionHeading
        disabled={disabled}
        key="agents"
        section="agents"
        label={translateApp("Agents")}
        count={translateApp("{{count}} total", { count: allAgents.length })}
        expanded={sections.agents}
        createLabel={translateApp("New agent")}
        onCreate={onCreateAgent}
        createDisabled={disabled || !active}
        onRefresh={() => void refreshAgents()}
        refreshing={refreshingAgents}
        refreshDisabled={!active}
        onPress={() =>
          setSections((value) => ({ ...value, agents: !value.agents }))
        }
      />,
    );
  let stickyIndex: number | undefined;
  if (sections.agents || mode === "picker") {
    rows.push(
      <View key="agent-status">
        {mode === "drawer" &&
        current?.kind === "agent" &&
        current.conversationId &&
        (current.agentId
          ? !allAgents.some((row) => row.id === current.agentId)
          : !chats.some((row) => row.id === current.conversationId)) ? (
          <ChatRow
            disabled={disabled}
            label={translateApp("Current chat")}
            selected
            onPress={() => onSelect(current)}
          />
        ) : null}
        {!isConnected ? (
          <DrawerMessage text={translateApp("Offline · showing saved chats")} />
        ) : null}
        {agents.isPending ? (
          <SkeletonRows
            rows={3}
            accessibilityLabel={translateApp("Loading agents")}
          />
        ) : null}
        {agents.isError ? (
          <DrawerMessage
            text={
              agents.data
                ? translateApp("Couldn’t refresh agents.")
                : translateApp("Couldn’t load agents.")
            }
            retry={() => void refreshAgents()}
            disabled={!active}
          />
        ) : null}
        {agents.isSuccess && !allAgents.length ? (
          <DrawerMessage text={translateApp("No agents yet.")} />
        ) : null}
      </View>,
    );
    let previousOwner: string | undefined;
    for (const row of previewAgents) {
      if (mode === "drawer") {
        const owner = sidebarAgentOwner(row);
        const counts = ownerCounts.get(owner.key)!;
        if (owner.key !== previousOwner)
          rows.push(
            <Text
              key={`owner:${owner.key}`}
              accessibilityRole="header"
              accessibilityLabel={translateApp(
                "{{owner}}, {{shown}} of {{total}} agents shown",
                {
                  owner: owner.label,
                  shown: counts.shown,
                  total: counts.total,
                },
              )}
              style={[styles.ownerHeading, { color: colors.textMuted }]}
            >
              {counts.shown === counts.total
                ? translateApp("{{label}} · {{count}}", {
                    label: owner.label,
                    count: counts.total,
                  })
                : translateApp("{{label}} · {{shown}} of {{count}}", {
                    label: owner.label,
                    shown: counts.shown,
                    count: counts.total,
                  })}
            </Text>,
          );
        previousOwner = owner.key;
      }
      const expanded = openId === row.id;
      if (expanded) stickyIndex = rows.length;
      rows.push(
        <View
          key={`agent:${row.id}`}
          style={{
            backgroundColor:
              mode === "drawer" ? colors.sidebarBase : colors.cardSoft,
          }}
          onLayout={
            expanded
              ? (event) => {
                  sticky.current.headerHeight = event.nativeEvent.layout.height;
                }
              : undefined
          }
        >
          <DisclosureRow
            disabled={disabled}
            agent={row}
            label={chatAgentLabel(row)}
            detail={
              mode === "drawer"
                ? groupedAgentSubtitle(row)
                : agentRowDetail(row)
            }
            accessibilityDetail={agentRowDetail(row)}
            expanded={expanded}
            selected={
              current?.kind === "agent" &&
              (current.agentId === row.id ||
                (!current.agentId &&
                  !current.conversationId &&
                  row.kind === "nyxbot"))
            }
            onPress={() => setExpandedAgent(expanded ? null : row.id)}
            kind="agent"
          />
        </View>,
      );
      if (!expanded) continue;
      rows.push(
        <View
          key={`threads:${row.id}`}
          accessibilityLabel={translateApp("Threads with {{label}}", {
            label: chatAgentLabel(row),
          })}
          style={[styles.children, { borderColor: colors.borderSoft }]}
          onLayout={(event) => {
            const { y, height } = event.nativeEvent.layout;
            sticky.current.bottom = y + height;
            setPastThreads(
              sticky.current.scrollY >=
                y + height - sticky.current.headerHeight,
            );
          }}
        >
          {historyStatus(true)}
          {currentRow}
          {previewChats.map(chatRow)}
          {threads.bots.map((bot) => {
            const key = `${row.id}:${bot.key}`;
            const holdsActive = bot.threads.some(
              (thread) => thread.id === selectedChatId,
            );
            const disclosure = channelDisclosures[key];
            const open =
              disclosure &&
              (disclosure.active === selectedChatId || !holdsActive)
                ? disclosure.open
                : holdsActive;
            const shown = disclosure?.shown ?? DRAWER_PREVIEW_LIMIT;
            const visibleThreads = bot.threads.filter(
              (thread, index) => index < shown || thread.id === selectedChatId,
            );
            const hidden = bot.threads.length - visibleThreads.length;
            const status = bot.threads.some(
              (thread) => thread.pending_acknowledgements,
            )
              ? translateApp("Needs you")
              : bot.threads.some((thread) => thread.active_turn)
                ? translateApp("Running")
                : undefined;
            const platform = channelPlatformName(bot.platform);
            const botUnread = bot.threads.reduce(
              (total, thread) =>
                total + (markers.conversations[thread.id] ?? 0),
              0,
            );
            const update = (nextOpen: boolean, nextShown = shown) =>
              setChannelDisclosures((value) => ({
                ...value,
                [key]: {
                  open: nextOpen,
                  active: selectedChatId,
                  shown: nextShown,
                },
              }));
            return (
              <View key={key}>
                <DisclosureRow
                  disabled={disabled}
                  label={bot.label}
                  detail={
                    bot.threads.length === 1
                      ? status
                        ? translateApp(
                            "{{platform}} · {{count}} chat · {{status}}",
                            {
                              platform: platform,
                              count: bot.threads.length,
                              status: status,
                            },
                          )
                        : translateApp("{{platform}} · {{count}} chat", {
                            platform: platform,
                            count: bot.threads.length,
                          })
                      : status
                        ? translateApp(
                            "{{platform}} · {{count}} chats · {{status}}",
                            {
                              platform: platform,
                              count: bot.threads.length,
                              status: status,
                            },
                          )
                        : translateApp("{{platform}} · {{count}} chats", {
                            platform: platform,
                            count: bot.threads.length,
                          })
                  }
                  expanded={Boolean(open)}
                  selected={holdsActive}
                  unread={botUnread}
                  onPress={() => update(!open)}
                  kind="group"
                />
                {open ? (
                  <View
                    style={[
                      styles.children,
                      { borderColor: colors.borderSoft },
                    ]}
                  >
                    {[
                      ...new Set(
                        visibleThreads.map(
                          (thread) =>
                            thread.channel?.parent_chat_id ||
                            thread.channel?.chat_id ||
                            thread.id,
                        ),
                      ),
                    ].map((parentId) => {
                      const children = visibleThreads.filter(
                        (thread) =>
                          (thread.channel?.parent_chat_id ||
                            thread.channel?.chat_id ||
                            thread.id) === parentId,
                      );
                      const followed = children.some(
                        (thread) => thread.channel?.thread_id,
                      );
                      return (
                        <View key={parentId}>
                          {followed ? (
                            <Text
                              style={[
                                styles.detail,
                                { color: colors.textMuted },
                              ]}
                            >
                              {children.find(
                                (thread) => thread.channel?.parent_title,
                              )?.channel?.parent_title ||
                                translateApp("Channel chat")}
                            </Text>
                          ) : null}
                          <View
                            style={
                              followed
                                ? [
                                    styles.subthreads,
                                    { borderColor: colors.border },
                                  ]
                                : undefined
                            }
                          >
                            {children.map(chatRow)}
                          </View>
                        </View>
                      );
                    })}
                    {hidden > 0 ? (
                      <MoreButton
                        disabled={disabled}
                        label={translateApp("Show {{min}} more", {
                          min: Math.min(hidden, DRAWER_PREVIEW_LIMIT * 4),
                        })}
                        onPress={() =>
                          update(true, shown + DRAWER_PREVIEW_LIMIT * 4)
                        }
                      />
                    ) : null}
                  </View>
                ) : null}
              </View>
            );
          })}
          {history.isSuccess && !chats.length && row.can_use !== false ? (
            <DrawerMessage
              text={
                row.status === "destroyed"
                  ? translateApp("No saved chats for this deleted agent.")
                  : translateApp("No chats yet.")
              }
            />
          ) : null}
          {row.can_use !== false &&
          (threads.own.length > previewChats.length || history.hasNextPage) ? (
            <MoreButton
              disabled={disabled}
              label={translateApp("View all chats")}
              onPress={() => setBrowser({ kind: "chats", fromAgents: false })}
            />
          ) : null}
          {startChat}
        </View>,
      );
    }
    // Recent only shortens the inline preview; the full browser lists everyone.
    if (roster.length > DRAWER_PREVIEW_LIMIT)
      rows.push(
        <MoreButton
          disabled={disabled}
          key="all-agents"
          label={translateApp("View all agents ({{count}})", {
            count: allAgents.length,
          })}
          onPress={() => setBrowser({ kind: "agents" })}
        />,
      );
  }
  if (mode === "drawer")
    rows.push(
      <SectionHeading
        disabled={disabled}
        key="crews"
        section="crews"
        label={translateApp("Crews")}
        count={allGroups.length}
        expanded={sections.groups}
        createLabel={translateApp("New crew")}
        onCreate={onCreateGroup}
        createDisabled={disabled || !active}
        onPress={() =>
          setSections((value) => ({ ...value, groups: !value.groups }))
        }
      />,
    );
  if (sections.groups && mode === "drawer")
    rows.push(
      <View key="crew-rows">
        {groups.isPending ? (
          <SkeletonRows
            rows={2}
            accessibilityLabel={translateApp("Loading crews")}
          />
        ) : null}
        {groups.isError ? (
          <DrawerMessage
            text={translateApp("Couldn’t load crews.")}
            retry={() => void groups.refetch()}
            disabled={!active}
          />
        ) : null}
        {groups.isSuccess && !allGroups.length ? (
          <DrawerMessage text={translateApp("No crews yet.")} />
        ) : null}
        {boundedDrawerPreview(
          allGroups,
          current?.kind === "group" ? current.groupId : undefined,
        ).map(groupRow)}
        {allGroups.length > DRAWER_PREVIEW_LIMIT ? (
          <MoreButton
            disabled={disabled}
            label={translateApp("View all crews ({{count}})", {
              count: allGroups.length,
            })}
            onPress={() => setBrowser({ kind: "groups" })}
          />
        ) : null}
      </View>,
    );
  return (
    <View style={styles.fill}>
      <ScrollView
        {...scrollProps}
        alwaysBounceVertical
        refreshControl={
          <RefreshControl
            accessibilityLabel={translateApp("Refresh agents")}
            enabled={active}
            refreshing={pullRefreshing}
            onRefresh={() => void refreshAgents()}
            tintColor={colors.primary}
            colors={[colors.primary]}
            progressBackgroundColor={colors.card}
          />
        }
        stickyHeaderIndices={
          stickyIndex !== undefined && !pastThreads ? [stickyIndex] : []
        }
        onScroll={(event) => {
          sticky.current.scrollY = event.nativeEvent.contentOffset.y;
          const past =
            sticky.current.scrollY >=
            sticky.current.bottom - sticky.current.headerHeight;
          if (past !== pastThreads) setPastThreads(past);
          scrollProps?.onScroll?.(event);
        }}
      >
        {rows}
        {scrollFooter}
      </ScrollView>
      {browser ? (
        <WorkspaceChatBrowser<BrowseRow>
          title={
            browser.kind === "agents"
              ? translateApp("All agents")
              : browser.kind === "groups"
                ? translateApp("All crews")
                : agent
                  ? chatAgentLabel(agent)
                  : translateApp("Agent chats")
          }
          titleIcon={
            browser.kind === "chats" && agent ? (
              <AgentAvatar agent={agent} size="md" />
            ) : undefined
          }
          scopeKey={
            browser.kind === "chats" ? `chats:${agent?.id}` : browser.kind
          }
          description={
            browser.kind === "agents"
              ? translateApp("Choose an agent to browse its chats.")
              : browser.kind === "groups"
                ? translateApp("Each crew has one shared conversation.")
                : !agent
                  ? translateApp(
                      "This agent is no longer available. Choose another agent to start a chat.",
                    )
                  : agent.can_use === false
                    ? translateApp(
                        "You don’t have permission to chat with this agent. Choose another agent to continue.",
                      )
                    : agent.status === "destroyed"
                      ? translateApp(
                          "This agent was deleted. You can still view its saved chats, or choose another agent to start a new chat.",
                        )
                      : history.hasNextPage
                        ? translateApp(
                            "Search loaded chats. Load older chats to search more history.",
                          )
                        : translateApp(
                            "Start a new chat or return to a saved conversation.",
                          )
          }
          items={
            browser.kind === "agents"
              ? allAgents.map((row) => ({ id: row.id, agent: row }))
              : browser.kind === "groups"
                ? allGroups.map((row) => ({ id: row.id, group: row }))
                : chats.map((row) => ({ id: row.id, chat: row }))
          }
          searchText={(row) =>
            "agent" in row
              ? `${chatAgentLabel(row.agent)} ${row.agent.name} ${row.agent.owner_name ?? ""}`
              : "group" in row
                ? `${row.group.name} ${row.group.owner?.name ?? ""}`
                : threadTitle(row.chat)
          }
          renderItem={(row) =>
            "agent" in row ? (
              <DisclosureRow
                disabled={disabled}
                agent={row.agent}
                label={chatAgentLabel(row.agent)}
                detail={agentRowDetail(row.agent)}
                expanded={false}
                selected={currentAgentId === row.id}
                onPress={() => {
                  setExpandedAgent(row.id);
                  setBrowser({ kind: "chats", fromAgents: true });
                }}
                kind="agent"
              />
            ) : "group" in row ? (
              groupRow(row.group)
            ) : (
              chatRow(row.chat)
            )
          }
          visible={!browserClosing}
          onRefresh={
            browser.kind === "agents" ? () => void refreshAgents() : undefined
          }
          refreshing={refreshingAgents}
          refreshDisabled={!active}
          refreshLabel={translateApp("Refresh agents")}
          loadingLabel={
            browser.kind === "agents"
              ? agents.isPending
                ? translateApp("Loading agents")
                : undefined
              : browser.kind === "groups"
                ? groups.isPending
                  ? translateApp("Loading crews")
                  : undefined
                : agent && agent.can_use !== false && history.isPending
                  ? translateApp("Loading chats")
                  : undefined
          }
          onClose={() => dismissBrowser()}
          onDismiss={() => finishBrowserDismissal(browserClosing)}
          primaryAction={
            browser.kind === "chats"
              ? canStartChat && agent
                ? {
                    disabled,
                    label: translateApp("New chat"),
                    accessibilityLabel: translateApp(
                      "New chat with {{label}}",
                      { label: chatAgentLabel(agent) },
                    ),
                    onPress: () => openChat(),
                  }
                : {
                    disabled,
                    label: translateApp("Choose another agent"),
                    accessibilityLabel: translateApp("Choose another agent"),
                    onPress: () => setBrowser({ kind: "agents" }),
                  }
              : undefined
          }
          onBack={
            browser.kind === "chats" && browser.fromAgents
              ? () => setBrowser({ kind: "agents" })
              : undefined
          }
          emptyText={
            browser.kind === "agents"
              ? translateApp("No agents available.")
              : browser.kind === "groups"
                ? translateApp("No crews yet.")
                : !agent
                  ? translateApp("This agent is no longer available.")
                  : agent.can_use === false
                    ? translateApp(
                        "Chat access isn’t available for this agent.",
                      )
                    : history.isError && !history.data
                      ? translateApp(
                          "Couldn’t load chats. You can retry below.",
                        )
                      : agent.status === "destroyed"
                        ? translateApp("No saved chats for this deleted agent.")
                        : translateApp(
                            "No chats yet. Start your first conversation above.",
                          )
          }
          footer={
            browser.kind === "agents" && agents.isError ? (
              <DrawerMessage
                text={
                  agents.data
                    ? translateApp("Couldn’t refresh agents.")
                    : translateApp("Couldn’t load agents.")
                }
                retry={() => void refreshAgents()}
                disabled={!active || refreshingAgents}
              />
            ) : browser.kind === "chats" ? (
              <>
                {historyStatus(false)}
                {currentRow}
                {history.hasNextPage ? (
                  <MoreButton
                    label={
                      history.isFetchingNextPage
                        ? translateApp("Loading older chats…")
                        : history.isFetchNextPageError
                          ? translateApp("Retry older chats")
                          : translateApp("Load older chats")
                    }
                    disabled={disabled || !active || history.isFetchingNextPage}
                    onPress={() => void history.fetchNextPage()}
                  />
                ) : null}
              </>
            ) : undefined
          }
        />
      ) : null}
    </View>
  );
}

function MoreButton({
  label,
  onPress,
  disabled,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={styles.more}
    >
      <Text
        style={[
          styles.moreText,
          { color: disabled ? colors.textMuted : colors.primaryOnTint },
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}
/** The compact count pill a sidebar row shows for its unread notifications. */
function UnreadCount({
  count,
  disabled = false,
}: {
  count: number | undefined;
  disabled?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  if (!count || count <= 0) return null;
  return (
    <Text
      testID="sidebar-unread-count"
      style={[
        styles.count,
        {
          color: disabled ? colors.textMuted : colors.textPrimary,
          backgroundColor: colors.navActive,
        },
      ]}
    >
      {count > 99 ? "99+" : String(count)}
    </Text>
  );
}

function GroupChatRow({
  group,
  selected,
  unread,
  onPress,
  disabled = false,
}: {
  group: Group;
  selected: boolean;
  unread?: number;
  onPress: () => void;
  disabled?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const detail = group.working_agent_ids.length
    ? group.owner?.type === "org"
      ? selected
        ? translateApp(
            "Agents working… · {{count}} agents · {{name}} · {{time}} · Current",
            {
              count: group.members.length,
              name: group.owner.name,
              time: formatRelativeTimeFromMs(
                Date.parse(group.last_message_at || group.created_at),
              ),
            },
          )
        : translateApp(
            "Agents working… · {{count}} agents · {{name}} · {{time}}",
            {
              count: group.members.length,
              name: group.owner.name,
              time: formatRelativeTimeFromMs(
                Date.parse(group.last_message_at || group.created_at),
              ),
            },
          )
      : selected
        ? translateApp(
            "Agents working… · {{count}} agents · {{time}} · Current",
            {
              count: group.members.length,
              time: formatRelativeTimeFromMs(
                Date.parse(group.last_message_at || group.created_at),
              ),
            },
          )
        : translateApp("Agents working… · {{count}} agents · {{time}}", {
            count: group.members.length,
            time: formatRelativeTimeFromMs(
              Date.parse(group.last_message_at || group.created_at),
            ),
          })
    : group.owner?.type === "org"
      ? selected
        ? translateApp(
            "{{count}} messages · {{agents}} agents · {{name}} · {{time}} · Current",
            {
              count: group.message_count,
              agents: group.members.length,
              name: group.owner.name,
              time: formatRelativeTimeFromMs(
                Date.parse(group.last_message_at || group.created_at),
              ),
            },
          )
        : translateApp(
            "{{count}} messages · {{agents}} agents · {{name}} · {{time}}",
            {
              count: group.message_count,
              agents: group.members.length,
              name: group.owner.name,
              time: formatRelativeTimeFromMs(
                Date.parse(group.last_message_at || group.created_at),
              ),
            },
          )
      : selected
        ? translateApp(
            "{{count}} messages · {{agents}} agents · {{time}} · Current",
            {
              count: group.message_count,
              agents: group.members.length,
              time: formatRelativeTimeFromMs(
                Date.parse(group.last_message_at || group.created_at),
              ),
            },
          )
        : translateApp("{{count}} messages · {{agents}} agents · {{time}}", {
            count: group.message_count,
            agents: group.members.length,
            time: formatRelativeTimeFromMs(
              Date.parse(group.last_message_at || group.created_at),
            ),
          });
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={workspaceCrewLabel(
        group.name,
        group.working_agent_ids.length,
        selected,
        unread,
      )}
      accessibilityState={{ selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.row,
        {
          backgroundColor:
            pressed || selected ? colors.navActive : "transparent",
        },
      ]}
    >
      <AgentAvatarStack
        agents={group.members}
        surface={selected ? colors.navActive : colors.sidebarBase}
      />
      <View style={styles.label}>
        <Text
          numberOfLines={1}
          style={[
            styles.name,
            { color: disabled ? colors.textMuted : colors.textPrimary },
          ]}
        >
          {group.name}
        </Text>
        <Text
          numberOfLines={1}
          style={[styles.detail, { color: colors.textMuted }]}
        >
          {detail}
        </Text>
      </View>
      <UnreadCount count={unread} disabled={disabled} />
      <ChevronRight size={16} color={colors.textMuted} />
    </Pressable>
  );
}

function DisclosureRow({
  agent,
  label,
  detail,
  accessibilityDetail,
  expanded,
  selected,
  unread,
  onPress,
  disabled = false,
  kind,
}: {
  label: string;
  detail?: string;
  accessibilityDetail?: string;
  expanded: boolean;
  selected: boolean;
  /** Unread notifications across the chats this row folds. */
  unread?: number;
  onPress: () => void;
  disabled?: boolean;
  kind: "agent" | "group";
  agent?: AssistantAgent;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={workspaceDisclosureLabel(
        label,
        expanded,
        accessibilityDetail ?? detail,
        unread,
      )}
      accessibilityState={{ expanded, selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.row,
        {
          // An expanded agent's current thread carries the highlight instead.
          backgroundColor:
            pressed || (selected && !expanded)
              ? colors.navActive
              : "transparent",
        },
      ]}
    >
      {agent ? (
        <AgentAvatar agent={agent} size="lg" />
      ) : (
        <Users size={18} color={colors.textSecondary} />
      )}
      <View style={styles.label}>
        <Text
          numberOfLines={1}
          style={[
            styles.name,
            { color: disabled ? colors.textMuted : colors.textPrimary },
          ]}
        >
          {label}
        </Text>
        {agent?.channels.length ? (
          <View style={styles.channelBadges}>
            {[
              ...new Set(
                agent.channels.map((channel) =>
                  channelPlatformName(channel.platform),
                ),
              ),
            ].map((platform) => (
              <Text
                key={platform}
                style={[
                  styles.channelBadge,
                  {
                    color: disabled ? colors.textMuted : colors.textSecondary,
                    backgroundColor: colors.navActive,
                  },
                ]}
              >
                {platform}
              </Text>
            ))}
          </View>
        ) : null}
        {detail ? (
          <Text
            numberOfLines={1}
            style={[styles.detail, { color: colors.textMuted }]}
          >
            {detail}
          </Text>
        ) : null}
      </View>
      <UnreadCount count={unread} disabled={disabled} />
      <Chevron size={16} color={colors.textMuted} />
    </Pressable>
  );
}

function ChatRow({
  agent,
  label,
  detail,
  selected,
  unread,
  onPress,
  disabled = false,
  accessibilityLabel,
  status,
  create,
}: {
  agent?: AssistantAgent;
  label: string;
  detail?: string;
  selected: boolean;
  unread?: number;
  onPress: () => void;
  disabled?: boolean;
  accessibilityLabel?: string;
  status?: string;
  create?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={
        accessibilityLabel ||
        translateApp(
          label === translateApp("Current chat")
            ? "Open current chat"
            : "Open {{label}}",
          { label },
        )
      }
      accessibilityState={{ selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.chat,
        {
          backgroundColor:
            pressed || selected ? colors.navActive : "transparent",
        },
      ]}
    >
      {agent ? <AgentAvatar agent={agent} size="lg" /> : null}
      {create ? <Plus size={14} color={colors.textMuted} /> : null}
      <View style={styles.label}>
        <View style={styles.chatLine}>
          <Text
            numberOfLines={2}
            style={[
              styles.chatTitle,
              {
                color: disabled
                  ? colors.textMuted
                  : selected || unread
                    ? colors.textPrimary
                    : colors.textSecondary,
                fontFamily: selected || unread ? fonts.bodySemi : fonts.body,
              },
            ]}
          >
            {label}
          </Text>
          {detail ? (
            <Text
              numberOfLines={1}
              style={[styles.chatTime, { color: colors.textMuted }]}
            >
              {detail}
            </Text>
          ) : null}
        </View>
        {status ? (
          <Text
            style={[
              styles.detail,
              {
                color: disabled
                  ? colors.textMuted
                  : status === "Needs you"
                    ? colors.warning
                    : colors.primaryOnTint,
              },
            ]}
          >
            {translateApp(status)}
          </Text>
        ) : null}
      </View>
      <UnreadCount count={unread} disabled={disabled} />
    </Pressable>
  );
}

function DrawerMessage({
  text,
  retry,
  disabled,
}: {
  text: string;
  retry?: () => void;
  disabled?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <View style={styles.message}>
      <Text style={[styles.messageText, { color: colors.textMuted }]}>
        {text}
      </Text>
      {retry ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={translateApp("Retry: {{text}}", { text: text })}
          disabled={disabled}
          onPress={retry}
          style={styles.retry}
        >
          <Text style={[styles.moreText, { color: colors.primaryOnTint }]}>
            {translateApp("Retry")}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  subthreads: { marginLeft: 8, paddingLeft: 5, borderLeftWidth: 1 },
  ownerHeading: {
    fontFamily: fonts.bodySemi,
    fontSize: 12,
    paddingHorizontal: 12,
    paddingTop: 12,
    paddingBottom: 4,
  },
  channelBadges: { flexDirection: "row", flexWrap: "wrap", gap: 4 },
  channelBadge: {
    fontFamily: fonts.bodyRegular,
    fontSize: 10,
    borderRadius: 4,
    paddingHorizontal: 5,
    paddingVertical: 2,
  },
  sections: { gap: 4, marginTop: 12 },
  count: {
    fontFamily: fonts.bodySemi,
    fontSize: 11,
    borderRadius: 10,
    paddingHorizontal: 7,
    paddingVertical: 2,
    overflow: "hidden",
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 48,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 8,
  },
  label: { flex: 1, gap: 3, minWidth: 0 },
  name: { fontFamily: fonts.bodySemi, fontSize: 13 },
  detail: { fontFamily: fonts.bodyRegular, fontSize: 10, lineHeight: 15 },
  children: {
    marginLeft: 21,
    paddingLeft: 8,
    borderLeftWidth: 1,
    marginBottom: 4,
  },
  chat: {
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
    paddingHorizontal: 10,
    paddingVertical: 10,
    minHeight: 44,
    borderRadius: 8,
  },
  chatLine: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "baseline",
    columnGap: 8,
  },
  chatTitle: { flexGrow: 1, flexShrink: 1, fontSize: 12, lineHeight: 18 },
  chatTime: {
    flexShrink: 0,
    fontFamily: fonts.bodyRegular,
    fontSize: 10,
    lineHeight: 15,
  },
  message: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  messageText: {
    flex: 1,
    fontFamily: fonts.bodyRegular,
    fontSize: 11,
    lineHeight: 17,
  },
  retry: { minHeight: 44, justifyContent: "center", paddingHorizontal: 4 },
  more: { minHeight: 44, justifyContent: "center", paddingHorizontal: 10 },
  moreText: { fontFamily: fonts.bodySemi, fontSize: 11 },
});
