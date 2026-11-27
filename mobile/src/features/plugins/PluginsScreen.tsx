import { translateApp } from "../../i18n/appTranslation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useContext, useEffect, useRef, useState } from "react";
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
import { ArrowRight, Plus, RefreshCw, Search, X } from "lucide-react-native";
import type { WorkspaceNavigationProp } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import * as WebBrowser from "expo-web-browser";
import { AssistantLinkContext } from "../assistant/AssistantLinkContext";
import { assistantLinkTarget } from "../assistant/assistantLinkTarget";
import { firstPartyFrontendUrls, getFrontendUrl } from "../../lib/env";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { connectLinksApi } from "../../lib/api/connectLinksApi";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import { ServiceIcon } from "../../components/ServiceIcon";
import { SkeletonRows } from "../../components/Skeleton";
import { OfflineBanner } from "../../components/OfflineBanner";
import { SegmentControl } from "../../components/SegmentControl";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { servicesApi } from "../../lib/api/servicesApi";
import { radius, spacing, typeScale } from "../../theme/designTokens";
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
import { useRailScrollIndicator } from "../../components/workspace/workspaceFloatingInset";

type Tab = "connectors" | "skills";
const ROOT = ["workspace", "services"] as const;
const errorText = (error: unknown) =>
  error instanceof Error
    ? error.message
    : translateApp("Could not load plugins. Try again.");

export function PluginsScreen({
  agentId,
  initialTab = "connectors",
  connectorId,
}: { agentId?: string; initialTab?: Tab; connectorId?: string } = {}) {
  const railIndicator = useRailScrollIndicator();
  const appLanguage = useAppLanguage().language;
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [tab, setTab] = useState<Tab>(initialTab);
  const [query, setQuery] = useState("");
  const canFetch = focused && foreground && isConnected;
  const currentCanFetch = useRef(canFetch);
  currentCanFetch.current = canFetch;
  const manageId = connectorId;
  const showLink = useContext(AssistantLinkContext);
  const [modelPicker, setModelPicker] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [columns, setColumns] = useState(1);
  const pending = useRef<AbortController | null>(null);
  useEffect(() => {
    setBusy(false);
    return () => {
      pending.current?.abort();
      pending.current = null;
    };
  }, [canFetch, tab]);

  const connections = useQuery({
    queryKey: [...ROOT, "plugins"],
    queryFn: ({ signal }) => servicesApi.pluginConnections(signal),
    enabled: canFetch,
    retry: false,
  });
  const catalog = useQuery({
    queryKey: [...ROOT, "catalog"],
    queryFn: ({ signal }) => servicesApi.catalog(signal),
    enabled: canFetch,
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
  // Offline queries remain pending while paused; that is not active loading.
  const loadingConnections = isConnected && connections.isPending;
  const cached = connections.data !== undefined || catalog.data !== undefined;
  const offlineBanner = !isConnected ? (
    <OfflineBanner
      subtitle={
        cached
          ? t("Showing cached plugins. Reconnect to refresh.")
          : t("Reconnect to load your plugins.")
      }
    />
  ) : null;
  // Only the user's own pull or Refresh tap shows a spinner; background
  // refocus and foreground refetches never do.
  const pull = usePullRefresh(() =>
    Promise.all([connections.refetch(), catalog.refetch()]),
  );
  const refreshing = canFetch && pull.refreshing;
  const refresh = () => {
    if (currentCanFetch.current) pull.onRefresh();
  };

  useEffect(() => {
    setTab(initialTab);
  }, [initialTab, agentId]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state === "active" && focused && isConnected) {
        void connections.refetch();
        void catalog.refetch();
      }
    });
    return () => subscription.remove();
  }, [focused, isConnected, connections.refetch, catalog.refetch]);
  useEffect(() => {
    navigation.setOptions({
      title: manageItem?.name ?? translateApp("Plugins"),
    });
  }, [navigation, manageItem?.name, appLanguage]);
  useWorkspaceBack({
    active: false,
    defaultRole: manageId ? "back" : "menu",
    label: translateApp("Back to plugins"),
    onBack: navigation.goBack,
  });

  function openConnector(item: ConnectorCardItem) {
    Keyboard.dismiss();
    if (item.added) {
      if (item.manageKeyIds?.length === 1)
        navigation.push("ServiceSetup", { connectionId: item.manageKeyIds[0] });
      else navigation.push("PluginConnections", { connectorId: item.id });
    } else if (item.advancedSetupSlug)
      void openAdvanced(item.advancedSetupSlug);
    else if (item.connectSlug) void connect(item.connectSlug, false);
  }

  async function runAction(
    action: (signal: AbortSignal, current: () => boolean) => Promise<void>,
  ) {
    if (!currentCanFetch.current || pending.current) return;
    const controller = new AbortController();
    const epoch = getAuthEpoch();
    const current = () =>
      !controller.signal.aborted &&
      currentCanFetch.current &&
      epoch === getAuthEpoch();
    pending.current = controller;
    setBusy(true);
    setActionError(undefined);
    try {
      await action(controller.signal, current);
    } catch (error) {
      if (current()) setActionError(errorText(error));
    } finally {
      if (pending.current === controller) {
        pending.current = null;
        setBusy(false);
      }
    }
  }

  function connect(slug: string, personal: boolean) {
    return runAction(async (signal, current) => {
      const link = await connectLinksApi.create(slug, signal, undefined, {
        personal,
      });
      if (!current()) return;
      const target = assistantLinkTarget(
        link.connect_url,
        firstPartyFrontendUrls(),
      );
      if (target?.kind !== "connect" || !showLink?.(target.href))
        throw new Error(t("Could not create connection link"));
    });
  }

  function openAdvanced(slug?: string) {
    return runAction(async (_signal, current) => {
      const url = new URL("/keys", validateBrowserUrl(getFrontendUrl()));
      url.searchParams.set("tab", "services");
      url.searchParams.set("action", "add-service");
      if (slug) url.searchParams.set("slug", slug);
      await WebBrowser.openBrowserAsync(url.href);
      if (current())
        await Promise.all([connections.refetch(), catalog.refetch()]);
    });
  }

  if (manageId)
    return (
      <WorkspacePage
        title={manageItem?.name ?? translateApp("Connections")}
        showHeading={false}
        contentStyle={styles.grouped}
        refreshing={refreshing}
        onRefresh={refresh}
      >
        {offlineBanner}
        {isConnected && loadError && (
          <Notice message={errorText(loadError)} tone="error" />
        )}
        {loadingConnections && !manageItem ? (
          <SkeletonRows
            rows={3}
            title
            accessibilityLabel={t("Loading plugins")}
          />
        ) : manageItem ? (
          <SettingsGroup
            title={manageItem.name}
            footer={translateApp("Choose a connection to manage.")}
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
                    : translateApp("Disabled")
                }
                accessibilityLabel={translateApp("Manage {{slug}}", {
                  slug: connection.label,
                })}
                onPress={() =>
                  navigation.push("ServiceSetup", {
                    connectionId: connection.id,
                  })
                }
              />
            ))}
          </SettingsGroup>
        ) : (
          <SettingsGroup
            footer={
              !isConnected
                ? t("Reconnect to load these connections.")
                : connections.isError
                  ? t("Your connections could not be loaded.")
                  : translateApp(
                      "These connections are no longer in your list. Refresh Plugins to see the current catalog.",
                    )
            }
          >
            <SettingsRow
              label={translateApp("Refresh plugins")}
              tone="action"
              disabled={!canFetch || refreshing}
              onPress={refresh}
            />
          </SettingsGroup>
        )}
      </WorkspacePage>
    );

  const searching = !!query.trim();
  const modelCard = {
    name: t("Add a model connection"),
    category: t("Models"),
    description: t(
      "Use your own provider account or a custom OpenAI-compatible endpoint.",
    ),
  };
  const showModels =
    !searching ||
    matchesPluginQuery(modelCard, query) ||
    items.models.some((item) => matchesPluginQuery(item, query));
  const sections: {
    key: string;
    title: string;
    data: ConnectorCardItem[][];
    empty?: string;
  }[] = [];
  const rows = (cards: readonly ConnectorCardItem[]) => {
    const result: ConnectorCardItem[][] = [];
    for (let i = 0; i < cards.length; i += columns)
      result.push(cards.slice(i, i + columns));
    return result;
  };
  if (added.length || !searching)
    sections.push({
      key: "added",
      title: t("Connected"),
      data: rows(added),
      empty:
        !isConnected && !connections.data
          ? t("Reconnect to load your added connectors.")
          : connections.isError
            ? t("Connected services could not be loaded right now.")
            : t(
                "No connected services yet. Connect one below and your assistant can call it through the NyxID proxy.",
              ),
    });
  if (showModels)
    sections.push({ key: "models", title: t("Models"), data: [] });
  if (available.length)
    sections.push({
      key: "discover",
      title: t("Browse"),
      data: rows(available),
    });
  const canPickModel = canFetch && !busy;
  const canAdd = canPickModel && catalogKnown;
  const customModel = "custom:model-endpoint";

  return (
    <WorkspacePage title={t("Plugins")} showHeading={false} scroll={false}>
      <Text style={[styles.intro, { color: colors.textSecondary }]}>
        {t(
          "Connect your services and model accounts. Add skills to teach your assistant new workflows.",
        )}
      </Text>
      <SegmentControl
        segments={[
          { label: translateApp("Connectors") },
          { label: translateApp("Skills") },
        ]}
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
          {...railIndicator}
        >
          <PluginSkills agentId={agentId} />
        </ScrollView>
      ) : (
        <View
          style={styles.connectorBody}
          onLayout={({ nativeEvent }) =>
            setColumns(
              Math.max(
                1,
                Math.min(
                  4,
                  Math.floor(
                    (nativeEvent.layout.width + spacing.lg) /
                      (280 + spacing.lg),
                  ),
                ),
              ),
            )
          }
        >
          <View style={styles.searchRow}>
            <PluginSearch value={query} onChange={setQuery} />
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={translateApp("Refresh plugins")}
              accessibilityState={{ disabled: !canFetch || refreshing }}
              disabled={!canFetch || refreshing}
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
          {actionError && <Notice message={actionError} tone="error" />}
          <SectionList
            style={styles.body}
            contentContainerStyle={styles.listContent}
            accessibilityLabel={translateApp("Connectors")}
            sections={sections}
            keyExtractor={(row) => row[0]!.id}
            stickySectionHeadersEnabled={false}
            renderSectionHeader={({ section }) => (
              <Text
                accessibilityRole="header"
                style={[styles.sectionTitle, { color: colors.textMuted }]}
              >
                {section.title}
              </Text>
            )}
            renderSectionFooter={({ section }) => (
              <View style={styles.sectionGap}>
                {section.key === "added" &&
                !section.data.length &&
                loadingConnections ? (
                  <SkeletonRows
                    rows={3}
                    title
                    accessibilityLabel={t("Loading plugins")}
                  />
                ) : section.key === "models" ? (
                  <View style={styles.cardGridRow}>
                    <View style={styles.cardColumn}>
                      <ConnectorCard
                        name={modelCard.name}
                        description={modelCard.description}
                        subtitle={t("Model providers")}
                        action={t("Choose provider")}
                        accessibilityLabel={modelCard.name}
                        disabled={!canPickModel}
                        onPress={() => {
                          Keyboard.dismiss();
                          setModelPicker(true);
                        }}
                      />
                    </View>
                    {Array.from({ length: columns - 1 }, (_, i) => (
                      <View key={i} style={styles.cardColumn} />
                    ))}
                  </View>
                ) : !section.data.length && section.empty ? (
                  <Text style={[styles.empty, { color: colors.textMuted }]}>
                    {section.empty}
                  </Text>
                ) : null}
              </View>
            )}
            renderItem={({ item: row }) => (
              <View style={styles.cardGridRow}>
                {row.map((item) => {
                  const subtitle = item.added
                    ? [item.status, item.connectionCountLabel]
                        .filter(Boolean)
                        .join(" · ")
                    : authLabel(item.meta);
                  const action = item.added
                    ? t("Manage")
                    : item.advancedSetupSlug
                      ? t("Advanced setup")
                      : t("Connect");
                  return (
                    <View key={item.id} style={styles.cardColumn}>
                      <ConnectorCard
                        name={item.name}
                        description={item.description}
                        subtitle={subtitle}
                        action={action}
                        item={item}
                        accessibilityLabel={`${action} ${item.name}, ${subtitle}`}
                        disabled={!item.added && !canAdd}
                        onPress={() => openConnector(item)}
                        onPersonal={
                          item.connectOwnSlug
                            ? () => void connect(item.connectOwnSlug!, true)
                            : undefined
                        }
                        canAdd={canAdd}
                      />
                    </View>
                  );
                })}
                {Array.from({ length: columns - row.length }, (_, i) => (
                  <View key={i} style={styles.cardColumn} />
                ))}
              </View>
            )}
            keyboardShouldPersistTaps="handled"
            keyboardDismissMode="on-drag"
            initialNumToRender={14}
            refreshing={refreshing}
            onRefresh={refresh}
            ListHeaderComponent={
              !isConnected || loadError ? (
                <View style={styles.listHeader}>
                  {offlineBanner}
                  {isConnected && loadError ? (
                    <>
                      <Notice
                        message={translateApp(
                          "Some plugins could not be loaded. {{errorText}}",
                          { errorText: errorText(loadError) },
                        )}
                        tone="error"
                      />
                      <SettingsGroup>
                        <SettingsRow
                          label={translateApp("Retry loading plugins")}
                          tone="action"
                          disabled={!canFetch || refreshing}
                          onPress={refresh}
                        />
                      </SettingsGroup>
                    </>
                  ) : null}
                </View>
              ) : null
            }
            ListEmptyComponent={
              searching && !loadingConnections && !catalog.isPending ? (
                <Text style={[styles.empty, { color: colors.textMuted }]}>
                  {t("No plugins match this search.")}
                </Text>
              ) : null
            }
            ListFooterComponent={
              <SettingsGroup inset={false}>
                <SettingsRow
                  label={t("Advanced setup")}
                  disabled={!canFetch || busy}
                  onPress={() => void openAdvanced()}
                />
              </SettingsGroup>
            }
            {...railIndicator}
          />
          {modelPicker && (
            <WorkspaceSelectionSheet
              label={modelCard.name}
              description={t(
                "Use your own provider account or model endpoint.",
              )}
              confirmLabel={t("Continue")}
              value={[]}
              disabled={!canPickModel}
              scopeKey={String(getAuthEpoch())}
              options={[
                ...items.models.map((item) => ({
                  value: item.connectSlug!,
                  label: item.name,
                  leading: (
                    <ServiceIcon
                      slug={item.iconSlug}
                      iconUrl={item.iconUrl}
                      size="md"
                    />
                  ),
                })),
                {
                  value: customModel,
                  label: t("Custom OpenAI-compatible endpoint"),
                  leading: <ServiceIcon slug="custom" size="md" />,
                },
              ]}
              onClose={() => setModelPicker(false)}
              onChange={([value]) => {
                if (value === customModel) void openAdvanced();
                else if (
                  value &&
                  items.models.some((item) => item.connectSlug === value)
                )
                  void connect(value, true);
              }}
            />
          )}
        </View>
      )}
    </WorkspacePage>
  );
}

export function AgentSkillsScreen({ agentId }: { agentId: string }) {
  useAppLanguage();
  return (
    <WorkspacePage
      title={translateApp("Agent skills")}
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
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <View style={[styles.search, { backgroundColor: colors.segmentTrack }]}>
      <Search size={17} color={colors.textMuted} accessible={false} />
      <TextInput
        accessibilityLabel={translateApp("Search plugins")}
        value={value}
        onChangeText={onChange}
        placeholder={translateApp("Search the catalog...")}
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
          accessibilityLabel={translateApp("Clear connector search")}
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

const sentence = (value: string) =>
  value.charAt(0).toUpperCase() + value.slice(1);

// Discover rows tag how the service authenticates, beside its name.
const authLabel = (meta: string) =>
  meta === "ssh"
    ? "SSH"
    : meta === "oauth"
      ? "OAuth"
      : meta === "api key"
        ? translateApp("API key")
        : sentence(meta);

function ConnectorCard({
  name,
  description,
  subtitle,
  action,
  accessibilityLabel,
  disabled,
  onPress,
  item,
  onPersonal,
  canAdd,
}: {
  name: string;
  description: string;
  subtitle: string;
  action: string;
  accessibilityLabel: string;
  disabled: boolean;
  onPress: () => void;
  item?: ConnectorCardItem;
  onPersonal?: () => void;
  canAdd?: boolean;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const statusColor =
    item?.statusTone === "success"
      ? colors.success
      : item?.statusTone === "warning"
        ? colors.warning
        : item?.statusTone === "error"
          ? colors.danger
          : colors.textMuted;
  return (
    <View
      style={[
        styles.card,
        { borderColor: colors.borderSoft, backgroundColor: colors.card },
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={accessibilityLabel}
        accessibilityHint={description}
        accessibilityState={{ disabled }}
        disabled={disabled}
        onPress={onPress}
        style={({ pressed }) => [
          styles.cardMain,
          { opacity: pressed || disabled ? 0.6 : 1 },
        ]}
      >
        <View style={styles.cardHead}>
          {item && (item.iconSlug || item.iconUrl) ? (
            <ServiceIcon
              iconUrl={item.iconUrl}
              slug={item.iconSlug ?? "custom"}
              size="md"
            />
          ) : (
            <View
              style={[styles.modelIcon, { backgroundColor: colors.cardSoft }]}
            >
              {item ? (
                <Text style={[styles.cardTitle, { color: colors.textPrimary }]}>
                  {name
                    .trim()
                    .split(/\s+/)
                    .slice(0, 2)
                    .map((word) => word.charAt(0))
                    .join("")
                    .toUpperCase() || "?"}
                </Text>
              ) : (
                <Plus size={22} color={colors.textPrimary} />
              )}
            </View>
          )}
          <View style={styles.cardColumn}>
            <Text style={[styles.cardTitle, { color: colors.textPrimary }]}>
              {name}
            </Text>
            <View style={styles.subtitle}>
              {item?.added && !item.disabled ? (
                <View
                  style={[styles.statusDot, { backgroundColor: statusColor }]}
                />
              ) : null}
              <Text style={[styles.cardMeta, { color: colors.textSecondary }]}>
                {subtitle}
              </Text>
            </View>
          </View>
        </View>
        <Text
          numberOfLines={2}
          style={[styles.cardDescription, { color: colors.textSecondary }]}
        >
          {description}
        </Text>
        <View style={styles.cardAction}>
          <Text style={[styles.cardActionText, { color: colors.textPrimary }]}>
            {action}
          </Text>
          <ArrowRight size={15} color={colors.textPrimary} />
        </View>
      </Pressable>
      {onPersonal && (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("Connect your own {{name}}", { name })}
          disabled={!canAdd}
          accessibilityState={{ disabled: !canAdd }}
          onPress={onPersonal}
          style={styles.personalAction}
        >
          <Text style={[styles.cardActionText, { color: colors.primary }]}>
            {t("Connect your own")}
          </Text>
        </Pressable>
      )}
    </View>
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
  intro: { ...typeScale.description },
  cardGridRow: {
    flexDirection: "row",
    gap: spacing.lg,
    marginBottom: spacing.lg,
  },
  cardColumn: { flex: 1, minWidth: 0 },
  card: {
    flex: 1,
    borderRadius: radius.lg,
    borderWidth: 1,
    overflow: "hidden",
  },
  cardMain: { flexGrow: 1, padding: spacing.xxl, gap: spacing.lg },
  cardHead: { flexDirection: "row", alignItems: "center", gap: spacing.lg },
  cardTitle: { ...typeScale.title },
  cardMeta: { ...typeScale.caption, flexShrink: 1 },
  subtitle: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    marginTop: spacing.xxs,
  },
  statusDot: { width: 7, height: 7, borderRadius: radius.full },
  modelIcon: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  cardDescription: { ...typeScale.body, flexGrow: 1 },
  cardAction: {
    flexDirection: "row",
    alignSelf: "flex-end",
    alignItems: "center",
    gap: spacing.xs,
  },
  cardActionText: { ...typeScale.bodyStrong },
  personalAction: {
    paddingHorizontal: spacing.xxl,
    justifyContent: "center",
    minHeight: 44,
  },
  listHeader: { gap: spacing.lg, paddingBottom: spacing.lg },
  sectionTitle: {
    ...typeScale.h2,
    paddingBottom: spacing.sm,
  },
  sectionGap: { marginBottom: spacing.lg },
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
