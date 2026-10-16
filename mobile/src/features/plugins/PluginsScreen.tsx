import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Keyboard,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import type { NativeStackNavigationProp } from "@react-navigation/native-stack";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowRight,
  ExternalLink,
  KeyRound,
  RefreshCw,
} from "lucide-react-native";
import * as WebBrowser from "expo-web-browser";
import type {
  RootStackParamList,
  WorkspaceNavigationProp,
} from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { servicesApi } from "../../lib/api/servicesApi";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import {
  connectionOwner,
  validateBrowserUrl,
} from "../services/serviceContracts";
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
}: { agentId?: string; initialTab?: Tab } = {}) {
  const { colors } = useTheme();
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [tab, setTab] = useState<Tab>(initialTab);
  const [query, setQuery] = useState("");
  const [manageId, setManageId] = useState<string>();
  const [browserError, setBrowserError] = useState<string>();
  const [browserBusy, setBrowserBusy] = useState(false);
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
    active: Boolean(manageId),
    label: "Back to plugins",
    onBack: () => setManageId(undefined),
  });

  function openConnector(item: ConnectorCardItem) {
    Keyboard.dismiss();
    if (item.added) {
      if (item.manageKeyIds?.length === 1)
        navigation.push("ServiceSetup", { connectionId: item.manageKeyIds[0] });
      else setManageId(item.id);
    } else if (item.connectSlug)
      navigation.push("ServiceSetup", { catalogSlug: item.connectSlug });
  }

  async function openStudio() {
    if (browserBusy) return;
    setBrowserBusy(true);
    setBrowserError(undefined);
    try {
      const url = new URL("/keys", validateBrowserUrl(NYXID_FRONTEND_URL));
      await WebBrowser.openBrowserAsync(url.href);
    } catch (error) {
      setBrowserError(errorText(error));
    } finally {
      setBrowserBusy(false);
    }
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
    <WorkspacePage title="Plugins" showHeading={false}>
      {tab === "connectors" && (
        <PluginSearch value={query} onChange={setQuery} />
      )}
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
        <PluginSkills
          agentId={agentId}
          onOpenChat={(params) => navigation.push("AssistantContext", params)}
        />
      ) : (
        <>
          {loadError && (
            <Notice
              message={`Some plugins could not be loaded. ${errorText(loadError)}`}
              tone="error"
            />
          )}
          {(connections.isPending || catalog.isPending) && (
            <ActivityIndicator
              color={colors.primary}
              accessibilityLabel="Loading plugins"
            />
          )}
          <View style={styles.section}>
            <Label accessibilityRole="header" style={styles.sectionTitle}>
              Added
            </Label>
            {added.map((item) => (
              <ConnectorCard
                key={item.id}
                item={item}
                onPress={() => openConnector(item)}
              />
            ))}
            {!connections.isPending && !added.length && (
              <Notice
                message={
                  connections.isError
                    ? "Your connections could not be loaded."
                    : query.trim()
                      ? "No added plugins match this search."
                      : "No connected services yet. Connect one below to get started."
                }
              />
            )}
          </View>
          {(available.length > 0 || !catalog.isPending) && (
            <View style={styles.section}>
              <Label accessibilityRole="header" style={styles.sectionTitle}>
                Available to add
              </Label>
              {available.map((item) => (
                <ConnectorCard
                  key={item.id}
                  item={item}
                  onPress={() => openConnector(item)}
                />
              ))}
              {!available.length && !catalog.isPending && (
                <Notice
                  message={
                    catalog.isError || connections.isError || !connections.data
                      ? "The available catalog could not be confirmed. Retry loading plugins."
                      : query.trim()
                        ? "No available plugins match this search."
                        : "All available connectors are already added."
                  }
                />
              )}
            </View>
          )}
          {loadError && (
            <ActionButton
              label="Retry loading plugins"
              variant="secondary"
              onPress={() => {
                void connections.refetch();
                void catalog.refetch();
              }}
            />
          )}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Manage services and Agent Keys in browser"
            accessibilityState={{ disabled: browserBusy }}
            disabled={browserBusy}
            onPress={() => void openStudio()}
            style={[styles.studio, { backgroundColor: colors.cardSoft }]}
          >
            <KeyRound color={colors.textSecondary} size={19} />
            <View style={styles.flex}>
              <Label style={styles.footerLabel}>
                Manage services &amp; Agent Keys
              </Label>
              <Label muted style={styles.externalLabel}>
                Open Studio in browser
              </Label>
            </View>
            <ExternalLink size={15} color={colors.textMuted} />
          </Pressable>
          {browserError && <Notice message={browserError} tone="error" />}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Refresh plugins"
            disabled={connections.isFetching || catalog.isFetching}
            onPress={() => {
              void connections.refetch();
              void catalog.refetch();
            }}
            style={styles.refresh}
          >
            <RefreshCw color={colors.textMuted} size={14} />
            <Label muted style={styles.description}>
              Refresh plugins
            </Label>
          </Pressable>
        </>
      )}
    </WorkspacePage>
  );
}

export function AgentSkillsScreen({ agentId }: { agentId: string }) {
  const navigation =
    useNavigation<NativeStackNavigationProp<RootStackParamList>>();
  return (
    <WorkspacePage title="Agent skills" showHeading={false}>
      <PluginSkills
        agentId={agentId}
        onOpenChat={(params) => navigation.push("AssistantContext", params)}
      />
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
    <View style={styles.search}>
      <Label muted style={styles.description}>
        Search
      </Label>
      <TextInput
        accessibilityLabel="Search plugins"
        value={value}
        onChangeText={onChange}
        placeholder="Search the catalog…"
        placeholderTextColor={colors.textMuted}
        selectionColor={colors.primary}
        autoCapitalize="none"
        autoCorrect={false}
        clearButtonMode="while-editing"
        returnKeyType="search"
        onSubmitEditing={Keyboard.dismiss}
        style={[
          styles.input,
          {
            color: colors.textPrimary,
            borderColor: colors.border,
            backgroundColor: colors.bg,
          },
        ]}
      />
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
      onPress={onPress}
    >
      <Card style={styles.card}>
        <View style={styles.row}>
          <Label style={[styles.badge, { color: statusColor }]}>
            ● {item.added ? item.status : "Connect"}
          </Label>
          <ArrowRight size={16} color={colors.textMuted} />
        </View>
        <View style={styles.row}>
          <View style={[styles.tile, { backgroundColor: colors.ghostBg }]}>
            <Text style={[styles.initial, { color: statusColor }]}>
              {item.initial}
            </Text>
          </View>
          <View style={styles.flex}>
            <Text style={[styles.name, { color: colors.textPrimary }]}>
              {item.name}
            </Text>
            <Label muted style={styles.description} numberOfLines={3}>
              {item.added ? item.summary : item.description}
            </Label>
            {!item.added && (
              <Label muted style={styles.externalLabel}>
                {item.meta}
              </Label>
            )}
          </View>
        </View>
      </Card>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, gap: 4 },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  search: { gap: 8 },
  input: {
    minHeight: 48,
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 14,
    paddingVertical: 12,
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
  section: { gap: 12 },
  sectionTitle: { fontFamily: fonts.bodySemi, fontSize: 13 },
  card: { minHeight: 114, padding: 16, gap: 8, borderRadius: 13 },
  badge: { fontFamily: fonts.bodySemi, fontSize: 10, lineHeight: 14 },
  tile: {
    width: 34,
    height: 34,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  initial: { fontFamily: fonts.bodySemi, fontSize: 14 },
  name: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 20 },
  description: { fontFamily: fonts.bodyRegular, fontSize: 12, lineHeight: 18 },
  studio: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    minHeight: 56,
    borderRadius: 8,
    padding: 14,
  },
  footerLabel: { fontSize: 12, lineHeight: 18 },
  externalLabel: { fontSize: 10, lineHeight: 15 },
  refresh: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
});
