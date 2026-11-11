import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  SectionList,
  Keyboard,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import { RefreshCw, Search, X } from "lucide-react-native";
import type { WorkspaceNavigationProp } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { ServiceIcon } from "../../components/ServiceIcon";
import { SkeletonRows } from "../../components/Skeleton";
import { SegmentControl } from "../../components/SegmentControl";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { servicesApi } from "../../lib/api/servicesApi";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { connectionOwner } from "../services/serviceContracts";
import {
  deriveConnectorItems,
  matchesPluginQuery,
  type ConnectorCardItem,
} from "./pluginConnectors";
import { PluginSkills } from "./PluginSkills";

type Tab = "connectors" | "skills";
const ROOT = ["workspace", "services"] as const;
const errorText = (error: unknown) =>
  error instanceof Error ? error.message : "Could not load plugins. Try again.";

export function PluginsScreen({
  agentId,
  initialTab = "connectors",
  connectorId,
}: { agentId?: string; initialTab?: Tab; connectorId?: string } = {}) {
  const { colors } = useTheme();
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [tab, setTab] = useState<Tab>(initialTab);
  const [query, setQuery] = useState("");
  const manageId = connectorId;
  const connections = useQuery({
    queryKey: [...ROOT, "plugins"],
    queryFn: ({ signal }) => servicesApi.pluginConnections(signal),
    enabled: focused && foreground,
    retry: false,
  });
  const catalog = useQuery({
    queryKey: [...ROOT, "catalog"],
    queryFn: ({ signal }) => servicesApi.catalog(signal),
    enabled: focused && foreground,
    staleTime: 60_000,
    retry: false,
  });
  const items = deriveConnectorItems(
    connections.data ?? [],
    catalog.data ?? [],
  );
  const manageItem = items.added.find((item) => item.id === manageId);
  const added = items.added.filter((item) => matchesPluginQuery(item, query));
  // A failed connections read must not offer duplicates as "Available".
  const available =
    connections.data && !connections.isError
      ? items.available.filter((item) => matchesPluginQuery(item, query))
      : [];
  const loadError = connections.error ?? catalog.error;
  const catalogKnown = !!connections.data && !connections.isError;
  const loading = connections.isPending || catalog.isPending;
  // Only the user's own pull or Refresh tap shows a spinner; background
  // refocus and foreground refetches never do.
  const pull = usePullRefresh(() =>
    Promise.all([connections.refetch(), catalog.refetch()]),
  );
  const refreshing = pull.refreshing;
  const refresh = pull.onRefresh;

  useEffect(() => {
    setTab(initialTab);
  }, [initialTab, agentId]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state === "active" && focused) {
        void connections.refetch();
        void catalog.refetch();
      }
    });
    return () => subscription.remove();
  }, [focused, connections.refetch, catalog.refetch]);
  useEffect(() => {
    navigation.setOptions({ title: manageItem?.name ?? "Plugins" });
  }, [navigation, manageItem?.name]);
  useWorkspaceBack({
    active: false,
    defaultRole: manageId ? "back" : "menu",
    label: "Back to plugins",
    onBack: navigation.goBack,
  });

  function openConnector(item: ConnectorCardItem) {
    Keyboard.dismiss();
    if (item.added) {
      if (item.manageKeyIds?.length === 1)
        navigation.push("ServiceSetup", { connectionId: item.manageKeyIds[0] });
      else navigation.push("PluginConnections", { connectorId: item.id });
    } else if (item.connectSlug)
      navigation.push("ServiceSetup", { catalogSlug: item.connectSlug });
  }

  if (manageId)
    return (
      <WorkspacePage
        title={manageItem?.name ?? "Connections"}
        showHeading={false}
        contentStyle={styles.grouped}
        refreshing={refreshing}
        onRefresh={refresh}
      >
        {loadError && <Notice message={errorText(loadError)} tone="error" />}
        {manageItem ? (
          <SettingsGroup
            title={manageItem.name}
            footer="Choose a connection to manage."
          >
            {manageItem.connections.map((connection) => (
              <SettingsRow
                key={connection.id}
                leading={
                  <ServiceIcon
                    iconUrl={
                      connection.icon_url ||
                      connection.catalog_icon_url ||
                      manageItem.iconUrl
                    }
                    slug={
                      connection.catalog_service_slug ??
                      manageItem.iconSlug ??
                      connection.slug
                    }
                    size="md"
                    color={colors.textSecondary}
                  />
                }
                label={connection.label}
                detail={`${connectionOwner(connection)} · ${connection.credential_type.replaceAll("_", " ")}`}
                value={
                  connection.is_active
                    ? sentence(connection.status.replaceAll("_", " "))
                    : "Disabled"
                }
                accessibilityLabel={`Manage ${connection.label}`}
                onPress={() =>
                  navigation.push("ServiceSetup", {
                    connectionId: connection.id,
                  })
                }
              />
            ))}
          </SettingsGroup>
        ) : (
          <SettingsGroup footer="These connections are no longer in your list. Refresh Plugins to see the current catalog.">
            <SettingsRow
              label="Refresh plugins"
              tone="action"
              disabled={refreshing}
              onPress={refresh}
            />
          </SettingsGroup>
        )}
      </WorkspacePage>
    );

  const searching = !!query.trim();
  const addedCount = connections.data ? items.added.length : undefined;
  const discoverCount =
    catalogKnown && catalog.data ? items.available.length : undefined;
  // One list: what you have added, then the catalog to add from. A single
  // search filters both, so there is no second selector to switch between.
  const sections = [
    {
      key: "added",
      title: `Added${addedCount === undefined ? "" : ` · ${addedCount}`}`,
      data: added,
      empty: connections.isError
        ? "Your connections could not be loaded."
        : searching
          ? "No added connectors match this search."
          : "No connectors added yet. Add one from Discover below.",
    },
    {
      key: "discover",
      title: `Discover${discoverCount === undefined ? "" : ` · ${discoverCount}`}`,
      data: available,
      empty:
        catalog.isError || !catalogKnown
          ? "The available catalog could not be confirmed. Retry loading plugins."
          : searching
            ? "No available connectors match this search."
            : "All available connectors are already added.",
    },
  ];
  return (
    <WorkspacePage title="Plugins" showHeading={false} scroll={false}>
      <SegmentControl
        segments={[{ label: "Connectors" }, { label: "Skills" }]}
        activeIndex={tab === "connectors" ? 0 : 1}
        onPress={(index) => {
          Keyboard.dismiss();
          setTab(index === 0 ? "connectors" : "skills");
        }}
        style={styles.tabs}
      />
      {tab === "skills" ? (
        <ScrollView
          key="skills"
          style={styles.body}
          contentContainerStyle={styles.skillContent}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="on-drag"
        >
          <PluginSkills agentId={agentId} />
        </ScrollView>
      ) : (
        <View style={styles.connectorBody}>
          <View style={styles.searchRow}>
            <PluginSearch value={query} onChange={setQuery} />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Refresh plugins"
              accessibilityState={{ disabled: refreshing }}
              disabled={refreshing}
              onPress={refresh}
              style={styles.refresh}
            >
              {refreshing ? (
                <ActivityIndicator size="small" color={colors.primary} />
              ) : (
                <RefreshCw color={colors.textSecondary} size={18} />
              )}
            </Pressable>
          </View>
          <SectionList
            style={styles.body}
            contentContainerStyle={styles.listContent}
            accessibilityLabel="Connectors"
            sections={loading ? [] : sections}
            keyExtractor={(item) => item.id}
            stickySectionHeadersEnabled={false}
            renderSectionHeader={({ section }) => (
              <Text
                accessibilityRole="header"
                style={[styles.sectionTitle, { color: colors.textMuted }]}
              >
                {section.title.toUpperCase()}
              </Text>
            )}
            renderSectionFooter={({ section }) =>
              section.data.length ? (
                <View style={styles.sectionGap} />
              ) : (
                <Text style={[styles.empty, { color: colors.textMuted }]}>
                  {section.empty}
                </Text>
              )
            }
            renderItem={({ item, index, section }) => (
              <View
                style={[
                  { backgroundColor: colors.groupedFill },
                  index === 0 && styles.firstRow,
                  index === section.data.length - 1 && styles.lastRow,
                ]}
              >
                <ConnectorRow item={item} onPress={() => openConnector(item)} />
              </View>
            )}
            ItemSeparatorComponent={RowSeparator}
            keyboardShouldPersistTaps="handled"
            keyboardDismissMode="on-drag"
            initialNumToRender={14}
            refreshing={refreshing}
            onRefresh={refresh}
            ListHeaderComponent={
              loadError ? (
                <View style={styles.listHeader}>
                  <Notice
                    message={`Some plugins could not be loaded. ${errorText(loadError)}`}
                    tone="error"
                  />
                  <SettingsGroup>
                    <SettingsRow
                      label="Retry loading plugins"
                      tone="action"
                      onPress={refresh}
                    />
                  </SettingsGroup>
                </View>
              ) : null
            }
            ListEmptyComponent={
              loading ? (
                <SkeletonRows
                  rows={6}
                  title
                  accessibilityLabel="Loading plugins"
                />
              ) : null
            }
          />
        </View>
      )}
    </WorkspacePage>
  );
}

export function AgentSkillsScreen({ agentId }: { agentId: string }) {
  return (
    <WorkspacePage
      title="Agent skills"
      showHeading={false}
      contentStyle={styles.grouped}
    >
      <PluginSkills agentId={agentId} />
    </WorkspacePage>
  );
}

function PluginSearch({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { colors } = useTheme();
  return (
    <View style={[styles.search, { backgroundColor: colors.segmentTrack }]}>
      <Search size={17} color={colors.textMuted} accessible={false} />
      <TextInput
        accessibilityLabel="Search connectors"
        value={value}
        onChangeText={onChange}
        placeholder="Search connectors"
        placeholderTextColor={colors.textMuted}
        selectionColor={colors.primary}
        autoCapitalize="none"
        autoCorrect={false}
        returnKeyType="search"
        onSubmitEditing={Keyboard.dismiss}
        style={[styles.input, { color: colors.textPrimary }]}
      />
      {value.length > 0 && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Clear connector search"
          onPress={() => onChange("")}
          hitSlop={8}
          style={styles.clearSearch}
        >
          <X size={16} color={colors.textMuted} />
        </Pressable>
      )}
    </View>
  );
}

function RowSeparator() {
  const { colors } = useTheme();
  return (
    <View style={{ backgroundColor: colors.groupedFill }}>
      <View
        style={[styles.separator, { backgroundColor: colors.borderSoft }]}
      />
    </View>
  );
}

const sentence = (value: string) =>
  value.charAt(0).toUpperCase() + value.slice(1);

// Discover rows tag how the service authenticates, beside its name.
const authLabel = (meta: string) =>
  meta === "oauth" ? "OAuth" : meta === "api key" ? "API key" : sentence(meta);

/** One connector: brand icon, name, summary and its status or "Connect". */
function ConnectorRow({
  item,
  onPress,
}: {
  item: ConnectorCardItem;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <SettingsRow
      leading={
        <ServiceIcon
          iconUrl={item.iconUrl}
          slug={item.iconSlug ?? "custom"}
          size="md"
          color={colors.textSecondary}
        />
      }
      label={item.name}
      detail={item.added ? item.summary : item.description}
      detailLines={2}
      // Added rows tag their state, catalog rows their sign-in method; both
      // sit beside the name so neither squeezes the description.
      badge={item.added ? item.status : authLabel(item.meta)}
      badgeTone={
        !item.added
          ? "default"
          : item.statusTone === "error"
            ? "danger"
            : item.statusTone === "warning"
              ? "warning"
              : "default"
      }
      accessibilityLabel={`${item.added ? "Manage" : "Connect"} ${item.name}, ${item.added ? item.status : `${authLabel(item.meta)}, available to add`}`}
      accessibilityHint={item.added ? item.summary : item.description}
      onPress={onPress}
    />
  );
}

const styles = StyleSheet.create({
  grouped: {
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.sm,
    paddingBottom: spacing.huge * 2,
    gap: spacing.huge,
  },
  body: { flex: 1 },
  skillContent: { paddingBottom: spacing.huge },
  connectorBody: { flex: 1, minHeight: 0, gap: spacing.lg },
  searchRow: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  search: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    minHeight: 40,
    borderRadius: radius.md + 2,
    paddingLeft: spacing.md,
    gap: spacing.sm - 2,
  },
  input: {
    flex: 1,
    minWidth: 0,
    minHeight: 40,
    paddingVertical: spacing.sm,
    paddingRight: spacing.sm,
    ...typeScale.label,
  },
  tabs: { marginBottom: 0 },
  listContent: { paddingBottom: spacing.huge, flexGrow: 1 },
  firstRow: {
    borderTopLeftRadius: radius.lg,
    borderTopRightRadius: radius.lg,
    overflow: "hidden",
  },
  lastRow: {
    borderBottomLeftRadius: radius.lg,
    borderBottomRightRadius: radius.lg,
    overflow: "hidden",
  },
  // Past the 30pt icon slot, as SettingsGroup insets its own separators.
  separator: {
    height: StyleSheet.hairlineWidth,
    marginLeft: spacing.xxl + 30 + spacing.lg,
  },
  listHeader: { gap: spacing.lg, paddingBottom: spacing.lg },
  sectionTitle: {
    ...typeScale.small,
    letterSpacing: 0.6,
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.sm,
  },
  sectionGap: { height: spacing.huge },
  empty: {
    ...typeScale.small,
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.huge,
  },
  clearSearch: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  refresh: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
});
