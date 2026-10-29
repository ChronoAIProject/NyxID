import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  FlatList,
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
import { ArrowRight, RefreshCw, Search, X } from "lucide-react-native";
import type { WorkspaceNavigationProp } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { ServiceIcon } from "../../components/ServiceIcon";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { servicesApi } from "../../lib/api/servicesApi";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { connectionOwner } from "../services/serviceContracts";
import {
  deriveConnectorItems,
  matchesPluginQuery,
  type ConnectorCardItem,
} from "./pluginConnectors";
import { PluginSkills } from "./PluginSkills";

type Tab = "connectors" | "skills";
type ConnectorTab = "added" | "discover";
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
  const [connectorTab, setConnectorTab] = useState<ConnectorTab>("added");
  const [queries, setQueries] = useState({ added: "", discover: "" });
  const query = queries[connectorTab];
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
  const refreshing = connections.isFetching || catalog.isFetching;

  function refresh() {
    void connections.refetch();
    void catalog.refetch();
  }

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
      >
        <Label muted>Choose a connection to manage.</Label>
        {loadError && <Notice message={errorText(loadError)} tone="error" />}
        {manageItem?.connections.map((connection) => (
          <Pressable
            key={connection.id}
            accessibilityRole="button"
            accessibilityLabel={`Manage ${connection.label}`}
            onPress={() =>
              navigation.push("ServiceSetup", { connectionId: connection.id })
            }
          >
            <Card>
              <View style={styles.row}>
                <ServiceIcon
                  iconUrl={
                    connection.icon_url ||
                    connection.catalog_icon_url ||
                    manageItem?.iconUrl
                  }
                  slug={
                    connection.catalog_service_slug ??
                    manageItem?.iconSlug ??
                    connection.slug
                  }
                  size="lg"
                  color={colors.textSecondary}
                />
                <View style={styles.flex}>
                  <Label style={styles.name}>{connection.label}</Label>
                  <Label muted style={styles.description}>
                    {connectionOwner(connection)} ·{" "}
                    {connection.credential_type.replaceAll("_", " ")}
                  </Label>
                  <Label muted style={styles.description}>
                    {connection.is_active
                      ? connection.status.replaceAll("_", " ")
                      : "Disabled"}
                  </Label>
                </View>
                <ArrowRight size={16} color={colors.textMuted} />
              </View>
            </Card>
          </Pressable>
        ))}
        {!manageItem && (
          <Notice message="These connections are no longer in your list. Refresh Plugins to see the current catalog." />
        )}
      </WorkspacePage>
    );

  return (
    <WorkspacePage title="Plugins" showHeading={false} scroll={false}>
      <View
        accessibilityRole="tablist"
        style={[
          styles.tabs,
          { backgroundColor: colors.cardSoft, borderColor: colors.border },
        ]}
      >
        {(["connectors", "skills"] as const).map((value) => (
          <Pressable
            key={value}
            accessibilityRole="tab"
            accessibilityState={{ selected: tab === value }}
            onPress={() => {
              Keyboard.dismiss();
              setTab(value);
            }}
            style={[
              styles.tab,
              {
                backgroundColor:
                  tab === value ? colors.navActive : "transparent",
              },
            ]}
          >
            <Text
              style={[
                styles.tabLabel,
                {
                  color: tab === value ? colors.textPrimary : colors.textMuted,
                },
              ]}
            >
              {value === "connectors" ? "Connectors" : "Skills"}
            </Text>
          </Pressable>
        ))}
      </View>
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
          <View
            accessibilityRole="tablist"
            style={[styles.scopes, { backgroundColor: colors.cardSoft }]}
          >
            {(["added", "discover"] as const).map((value) => {
              const count =
                value === "added"
                  ? connections.data && items.added.length
                  : catalogKnown && catalog.data && items.available.length;
              const label = value === "added" ? "Added" : "Discover";
              return (
                <Pressable
                  key={value}
                  accessibilityRole="tab"
                  accessibilityLabel={`${label} connectors${typeof count === "number" ? `, ${count}` : ""}`}
                  accessibilityState={{ selected: connectorTab === value }}
                  onPress={() => {
                    Keyboard.dismiss();
                    setConnectorTab(value);
                  }}
                  style={[
                    styles.tab,
                    {
                      backgroundColor:
                        connectorTab === value ? colors.card : "transparent",
                    },
                  ]}
                >
                  <Text
                    style={[
                      styles.tabLabel,
                      {
                        color:
                          connectorTab === value
                            ? colors.textPrimary
                            : colors.textSecondary,
                      },
                    ]}
                  >
                    {label}
                    {typeof count === "number" ? ` · ${count}` : ""}
                  </Text>
                </Pressable>
              );
            })}
          </View>
          <View style={styles.row}>
            <PluginSearch
              value={query}
              discover={connectorTab === "discover"}
              onChange={(value) =>
                setQueries((previous) => ({
                  ...previous,
                  [connectorTab]: value,
                }))
              }
            />
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
          <View
            style={[
              styles.list,
              { borderColor: colors.border, backgroundColor: colors.card },
            ]}
          >
            <FlatList
              key={connectorTab}
              accessibilityLabel={
                connectorTab === "added"
                  ? "Added connectors"
                  : "Discover connectors"
              }
              data={connectorTab === "added" ? added : available}
              keyExtractor={(item) => item.id}
              renderItem={({ item }) => (
                <ConnectorCard
                  item={item}
                  onPress={() => openConnector(item)}
                />
              )}
              keyboardShouldPersistTaps="handled"
              keyboardDismissMode="on-drag"
              initialNumToRender={12}
              refreshing={refreshing}
              onRefresh={refresh}
              ListHeaderComponent={
                loadError ? (
                  <View style={styles.listNotice}>
                    <Notice
                      message={`Some plugins could not be loaded. ${errorText(loadError)}`}
                      tone="error"
                    />
                    <ActionButton
                      label="Retry loading plugins"
                      variant="secondary"
                      onPress={refresh}
                    />
                  </View>
                ) : null
              }
              ListEmptyComponent={
                <View style={styles.empty}>
                  {loading ? (
                    <ActivityIndicator
                      color={colors.primary}
                      accessibilityLabel="Loading plugins"
                    />
                  ) : (
                    <>
                      <Notice
                        message={
                          connectorTab === "added"
                            ? connections.isError
                              ? "Your connections could not be loaded."
                              : query.trim()
                                ? "No added connectors match this search."
                                : "No connectors added yet. Discover a service to connect."
                            : catalog.isError || !catalogKnown
                              ? "The available catalog could not be confirmed. Retry loading plugins."
                              : query.trim()
                                ? "No available connectors match this search."
                                : "All available connectors are already added."
                        }
                      />
                      {connectorTab === "added" && !connections.isError && (
                        <ActionButton
                          label="Discover connectors"
                          variant="secondary"
                          onPress={() => {
                            Keyboard.dismiss();
                            setConnectorTab("discover");
                          }}
                        />
                      )}
                    </>
                  )}
                </View>
              }
            />
          </View>
        </View>
      )}
    </WorkspacePage>
  );
}

export function AgentSkillsScreen({ agentId }: { agentId: string }) {
  return (
    <WorkspacePage title="Agent skills" showHeading={false}>
      <PluginSkills agentId={agentId} />
    </WorkspacePage>
  );
}

function PluginSearch({
  value,
  onChange,
  discover,
}: {
  value: string;
  onChange: (value: string) => void;
  discover: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View
      style={[
        styles.search,
        { borderColor: colors.border, backgroundColor: colors.card },
      ]}
    >
      <Search size={18} color={colors.textMuted} accessible={false} />
      <TextInput
        accessibilityLabel={
          discover ? "Search connector catalog" : "Search added connectors"
        }
        value={value}
        onChangeText={onChange}
        placeholder={
          discover ? "Search connector catalog…" : "Search added connectors…"
        }
        placeholderTextColor={colors.textMuted}
        selectionColor={colors.primary}
        autoCapitalize="none"
        autoCorrect={false}
        returnKeyType="search"
        onSubmitEditing={Keyboard.dismiss}
        style={[
          styles.input,
          {
            color: colors.textPrimary,
          },
        ]}
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

function ConnectorCard({
  item,
  onPress,
}: {
  item: ConnectorCardItem;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  const statusColor = item.added
    ? item.statusTone === "success"
      ? colors.success
      : item.statusTone === "error"
        ? colors.danger
        : item.statusTone === "warning"
          ? colors.warning
          : colors.textSecondary
    : colors.primary;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${item.added ? "Manage" : "Connect"} ${item.name}, ${item.added ? item.status : "Available to add"}`}
      accessibilityHint={item.added ? item.summary : item.description}
      onPress={onPress}
      style={[styles.connectorRow, { borderBottomColor: colors.border }]}
    >
      <ServiceIcon
        iconUrl={item.iconUrl}
        slug={item.iconSlug ?? "custom"}
        size="lg"
        color={colors.textSecondary}
      />
      <View style={styles.flex}>
        <Text
          style={[styles.name, { color: colors.textPrimary }]}
          numberOfLines={2}
        >
          {item.name}
        </Text>
        <Label muted style={styles.description} numberOfLines={2}>
          {item.added ? item.summary : item.description}
        </Label>
        <View style={styles.metadata}>
          <Label style={[styles.badge, { color: statusColor }]}>
            {item.added ? `● ${item.status}` : "Connect"}
          </Label>
          {!item.added && (
            <Label muted style={styles.externalLabel}>
              · {item.meta}
            </Label>
          )}
        </View>
      </View>
      <ArrowRight size={16} color={colors.textMuted} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, minWidth: 0, gap: 3 },
  body: { flex: 1 },
  skillContent: { paddingBottom: 8 },
  connectorBody: { flex: 1, minHeight: 0, gap: 12 },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  search: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
    borderWidth: 1,
    borderRadius: 10,
    paddingLeft: 12,
    gap: 8,
  },
  input: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingVertical: 10,
    paddingRight: 8,
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
  },
  tabs: {
    flexDirection: "row",
    borderWidth: 1,
    borderRadius: 10,
    padding: 3,
    gap: 3,
  },
  tab: {
    flex: 1,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 7,
    padding: 8,
  },
  tabLabel: { fontFamily: fonts.bodySemi, fontSize: 12 },
  scopes: { flexDirection: "row", borderRadius: 10, padding: 3, gap: 3 },
  list: { flex: 1, borderWidth: 1, borderRadius: 12, overflow: "hidden" },
  connectorRow: {
    flexDirection: "row",
    alignItems: "center",
    minHeight: 88,
    padding: 12,
    gap: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  metadata: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 4,
  },
  listNotice: { padding: 12, gap: 8 },
  empty: { padding: 16, gap: 12 },
  clearSearch: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  badge: { fontFamily: fonts.bodySemi, fontSize: 10, lineHeight: 14 },
  name: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 20 },
  description: { fontFamily: fonts.bodyRegular, fontSize: 12, lineHeight: 18 },
  externalLabel: { fontSize: 10, lineHeight: 15 },
  refresh: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
});
