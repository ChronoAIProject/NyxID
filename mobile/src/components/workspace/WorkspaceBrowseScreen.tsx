import { useCallback, useEffect, useMemo, useState } from "react";
import {
  FlatList,
  Keyboard,
  Pressable,
  RefreshControl,
  ScrollView,
  SectionList,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import {
  Search,
  X,
  CalendarClock,
  Monitor,
  Plug,
  ShieldCheck,
  UserPlus,
  Users,
  MessagesSquare,
  ScanQrCode,
  Terminal,
  Settings,
  Bell,
  SunMoon,
  LockKeyhole,
  CircleDollarSign,
  FileText,
  Moon,
  Sun,
  MessageSquare,
} from "lucide-react-native";
import { AgentAvatar, AgentAvatarStack } from "../AgentAvatar";
import { sidebarSectionTitle } from "./workspaceSidebarStyles";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import type { ContentScreenProps } from "../../app/navigationTypes";
import {
  openWorkspacePageAction,
  openWorkspaceSheetAction,
} from "../../app/navigationHierarchy";
import {
  workspaceChatNavigationAction,
  type WorkspaceChatTarget,
} from "../../app/workspaceChatNavigation";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { agentsApi } from "../../lib/api/agentsApi";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { useWorkspaceRecents } from "./useWorkspaceRecents";
import { WorkspaceRecentRow } from "./WorkspaceRecents";
import { WorkspacePage } from "./WorkspaceUI";
import type { WorkspaceRecent } from "./workspaceSidebarModel";
import {
  filterWorkspaceRecents,
  workspaceSearchSections,
  type WorkspaceSearchEntry,
  type WorkspaceSearchTarget,
} from "./workspaceSearchModel";

const recentKey = (item: WorkspaceRecent) => item.key;
const searchKey = (item: WorkspaceSearchEntry) => item.key;
const destinationIcons: Record<string, typeof Search> = {
  "page:automations": CalendarClock,
  "page:machines": Monitor,
  "page:logins": Monitor,
  "page:plugins": Plug,
  "page:activity": ShieldCheck,
  "page:agents": Users,
  "page:create-agent": UserPlus,
  "page:recents": MessagesSquare,
  "quick:scanner": ScanQrCode,
  "quick:terminal": Terminal,
  "settings:profile": Settings,
  "settings:security": ShieldCheck,
  "settings:display": SunMoon,
  "settings:privacy": LockKeyhole,
  "settings:notifications": Bell,
  "settings:usage": CircleDollarSign,
  "settings:terms": FileText,
  "settings:privacy-policy": LockKeyhole,
  "theme:light": Sun,
  "theme:dark": Moon,
  "theme:system": Monitor,
};

function useBrowseData(agentId?: string) {
  const focused = useIsFocused();
  const data = useWorkspaceRecents(focused, agentId);
  const epoch = getAuthEpoch();
  // Reuse the sidebar's index/cache; there is no server search endpoint.
  const agents = useQuery({
    queryKey: ["workspace", "agents", "drawer", epoch],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: data.active,
    staleTime: 5000,
    retry: false,
  });
  return {
    ...data,
    agents: agents.data?.agents ?? [],
    agentsError: agents.isError,
    loading: data.loading || (data.active && agents.isPending),
    refresh: async () => {
      if (data.active && epoch === getAuthEpoch())
        await Promise.allSettled([data.refresh(), agents.refetch()]);
    },
    epoch,
    focused,
  };
}

function BrowseSearchField({
  value,
  onChange,
  label,
  autoFocus = false,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  autoFocus?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View
      style={[
        styles.search,
        { backgroundColor: colors.cardSoft, borderColor: colors.border },
      ]}
    >
      <Search size={18} color={colors.textMuted} accessible={false} />
      <TextInput
        accessibilityLabel={label}
        placeholder={label}
        placeholderTextColor={colors.textMuted}
        value={value}
        onChangeText={onChange}
        autoFocus={autoFocus}
        autoCorrect={false}
        autoCapitalize="none"
        returnKeyType="search"
        clearButtonMode="never"
        onSubmitEditing={Keyboard.dismiss}
        style={[styles.input, { color: colors.textPrimary }]}
      />
      {!!value && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Clear search"
          style={styles.clear}
          onPress={() => onChange("")}
        >
          <X size={18} color={colors.textMuted} />
        </Pressable>
      )}
    </View>
  );
}

export function WorkspaceRecentsScreen({
  navigation,
  route,
}: ContentScreenProps<"WorkspaceRecents" | "AssistantHistory">) {
  const { colors } = useTheme();
  const [query, setQuery] = useState("");
  const [agentId, setAgentId] = useState(route.params?.agentId);
  useEffect(() => {
    setAgentId(route.params?.agentId);
    setQuery("");
  }, [route.params?.agentId]);
  const data = useBrowseData(agentId);
  const history = route.name === "AssistantHistory";
  const rows = useMemo(
    () => filterWorkspaceRecents(data.rows, query, agentId),
    [data.rows, query, agentId],
  );
  const openChat = useCallback(
    (target: WorkspaceChatTarget) => {
      if (getAuthEpoch() !== data.epoch || !data.focused) return;
      Keyboard.dismiss();
      const action = workspaceChatNavigationAction(
        navigation.getParent()?.getState(),
        target,
      );
      if (action) navigation.dispatch(action);
    },
    [navigation, data.epoch, data.focused],
  );
  const renderItem = useCallback(
    ({ item }: { item: WorkspaceRecent }) => (
      <WorkspaceRecentRow item={item} onSelect={openChat} />
    ),
    [openChat],
  );
  const filters = [
    { id: undefined, name: "All agents & crews" },
    ...data.agents
      .filter((a) => a.status !== "destroyed")
      .map((agent) => ({
        id: agent.id,
        name: agent.display_name || agent.name,
      })),
  ];
  return (
    <WorkspacePage
      title={history ? "Chat history" : "Recents"}
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      <View style={styles.toolbar}>
        {history && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="New chat"
            style={styles.more}
            onPress={() =>
              openChat({ kind: "agent", agentId: route.params?.agentId })
            }
          >
            <Text style={{ color: colors.textPrimary }}>New chat</Text>
          </Pressable>
        )}
        <BrowseSearchField
          label="Search chats"
          value={query}
          onChange={setQuery}
        />
        <ScrollView
          horizontal
          keyboardShouldPersistTaps="handled"
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={styles.filters}
        >
          {filters.map((filter) => (
            <Pressable
              key={filter.id ?? "all"}
              accessibilityRole="button"
              accessibilityLabel={`Filter chats: ${filter.name}`}
              accessibilityState={{ selected: agentId === filter.id }}
              onPress={() => setAgentId(filter.id)}
              style={[
                styles.filter,
                {
                  backgroundColor:
                    agentId === filter.id ? colors.navActive : colors.cardSoft,
                  borderColor: colors.border,
                },
              ]}
            >
              <Text style={[styles.filterLabel, { color: colors.textPrimary }]}>
                {filter.name}
              </Text>
            </Pressable>
          ))}
        </ScrollView>
      </View>
      <FlatList
        style={styles.fill}
        data={rows}
        keyExtractor={recentKey}
        renderItem={renderItem}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="on-drag"
        contentContainerStyle={styles.list}
        ListHeaderComponent={
          <Text
            accessibilityRole="header"
            style={[styles.heading, { color: colors.textPrimary }]}
          >
            {history ? "Chats" : "Recent chats"}
          </Text>
        }
        refreshControl={
          <RefreshControl
            refreshing={data.refreshing}
            onRefresh={() => void data.refresh()}
            tintColor={colors.textMuted}
          />
        }
        ListEmptyComponent={
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.note, { color: colors.textMuted }]}
          >
            {data.loading
              ? "Loading recent chats…"
              : data.error
                ? "Couldn’t load recent chats. Pull to refresh."
                : !data.online
                  ? "Offline · showing saved recent chats."
                  : query.trim()
                    ? "No matching chats in loaded history."
                    : "No recent chats."}
          </Text>
        }
        ListFooterComponent={
          data.hasNextPage ? (
            <Pressable
              accessibilityRole="button"
              accessibilityState={{
                disabled: !data.active || data.loadingMore,
              }}
              disabled={!data.active || data.loadingMore}
              style={styles.more}
              onPress={data.loadMore}
            >
              <Text style={{ color: colors.textSecondary }}>
                {data.loadingMore ? "Loading older chats…" : "Load older chats"}
              </Text>
            </Pressable>
          ) : null
        }
      />
    </WorkspacePage>
  );
}

export function WorkspaceSearchScreen({
  navigation,
}: ContentScreenProps<"WorkspaceSearch">) {
  const { colors, setPreference } = useTheme();
  const [query, setQuery] = useState("");
  const data = useBrowseData();
  const sections = useMemo(
    () =>
      workspaceSearchSections(
        query,
        data.conversations,
        data.agents,
        data.crews,
      ),
    [query, data.conversations, data.agents, data.crews],
  );
  const select = useCallback(
    (target: WorkspaceSearchTarget) => {
      if (getAuthEpoch() !== data.epoch || !data.focused) return;
      Keyboard.dismiss();
      if (target.kind === "chat") {
        const action = workspaceChatNavigationAction(
          navigation.getParent()?.getState(),
          target.target,
        );
        if (action) navigation.dispatch(action);
      } else if (target.kind === "page")
        navigation.dispatch(
          openWorkspacePageAction(target.name, target.params),
        );
      else if (target.kind === "settings")
        navigation.dispatch(
          openWorkspaceSheetAction(target.name, target.params),
        );
      else setPreference(target.preference);
    },
    [navigation, data.epoch, data.focused, setPreference],
  );
  const renderItem = useCallback(
    ({ item }: { item: WorkspaceSearchEntry }) => {
      const target = item.target;
      const chat = target.kind === "chat" ? target.target : undefined;
      const agent =
        chat?.kind === "agent" && !chat.conversationId
          ? data.agents.find((row) => row.id === chat.agentId)
          : undefined;
      const crew =
        chat?.kind === "group"
          ? data.crews.find((row) => row.id === chat.groupId)
          : undefined;
      const Icon = destinationIcons[item.key] ?? MessageSquare;
      return (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`${item.label}${item.hint ? `, ${item.hint}` : ""}`}
          style={({ pressed }) => [
            styles.result,
            { backgroundColor: pressed ? colors.navActive : "transparent" },
          ]}
          onPress={() => select(item.target)}
        >
          {agent ? (
            <AgentAvatar agent={agent} size="sm" />
          ) : crew ? (
            <AgentAvatarStack
              agents={crew.members}
              size="sm"
              surface={colors.bg}
            />
          ) : (
            <Icon size={20} color={colors.textSecondary} accessible={false} />
          )}
          <Text
            numberOfLines={2}
            style={[styles.resultTitle, { color: colors.textPrimary }]}
          >
            {item.label}
          </Text>
          {!!item.hint && (
            <Text
              numberOfLines={2}
              style={[styles.hint, { color: colors.textMuted }]}
            >
              {item.hint}
            </Text>
          )}
        </Pressable>
      );
    },
    [colors, select, data.agents, data.crews],
  );
  return (
    <WorkspacePage
      title="Search"
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      <View style={styles.toolbar}>
        <BrowseSearchField
          label="Search chats, agents, crews, pages and settings"
          value={query}
          onChange={setQuery}
          autoFocus
        />
      </View>
      {(data.loading || data.error || data.agentsError || !data.online) && (
        <Text
          accessibilityLiveRegion="polite"
          style={[styles.notice, { color: colors.textMuted }]}
        >
          {!data.online
            ? "Offline · searching saved data."
            : data.error || data.agentsError
              ? "Some results couldn’t load. Pull to refresh."
              : "Loading chats, agents and crews…"}
        </Text>
      )}
      <SectionList
        style={styles.fill}
        sections={sections}
        keyExtractor={searchKey}
        renderItem={renderItem}
        renderSectionHeader={({ section }) => (
          <Text
            accessibilityRole="header"
            style={[styles.heading, { color: colors.textPrimary }]}
          >
            {section.title}
          </Text>
        )}
        stickySectionHeadersEnabled={false}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="on-drag"
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl
            refreshing={data.refreshing}
            onRefresh={() => void data.refresh()}
            tintColor={colors.textMuted}
          />
        }
        ListEmptyComponent={
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.note, { color: colors.textMuted }]}
          >
            No results.
          </Text>
        }
      />
    </WorkspacePage>
  );
}

const styles = StyleSheet.create({
  page: { padding: 0, gap: 0 },
  fill: { flex: 1 },
  list: { paddingHorizontal: 16, paddingTop: 8, paddingBottom: 40 },
  toolbar: { paddingHorizontal: 16, paddingTop: 12, gap: 10 },
  search: {
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 12,
    borderRadius: 12,
    borderWidth: 1,
    gap: 8,
    minHeight: 48,
  },
  input: {
    flex: 1,
    minWidth: 0,
    minHeight: 48,
    fontFamily: fonts.bodyRegular,
    fontSize: 15,
    paddingVertical: 10,
  },
  clear: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  filters: { gap: 8, paddingBottom: 4 },
  filter: {
    borderRadius: 22,
    minHeight: 44,
    paddingHorizontal: 14,
    justifyContent: "center",
    borderWidth: 1,
  },
  filterLabel: { fontFamily: fonts.bodyRegular, fontSize: 13 },
  heading: {
    ...sidebarSectionTitle,
    paddingHorizontal: 12,
    paddingTop: 22,
    paddingBottom: 10,
  },
  result: {
    minHeight: 48,
    paddingVertical: 12,
    paddingHorizontal: 12,
    flexDirection: "row",
    gap: 12,
    alignItems: "center",
    borderRadius: 8,
  },
  resultTitle: { flex: 1, fontFamily: fonts.bodyRegular, fontSize: 15 },
  hint: { maxWidth: "40%", fontFamily: fonts.bodyRegular, fontSize: 12 },
  note: { padding: 12, fontFamily: fonts.bodyRegular, fontSize: 14 },
  notice: {
    paddingHorizontal: 28,
    paddingTop: 10,
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
  },
  more: { minHeight: 44, padding: 12, justifyContent: "center" },
});
