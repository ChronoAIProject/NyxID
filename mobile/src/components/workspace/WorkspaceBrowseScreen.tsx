import { searchFieldContent, searchFieldInput } from "./searchFieldStyles";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
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
  useWindowDimensions,
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
import { fittingPlaceholder, SEARCH_FIELD } from "./textFit";
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
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { agentsApi } from "../../lib/api/agentsApi";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { SkeletonRows } from "../Skeleton";
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
import { useRailScrollIndicator } from "./workspaceFloatingInset";

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
  useAppLanguage();
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
  placeholders,
  autoFocus = false,
}: {
  value: string;
  onChange: (value: string) => void;
  /**
   * Complete phrases, most descriptive first; the first is also the field's
   * accessibility label. The placeholder is the first that fits the field at
   * the current text size, else the last (e.g. "Search"), so it never clips.
   */
  placeholders: readonly string[];
  autoFocus?: boolean;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const { fontScale } = useWindowDimensions();
  const [textWidth, setTextWidth] = useState<number>();
  const placeholder = fittingPlaceholder(
    placeholders,
    textWidth,
    SEARCH_FIELD.fontSize * fontScale,
  );
  return (
    <View
      style={[
        styles.search,
        { backgroundColor: colors.cardSoft, borderColor: colors.border },
      ]}
    >
      <Search
        size={SEARCH_FIELD.iconSize}
        color={colors.textMuted}
        accessible={false}
      />
      <TextInput
        accessibilityLabel={placeholders[0]}
        placeholder={placeholder}
        placeholderTextColor={colors.textMuted}
        value={value}
        onChangeText={onChange}
        onLayout={(event) =>
          setTextWidth(
            event.nativeEvent.layout.width - SEARCH_FIELD.textPaddingRight,
          )
        }
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
          accessibilityLabel={translateApp("Clear search")}
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
  const railIndicator = useRailScrollIndicator();
  useAppLanguage();
  const { colors } = useTheme();
  const [query, setQuery] = useState("");
  const [agentId, setAgentId] = useState(route.params?.agentId);
  useEffect(() => {
    setAgentId(route.params?.agentId);
    setQuery("");
  }, [route.params?.agentId]);
  const data = useBrowseData(agentId);
  const pull = usePullRefresh(data.refresh);
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
    { id: undefined, name: translateApp("All agents & crews") },
    ...data.agents
      .filter((a) => a.status !== "destroyed")
      .map((agent) => ({
        id: agent.id,
        name: agent.display_name || agent.name,
      })),
  ];
  return (
    <WorkspacePage
      title={history ? translateApp("Chat history") : translateApp("Recents")}
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      <View style={styles.toolbar}>
        {history && (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={translateApp("New chat")}
            style={styles.more}
            onPress={() =>
              openChat({ kind: "agent", agentId: route.params?.agentId })
            }
          >
            <Text style={{ color: colors.textPrimary }}>
              {translateApp("New chat")}
            </Text>
          </Pressable>
        )}
        <BrowseSearchField
          placeholders={[translateApp("Search chats"), translateApp("Search")]}
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
              accessibilityLabel={translateApp("Filter chats: {{name}}", {
                name: filter.name,
              })}
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
            {history ? translateApp("Chats") : translateApp("Recent chats")}
          </Text>
        }
        refreshControl={
          <RefreshControl
            refreshing={pull.refreshing}
            onRefresh={pull.onRefresh}
            tintColor={colors.textMuted}
          />
        }
        ListEmptyComponent={
          data.loading ? (
            <SkeletonRows
              rows={5}
              accessibilityLabel={translateApp("Loading recent chats")}
              style={styles.skeleton}
            />
          ) : (
            <Text
              accessibilityLiveRegion="polite"
              style={[styles.note, { color: colors.textMuted }]}
            >
              {data.error
                ? translateApp("Couldn’t load recent chats. Pull to refresh.")
                : !data.online
                  ? translateApp("Offline · showing saved recent chats.")
                  : query.trim()
                    ? translateApp("No matching chats in loaded history.")
                    : translateApp("No recent chats.")}
            </Text>
          )
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
                {data.loadingMore ? translateApp("Loading older chats…")
                  : translateApp("Load older chats")}
              </Text>
            </Pressable>
          ) : null
        }
        {...railIndicator}
      />
    </WorkspacePage>
  );
}

export function WorkspaceSearchScreen({
  navigation,
}: ContentScreenProps<"WorkspaceSearch">) {
  const railIndicator = useRailScrollIndicator();
  const appLanguage = useAppLanguage().language;
  const { colors, setPreference } = useTheme();
  const [query, setQuery] = useState("");
  const data = useBrowseData();
  const pull = usePullRefresh(data.refresh);
  const sections = useMemo(
    () =>
      workspaceSearchSections(
        query,
        data.conversations,
        data.agents,
        data.crews,
      ),
    [query, data.conversations, data.agents, data.crews, appLanguage],
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
      title={translateApp("Search")}
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      <View style={styles.toolbar}>
        <BrowseSearchField
          placeholders={[
            translateApp("Search chats, agents, settings"),
            translateApp("Search"),
          ]}
          value={query}
          onChange={setQuery}
          autoFocus
        />
      </View>
      {data.online && !data.error && !data.agentsError ? (
        data.loading && (
          <SkeletonRows
            rows={3}
            accessibilityLabel={translateApp("Loading chats, agents and crews")}
            style={styles.searchSkeleton}
          />
        )
      ) : (
        <Text
          accessibilityLiveRegion="polite"
          style={[styles.notice, { color: colors.textMuted }]}
        >
          {!data.online
            ? translateApp("Offline · searching saved data.")
            : translateApp("Some results couldn’t load. Pull to refresh.")}
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
            refreshing={pull.refreshing}
            onRefresh={pull.onRefresh}
            tintColor={colors.textMuted}
          />
        }
        ListEmptyComponent={
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.note, { color: colors.textMuted }]}
          >
            {translateApp("No results.")}
          </Text>
        }
        {...railIndicator}
      />
    </WorkspacePage>
  );
}

const styles = StyleSheet.create({
  page: { padding: 0, gap: 0 },
  fill: { flex: 1 },
  list: { paddingHorizontal: 16, paddingTop: 8, paddingBottom: 40 },
  toolbar: { paddingHorizontal: 16, paddingTop: 12, gap: 10 },
  search: { ...searchFieldContent, borderRadius: 12, borderWidth: SEARCH_FIELD.border },
  input: searchFieldInput,
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
  skeleton: { paddingTop: 4 },
  searchSkeleton: { paddingHorizontal: 16, paddingTop: 10 },
  notice: {
    paddingHorizontal: 28,
    paddingTop: 10,
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
  },
  more: { minHeight: 44, padding: 12, justifyContent: "center" },
});
