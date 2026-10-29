import {
  sidebarAgentOwner,
  sidebarOrderedAgents,
} from "./workspaceSidebarModel";
import { AgentAvatar, AgentAvatarStack } from "../AgentAvatar";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  AppState,
  Pressable,
  ScrollView,
  type ScrollViewProps,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import {
  Check,
  ChevronDown,
  ChevronRight,
  MessageSquare,
  Plus,
  Users,
} from "lucide-react-native";
import type { WorkspaceChatTarget } from "../../app/workspaceChatNavigation";
import type { AssistantAgent } from "../../features/agents/agentContracts";
import type { AssistantConversation } from "../../features/assistant/assistantTypes";
import type { Group } from "../../features/groups/groupContracts";
import { chatAgentLabel } from "../../features/assistant/assistantAgentContext";
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
import { WorkspaceChatBrowser } from "./WorkspaceChatBrowser";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
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

type Props = {
  mode?: "drawer" | "picker";
  scrollFooter?: ReactNode;
  scrollHeader?: ReactNode;
  scrollProps?: ScrollViewProps;
  visible: boolean;
  current?: WorkspaceChatTarget;
  onSelect: (target: WorkspaceChatTarget) => void;
  onCreateAgent: () => void;
  onCreateGroup: () => void;
};
type BrowseRow =
  | { id: string; agent: AssistantAgent }
  | { id: string; group: Group }
  | { id: string; chat: AssistantConversation };
type Browser =
  | { kind: "agents" | "groups" }
  | { kind: "chats"; fromAgents: boolean };

export function WorkspaceChatList({
  mode = "drawer",
  scrollFooter,
  scrollHeader,
  scrollProps,
  visible,
  current,
  onSelect,
  onCreateAgent,
  onCreateGroup,
}: Props) {
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
    return () => subscription.remove();
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
    refetchInterval: livePollInterval(5000, live),
    retry: false,
  });
  const groups = useQuery({
    queryKey: [...groupKeys(epoch), "list"],
    queryFn: ({ signal }) => groupsApi.list(signal),
    enabled: active && mode === "drawer",
    refetchInterval: livePollInterval(5000, live),
    retry: false,
  });
  const allAgents = (
    mode === "drawer" ? sidebarOrderedAgents : orderedDrawerAgents
  )(agents.data?.agents ?? []).filter(
    (agent) =>
      mode !== "picker" ||
      (agent.status !== "destroyed" && agent.can_use === true),
  );
  const allGroups = [...(groups.data ?? [])].sort((a, b) =>
    (b.last_message_at || b.created_at).localeCompare(
      a.last_message_at || a.created_at,
    ),
  );
  const currentAgentId =
    current?.kind === "agent" ? current.agentId : undefined;
  const openId = expandedAgent !== undefined ? expandedAgent : currentAgentId;
  const agent = allAgents.find((row) => row.id === openId);
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
      ? "Past agent · chat history"
      : row.can_use === false
        ? "View only"
        : row.owner_kind === "org"
          ? row.owner_name || "Shared agent"
          : row.kind === "nyxbot"
            ? "Personal · Your personal agent"
            : "Personal";
  const agentRowDetail = (row: AssistantAgent) =>
    [
      row.pending_acknowledgements
        ? "Needs you"
        : row.status === "running"
          ? "Running"
          : undefined,
      agentDetail(row),
    ]
      .filter(Boolean)
      .join(" · ");
  const groupedAgentSubtitle = (row: AssistantAgent) =>
    [
      row.pending_acknowledgements
        ? "Needs you"
        : row.status === "running"
          ? "Running"
          : undefined,
      row.status === "destroyed"
        ? "Past agent · chat history"
        : row.can_use === false
          ? "View only"
          : row.kind === "nyxbot"
            ? "Your personal agent"
            : row.description,
    ]
      .filter(Boolean)
      .join(" · ");
  const chatRow = (row: AssistantConversation) => (
    <ChatRow
      key={row.id}
      label={threadTitle(row)}
      detail={formatRelativeTimeFromMs(Date.parse(row.last_message_at))}
      status={threadStatus(row)}
      selected={selectedChatId === row.id}
      onPress={() => openChat(row.id)}
      accessibilityLabel={`Open chat ${threadTitle(row)} with ${agent ? chatAgentLabel(agent) : "agent"}${threadStatus(row) ? `, ${threadStatus(row)}` : ""}${selectedChatId === row.id ? ", current thread" : ""}`}
    />
  );
  const groupRow = (row: Group) => (
    <GroupChatRow
      key={row.id}
      group={row}
      selected={current?.kind === "group" && current.groupId === row.id}
      onPress={() => openGroup(row)}
    />
  );
  const historyStatus = (
    <>
      {agent?.can_use === false ? (
        <DrawerMessage text="View only · chat access isn’t available for this agent." />
      ) : (
        <>
          {history.isPending ? (
            <DrawerMessage text="Loading chats…" loading={active} />
          ) : null}
          {history.isError ? (
            <DrawerMessage
              text={
                history.data
                  ? "Couldn’t refresh chats."
                  : "Couldn’t load chats."
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
        label={`New chat with ${chatAgentLabel(agent)}`}
        create
        selected={
          current?.kind === "agent" &&
          current.agentId === agent.id &&
          !current.conversationId
        }
        onPress={() => openChat()}
        accessibilityLabel={`New chat with ${chatAgentLabel(agent)}`}
      />
    ) : null;
  const currentRow = missingCurrent ? (
    <ChatRow
      label="Current chat"
      selected
      onPress={() => openChat(selectedChatId)}
    />
  ) : null;
  const rows: ReactNode[] = [<View key="workspace">{scrollHeader}</View>];
  if (mode === "drawer")
    rows.push(
      <SectionHeading
        key="agents"
        label="Agents"
        count={allAgents.length}
        expanded={sections.agents}
        createLabel="New agent"
        onCreate={onCreateAgent}
        createDisabled={!active}
        onPress={() =>
          setSections((value) => ({ ...value, agents: !value.agents }))
        }
      />,
    );
  let stickyIndex: number | undefined;
  if (sections.agents || mode === "picker") {
    rows.push(
      <View key="agent-status">
        {current?.kind === "agent" &&
        current.conversationId &&
        !current.agentId &&
        !chats.some((row) => row.id === current.conversationId) ? (
          <ChatRow
            label="Current chat"
            selected
            onPress={() => onSelect(current)}
          />
        ) : null}
        {!isConnected ? (
          <DrawerMessage text="Offline · showing saved chats" />
        ) : null}
        {agents.isPending ? (
          <DrawerMessage text="Loading agents…" loading={active} />
        ) : null}
        {agents.isError ? (
          <DrawerMessage
            text="Couldn’t load agents."
            retry={() => void agents.refetch()}
            disabled={!active}
          />
        ) : null}
        {agents.isSuccess && !allAgents.length ? (
          <DrawerMessage text="No agents yet." />
        ) : null}
      </View>,
    );
    let previousOwner: string | undefined;
    for (const row of boundedDrawerPreview(
      allAgents,
      openId || currentAgentId,
    )) {
      if (mode === "drawer") {
        const owner = sidebarAgentOwner(row);
        if (owner.key !== previousOwner)
          rows.push(
            <Text
              key={`owner:${owner.key}`}
              accessibilityRole="header"
              style={[styles.ownerHeading, { color: colors.textMuted }]}
            >
              {owner.label}
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
          accessibilityLabel={`Threads with ${chatAgentLabel(row)}`}
          style={[styles.children, { borderColor: colors.border }]}
          onLayout={(event) => {
            const { y, height } = event.nativeEvent.layout;
            sticky.current.bottom = y + height;
            setPastThreads(
              sticky.current.scrollY >=
                y + height - sticky.current.headerHeight,
            );
          }}
        >
          {historyStatus}
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
              ? "Needs you"
              : bot.threads.some((thread) => thread.active_turn)
                ? "Running"
                : undefined;
            const platform = channelPlatformName(bot.platform);
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
                  label={bot.label}
                  detail={`${platform} · ${bot.threads.length} ${bot.threads.length === 1 ? "chat" : "chats"}${status ? ` · ${status}` : ""}`}
                  expanded={Boolean(open)}
                  selected={holdsActive}
                  onPress={() => update(!open)}
                  kind="group"
                />
                {open ? (
                  <View
                    style={[styles.children, { borderColor: colors.border }]}
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
                              )?.channel?.parent_title || "Channel chat"}
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
                        label={`Show ${Math.min(hidden, DRAWER_PREVIEW_LIMIT * 4)} more`}
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
                  ? "No saved chats for this deleted agent."
                  : "No chats yet."
              }
            />
          ) : null}
          {row.can_use !== false &&
          (threads.own.length > previewChats.length || history.hasNextPage) ? (
            <MoreButton
              label="View all chats"
              onPress={() => setBrowser({ kind: "chats", fromAgents: false })}
            />
          ) : null}
          {startChat}
        </View>,
      );
    }
    if (allAgents.length > DRAWER_PREVIEW_LIMIT)
      rows.push(
        <MoreButton
          key="all-agents"
          label={`View all agents (${allAgents.length})`}
          onPress={() => setBrowser({ kind: "agents" })}
        />,
      );
  }
  if (mode === "drawer")
    rows.push(
      <SectionHeading
        key="crews"
        label="Crews"
        count={allGroups.length}
        expanded={sections.groups}
        createLabel="New crew"
        onCreate={onCreateGroup}
        createDisabled={!active}
        onPress={() =>
          setSections((value) => ({ ...value, groups: !value.groups }))
        }
      />,
    );
  if (sections.groups && mode === "drawer")
    rows.push(
      <View key="crew-rows">
        {groups.isPending ? (
          <DrawerMessage text="Loading crews..." loading={active} />
        ) : null}
        {groups.isError ? (
          <DrawerMessage
            text="Couldn’t load crews."
            retry={() => void groups.refetch()}
            disabled={!active}
          />
        ) : null}
        {groups.isSuccess && !allGroups.length ? (
          <DrawerMessage text="No crews yet." />
        ) : null}
        {boundedDrawerPreview(
          allGroups,
          current?.kind === "group" ? current.groupId : undefined,
        ).map(groupRow)}
        {allGroups.length > DRAWER_PREVIEW_LIMIT ? (
          <MoreButton
            label={`View all crews (${allGroups.length})`}
            onPress={() => setBrowser({ kind: "groups" })}
          />
        ) : null}
      </View>,
    );
  return (
    <View style={styles.fill}>
      <ScrollView
        {...scrollProps}
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
              ? "All agents"
              : browser.kind === "groups"
                ? "All crews"
                : agent
                  ? chatAgentLabel(agent)
                  : "Agent chats"
          }
          scopeKey={
            browser.kind === "chats" ? `chats:${agent?.id}` : browser.kind
          }
          description={
            browser.kind === "agents"
              ? "Choose an agent to browse its chats."
              : browser.kind === "groups"
                ? "Each crew has one shared conversation."
                : !agent
                  ? "This agent is no longer available. Choose another agent to start a chat."
                  : agent.can_use === false
                    ? "You don’t have permission to chat with this agent. Choose another agent to continue."
                    : agent.status === "destroyed"
                      ? "This agent was deleted. You can still view its saved chats, or choose another agent to start a new chat."
                      : history.hasNextPage
                        ? "Search loaded chats. Load older chats to search more history."
                        : "Start a new chat or return to a saved conversation."
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
          onClose={() => dismissBrowser()}
          onDismiss={() => finishBrowserDismissal(browserClosing)}
          primaryAction={
            browser.kind === "chats"
              ? canStartChat && agent
                ? {
                    label: "New chat",
                    accessibilityLabel: `New chat with ${chatAgentLabel(agent)}`,
                    onPress: () => openChat(),
                  }
                : {
                    label: "Choose another agent",
                    accessibilityLabel: "Choose another agent",
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
              ? "No agents available."
              : browser.kind === "groups"
                ? "No crews yet."
                : !agent
                  ? "This agent is no longer available."
                  : agent.can_use === false
                    ? "Chat access isn’t available for this agent."
                    : history.isPending
                      ? "Loading chats…"
                      : history.isError && !history.data
                        ? "Couldn’t load chats. You can retry below."
                        : agent.status === "destroyed"
                          ? "No saved chats for this deleted agent."
                          : "No chats yet. Start your first conversation above."
          }
          footer={
            browser.kind === "chats" ? (
              <>
                {historyStatus}
                {currentRow}
                {history.hasNextPage ? (
                  <MoreButton
                    label={
                      history.isFetchingNextPage
                        ? "Loading older chats…"
                        : history.isFetchNextPageError
                          ? "Retry older chats"
                          : "Load older chats"
                    }
                    disabled={!active || history.isFetchingNextPage}
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

function SectionHeading({
  label,
  count,
  expanded,
  onPress,
  createLabel,
  onCreate,
  createDisabled,
}: {
  label: string;
  count: number;
  expanded: boolean;
  onPress: () => void;
  createLabel: string;
  onCreate: () => void;
  createDisabled: boolean;
}) {
  const { colors } = useTheme();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return (
    <View style={styles.sectionHeading}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${expanded ? "Collapse" : "Expand"} ${label.toLowerCase()}`}
        accessibilityState={{ expanded }}
        onPress={onPress}
        style={styles.sectionRow}
      >
        <Text
          accessibilityRole="header"
          style={[styles.section, { color: colors.textPrimary }]}
        >
          {label}
        </Text>
        <Text style={[styles.sectionCount, { color: colors.textMuted }]}>
          {count}
        </Text>
        <Chevron size={16} color={colors.textMuted} />
      </Pressable>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={createLabel}
        accessibilityState={{ disabled: createDisabled }}
        disabled={createDisabled}
        onPress={onCreate}
        style={[styles.createButton, { opacity: createDisabled ? 0.4 : 1 }]}
      >
        <Plus size={18} color={colors.textSecondary} />
      </Pressable>
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
          { color: colors.primaryOnTint, opacity: disabled ? 0.5 : 1 },
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}
function GroupChatRow({
  group,
  selected,
  onPress,
}: {
  group: Group;
  selected: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open crew ${group.name}${group.working_agent_ids.length ? `, ${group.working_agent_ids.length} running` : ""}${selected ? ", current crew" : ""}`}
      accessibilityState={{ selected }}
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
          style={[styles.name, { color: colors.textPrimary }]}
        >
          {group.name}
        </Text>
        <Text
          numberOfLines={1}
          style={[styles.detail, { color: colors.textMuted }]}
        >
          {group.working_agent_ids.length
            ? "Agents working…"
            : `${group.message_count} messages`}{" "}
          · {group.members.length} agents
          {group.owner?.type === "org" ? ` · ${group.owner.name}` : ""}
          {` · ${formatRelativeTimeFromMs(Date.parse(group.last_message_at || group.created_at))}`}
          {selected ? " · Current" : ""}
        </Text>
      </View>
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
  onPress,
  kind,
}: {
  label: string;
  detail?: string;
  accessibilityDetail?: string;
  expanded: boolean;
  selected: boolean;
  onPress: () => void;
  kind: "agent" | "group";
  agent?: AssistantAgent;
}) {
  const { colors } = useTheme();
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${expanded ? "Collapse" : "Expand"} chats for ${label}${(accessibilityDetail ?? detail) ? `, ${accessibilityDetail ?? detail}` : ""}`}
      accessibilityState={{ expanded, selected }}
      onPress={onPress}
      style={({ pressed }) => [
        styles.row,
        {
          backgroundColor:
            pressed || selected ? colors.navActive : "transparent",
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
          style={[styles.name, { color: colors.textPrimary }]}
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
                    color: colors.textSecondary,
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
      {selected ? <Check size={16} color={colors.primaryOnTint} /> : null}
      <Chevron size={16} color={colors.textMuted} />
    </Pressable>
  );
}

function ChatRow({
  label,
  detail,
  selected,
  onPress,
  accessibilityLabel,
  status,
  create,
}: {
  label: string;
  detail?: string;
  selected: boolean;
  onPress: () => void;
  accessibilityLabel?: string;
  status?: string;
  create?: boolean;
}) {
  const { colors } = useTheme();
  const Icon = create ? Plus : MessageSquare;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel || `Open ${label.toLowerCase()}`}
      accessibilityState={{ selected }}
      onPress={onPress}
      style={({ pressed }) => [
        styles.chat,
        {
          backgroundColor:
            pressed || selected ? colors.navActive : "transparent",
        },
      ]}
    >
      <Icon size={14} color={selected ? colors.primary : colors.textMuted} />
      <View style={styles.label}>
        <Text
          numberOfLines={2}
          style={[
            styles.chatTitle,
            { color: selected ? colors.primaryOnTint : colors.textSecondary },
          ]}
        >
          {label}
        </Text>
        {selected || status ? (
          <Text
            style={[
              styles.detail,
              {
                color:
                  status === "Needs you"
                    ? colors.warning
                    : colors.primaryOnTint,
              },
            ]}
          >
            {[selected ? "Current" : undefined, status]
              .filter(Boolean)
              .join(" · ")}
          </Text>
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
    </Pressable>
  );
}

function DrawerMessage({
  text,
  loading,
  retry,
  disabled,
}: {
  text: string;
  loading?: boolean;
  retry?: () => void;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View style={styles.message}>
      {loading ? (
        <ActivityIndicator size="small" color={colors.primary} />
      ) : null}
      <Text style={[styles.messageText, { color: colors.textMuted }]}>
        {text}
      </Text>
      {retry ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Retry: ${text}`}
          disabled={disabled}
          onPress={retry}
          style={styles.retry}
        >
          <Text style={[styles.moreText, { color: colors.primaryOnTint }]}>
            Retry
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
  sectionHeading: { flexDirection: "row", alignItems: "center" },
  createButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 8,
  },
  sections: { gap: 4, marginTop: 12 },
  sectionRow: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
    paddingRight: 10,
    gap: 8,
  },
  sectionCount: { fontFamily: fonts.bodyRegular, fontSize: 11 },
  section: {
    ...sidebarSectionTitle,
    padding: 12,
    flex: 1,
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
  chatTitle: { fontFamily: fonts.body, fontSize: 12, lineHeight: 18 },
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
