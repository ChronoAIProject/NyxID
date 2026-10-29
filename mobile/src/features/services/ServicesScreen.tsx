import { ServiceIcon } from "../../components/ServiceIcon";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Keyboard,
  Linking,
  Pressable,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as WebBrowser from "expo-web-browser";
import * as Clipboard from "expo-clipboard";
import { ChevronRight, ChevronDown } from "lucide-react-native";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { ApiError } from "../../lib/api/ApiError";
import { servicesApi } from "../../lib/api/servicesApi";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import type { WorkspaceNavigationProp } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import {
  buildCatalogConnectionInput,
  canEditConnection,
  catalogForConnection,
  connectionOwner,
  supportsCredentialForm,
  supportsManagedOAuth,
  validateBrowserUrl,
  type CatalogEntry,
  type ServiceConnection,
} from "./serviceContracts";
import {
  canManageProviderPermissions,
  describeConnection,
  isOAuthConnection,
  reconnectScopeOverride,
} from "./connectionPresentation";
import { PluginConnectionHistory } from "../plugins/PluginConnectionHistory";

const ROOT = ["workspace", "services"] as const;
const messageOf = (cause: unknown) =>
  cause instanceof Error
    ? cause.message
    : "Could not complete this action. Try again.";

export function ServicesScreen({
  initialConnectionId,
  initialCatalogSlug,
  onClose,
}: {
  initialConnectionId?: string;
  initialCatalogSlug?: string;
  onClose?: () => void;
} = {}) {
  const { colors } = useTheme();
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const focused = useIsFocused();
  const queryClient = useQueryClient();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [tab, setTab] = useState<"connected" | "catalog">("connected");
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<string | undefined>(
    initialConnectionId,
  );
  const [adding, setAdding] = useState<CatalogEntry>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [flowComplete, setFlowComplete] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [tokenOnlyRemoval, setTokenOnlyRemoval] = useState(false);
  const [replacement, setReplacement] = useState("");
  const [formDirty, setFormDirty] = useState(false);
  const [managementTab, setManagementTab] = useState<
    "overview" | "advanced" | "history"
  >("overview");
  const [permissionsOpen, setPermissionsOpen] = useState(false);
  const [credentialEditorOpen, setCredentialEditorOpen] = useState(false);
  const actionLock = useRef(false);
  const launchedCatalog = useRef(false);
  const connections = useQuery({
    queryKey: [...ROOT, "connections"],
    queryFn: ({ signal }) => servicesApi.connections(signal),
    retry: false,
    enabled: focused && foreground,
  });
  const catalog = useQuery({
    queryKey: [...ROOT, "catalog"],
    queryFn: ({ signal }) => servicesApi.catalog(signal),
    retry: false,
    enabled: focused && foreground,
    staleTime: 60_000,
  });
  const detail = useQuery({
    queryKey: [...ROOT, "connection", selectedId],
    queryFn: ({ signal }) => servicesApi.connection(selectedId!, signal),
    enabled: focused && foreground && Boolean(selectedId),
    retry: false,
  });
  const detailSlug = detail.data?.catalog_service_slug;
  useEffect(() => {
    navigation.setOptions({
      title: selectedId
        ? "Manage connection"
        : adding
          ? `Connect ${adding.name}`
          : initialCatalogSlug
            ? "Connect service"
            : "Services",
    });
  }, [navigation, selectedId, adding, initialCatalogSlug]);
  useEffect(() => {
    setManagementTab("overview");
    setPermissionsOpen(false);
    setCredentialEditorOpen(false);
  }, [selectedId]);
  const detailCatalog = useQuery({
    queryKey: [...ROOT, "catalog-entry", detailSlug],
    queryFn: ({ signal }) => servicesApi.catalogEntry(detailSlug!, signal),
    enabled: focused && foreground && Boolean(detailSlug),
    staleTime: 60_000,
    retry: false,
  });
  useEffect(() => {
    if (!initialCatalogSlug || launchedCatalog.current || !catalog.data) return;
    const entry = catalog.data.find((item) => item.slug === initialCatalogSlug);
    if (entry) {
      launchedCatalog.current = true;
      setAdding(entry);
    }
  }, [initialCatalogSlug, catalog.data]);

  const reload = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ROOT });
    await queryClient.invalidateQueries({ queryKey: ["workspace", "agents"] });
  }, [queryClient]);

  useEffect(() => {
    if (!focused) return;
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state === "active") void reload();
      else setReplacement("");
    });
    return () => subscription.remove();
  }, [focused, reload]);

  useEffect(() => {
    if (!focused) setReplacement("");
  }, [focused]);

  function closeDetail() {
    if (actionLock.current) return;
    setSelectedId(undefined);
    setAdding(undefined);
    setError(undefined);
    setConfirmRemove(false);
    setTokenOnlyRemoval(false);
    setReplacement("");
    onClose?.();
  }
  useWorkspaceBack({
    // A dedicated setup route already has native history. Guard it only while
    // writing; a local detail screen must instead consume its own Back action.
    active: onClose ? busy : Boolean(selectedId) || Boolean(adding),
    defaultRole: onClose ? "back" : "menu",
    disabled: busy,
    discardOnClose:
      formDirty || Boolean(replacement)
        ? {
            title: "Discard connection changes?",
            message: "Your unsaved connection changes will be lost.",
          }
        : undefined,
    label: onClose ? "Back to plugins" : "Back to services",
    onBack: () => {
      if (!actionLock.current) closeDetail();
    },
  });

  async function run(action: () => Promise<void>) {
    if (actionLock.current) return;
    actionLock.current = true;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await action();
    } catch (cause) {
      setError(messageOf(cause));
    } finally {
      actionLock.current = false;
      setBusy(false);
    }
  }

  async function authorize(
    connection: ServiceConnection,
    entry: CatalogEntry,
    repairing = false,
  ) {
    if (!entry.provider_config_id)
      throw new Error("This service has no OAuth provider configured.");
    const org =
      connection.credential_source?.type === "org"
        ? connection.credential_source.org_id
        : undefined;
    const response = await servicesApi.initiateOAuth(
      entry.provider_config_id,
      connection.id,
      org,
      repairing ? reconnectScopeOverride(connection, entry) : undefined,
    );
    await WebBrowser.openBrowserAsync(
      validateBrowserUrl(response.authorization_url),
    );
    await reload();
    const latest = await servicesApi.connection(connection.id);
    queryClient.setQueryData([...ROOT, "connection", connection.id], latest);
    const authorized =
      latest.status === "active" &&
      (connection.status !== "active" ||
        latest.last_authorized_at !== connection.last_authorized_at);
    setNotice(
      authorized
        ? "Authorization completed. Your connection is ready."
        : "Authorization has not completed yet. Finish it in the browser, then check again.",
    );
  }

  async function remove(connection: ServiceConnection) {
    try {
      const result = await servicesApi.remove(connection.id, tokenOnlyRemoval);
      if (!result.deleted)
        throw new Error(result.message || "The connection was not removed.");
      // Removal completes this flow; the action lock only guards user taps.
      setSelectedId(undefined);
      setConfirmRemove(false);
      setTokenOnlyRemoval(false);
      setReplacement("");
      await reload();
      setNotice(
        result.upstream_revocation_scheduled
          ? "Connection removed. Provider authorization revocation was scheduled."
          : "Connection removed.",
      );
      if (onClose) setFlowComplete(true);
    } catch (cause) {
      if (cause instanceof ApiError && cause.errorCode === 11500) {
        setTokenOnlyRemoval(true);
        setError(
          "Other connections share this provider authorization. You can remove only this service below; the provider authorization and other connections will be kept.",
        );
      } else {
        throw cause;
      }
    }
  }

  if (flowComplete)
    return (
      <WorkspacePage showHeading={false} title="Connection removed">
        <Notice message={notice ?? "Connection removed."} tone="success" />
      </WorkspacePage>
    );

  if (adding) {
    return (
      <ConnectForm
        entry={adding}
        busy={busy}
        error={error}
        onRefresh={reload}
        onDirty={setFormDirty}
        onSubmit={(values) =>
          void run(async () => {
            const connection = await servicesApi.create(
              buildCatalogConnectionInput(adding, values),
            );
            setSelectedId(connection.id);
            setAdding(undefined);
            setReplacement("");
            await reload();
            if (!values.platform && supportsManagedOAuth(adding))
              await authorize(connection, adding);
            else
              setNotice("Connection added. Its status below comes from NyxID.");
          })
        }
      />
    );
  }

  if (selectedId) {
    const connection = detail.data;
    const entry =
      connection &&
      (detailCatalog.data ??
        catalogForConnection(connection, catalog.data ?? []));
    const editable = connection && canEditConnection(connection);
    const health = connection && describeConnection(connection);
    const oauth = connection && isOAuthConnection(connection, entry);
    const restricted =
      connection?.credential_source?.type === "org" &&
      !connection.credential_source.allowed;
    const providerName =
      entry?.name ?? connection?.catalog_service_name ?? "the provider";
    const scopes = [...new Set(connection?.granted_scopes ?? [])];
    const showPermissions = Boolean(oauth || scopes.length);
    const canEditPermissions =
      connection && canManageProviderPermissions(connection, entry);
    const canRecover =
      editable &&
      health?.recovery &&
      oauth &&
      connection.credential_binding !== "platform";
    const revokesGrant =
      connection?.revocation?.revokes_grant ??
      entry?.revocation?.revokes_grant ??
      false;
    const replaceable =
      editable &&
      Boolean(connection.api_key_id) &&
      !["oauth2", "node_managed"].includes(connection.credential_type) &&
      !connection.credential_missing &&
      connection.status !== "pending_auth" &&
      connection.credential_binding !== "platform" &&
      (!connection.catalog_service_slug || Boolean(entry)) &&
      (!entry ||
        (supportsCredentialForm(entry) &&
          !entry.token_exchange_credential_fields?.length));
    const openWeb = (target: "overview" | "advanced") =>
      run(async () => {
        const url = new URL(
          `/keys/${encodeURIComponent(selectedId)}`,
          validateBrowserUrl(NYXID_FRONTEND_URL),
        );
        url.searchParams.set("tab", target);
        await WebBrowser.openBrowserAsync(url.href);
        await reload();
      });
    return (
      <WorkspacePage title="Manage connection" showHeading={false}>
        {connection && health && (
          <Card style={styles.serviceCard}>
            <View style={styles.row}>
              <View style={[styles.icon, { backgroundColor: colors.ghostBg }]}>
                <ServiceIcon
                  slug={connection.catalog_service_slug ?? connection.slug}
                  iconUrl={connection.icon_url || connection.catalog_icon_url}
                  size="md"
                />
              </View>
              <View style={styles.flex}>
                <Text style={[styles.title, { color: colors.textPrimary }]}>
                  {connection.label}
                </Text>
                <Label muted style={styles.cardMetadata}>
                  {connectionOwner(connection)} ·{" "}
                  {connection.credential_binding === "platform"
                    ? "Platform key"
                    : connection.credential_type === "none"
                      ? "No authentication"
                      : connection.credential_type === "oauth2"
                        ? "OAuth"
                        : connection.credential_type.replaceAll("_", " ")}
                </Label>
              </View>
            </View>
            {(entry?.description ?? connection.description) ? (
              <Label muted style={styles.cardDescription}>
                {entry?.description ?? connection.description}
              </Label>
            ) : null}
            <Text
              style={[
                styles.connectionStatus,
                {
                  color:
                    health.tone === "success"
                      ? colors.success
                      : health.tone === "error"
                        ? colors.danger
                        : health.tone === "warning"
                          ? colors.warning
                          : colors.textSecondary,
                },
              ]}
            >
              ●{" "}
              {!connection.is_active
                ? "Paused"
                : health.tone === "success"
                  ? "Connected"
                  : health.label}
            </Text>
            <Label muted style={styles.cardDescription}>
              {!connection.is_active && health.tone !== "success"
                ? `${health.label}. ${health.description}`
                : health.description}
            </Label>
            {connection.error_message && (
              <Notice message={connection.error_message} tone="error" />
            )}
            {canRecover && (
              <>
                <Label muted style={styles.cardDescription}>
                  Continue in your browser, then return here to check the
                  result.
                </Label>
                <ActionButton
                  label={
                    health.recovery === "continue"
                      ? "Continue authentication"
                      : "Reconnect"
                  }
                  disabled={busy}
                  onPress={() =>
                    entry &&
                    supportsManagedOAuth(entry) &&
                    !connection.credential_missing
                      ? void run(() => authorize(connection, entry, true))
                      : void openWeb("overview")
                  }
                />
              </>
            )}
            {editable &&
              health.recovery &&
              !oauth &&
              connection.credential_binding !== "platform" && (
                <ActionButton
                  label="Repair connection in Web"
                  variant="secondary"
                  disabled={busy}
                  onPress={() => void openWeb("overview")}
                />
              )}
          </Card>
        )}
        <View
          style={[
            styles.tabs,
            { backgroundColor: colors.cardSoft, borderColor: colors.border },
          ]}
        >
          {(["overview", "advanced", "history"] as const).map((value) => (
            <Pressable
              key={value}
              accessibilityRole="tab"
              accessibilityLabel={
                value.charAt(0).toUpperCase() + value.slice(1)
              }
              accessibilityState={{
                selected: managementTab === value,
                disabled: busy,
              }}
              disabled={busy}
              onPress={() => {
                setManagementTab(value);
                setReplacement("");
                setCredentialEditorOpen(false);
                setConfirmRemove(false);
                setTokenOnlyRemoval(false);
              }}
              style={[
                styles.tab,
                styles.managementTab,
                {
                  backgroundColor:
                    managementTab === value ? colors.ghostBg : "transparent",
                },
              ]}
            >
              <Text
                style={[
                  styles.tabLabel,
                  {
                    color:
                      managementTab === value
                        ? colors.textPrimary
                        : colors.textMuted,
                  },
                ]}
              >
                {value.charAt(0).toUpperCase() + value.slice(1)}
              </Text>
            </Pressable>
          ))}
        </View>
        {notice && <Notice message={notice} tone="success" />}
        {error && <Notice message={error} tone="error" />}
        {managementTab !== "history" && detail.isPending && (
          <ActivityIndicator
            color={colors.primary}
            accessibilityLabel="Loading connection"
          />
        )}
        {managementTab !== "history" && detail.isError && (
          <>
            <Notice message={messageOf(detail.error)} tone="error" />
            <ActionButton label="Retry" onPress={() => void detail.refetch()} />
          </>
        )}
        {connection && detailCatalog.isError && managementTab !== "history" && (
          <>
            <Notice
              message="Service setup details could not be loaded. Reload them to check available authorization and credential actions."
              tone="error"
            />
            <ActionButton
              label="Reload service details"
              variant="secondary"
              onPress={() => void detailCatalog.refetch()}
            />
          </>
        )}
        {managementTab === "history" && (
          <PluginConnectionHistory serviceId={selectedId} />
        )}
        {connection && managementTab === "overview" && (
          <>
            <Card>
              <Label style={styles.sectionTitle}>Use in Chat</Label>
              {restricted ? (
                <Label muted style={styles.cardDescription}>
                  You can view this organization connection, but you cannot use
                  it yet. Ask an organization admin to grant you access.
                </Label>
              ) : !connection.is_active ? (
                <Label muted style={styles.cardDescription}>
                  {editable
                    ? "This connection is paused. Turn on Connection enabled below before using it in Chat."
                    : "This connection is paused. Ask its administrator to enable it before using it in Chat."}
                </Label>
              ) : health?.tone === "error" || health?.tone === "warning" ? (
                <Label muted style={styles.cardDescription}>
                  Resolve the connection issue above before using it in Chat.
                  The agent must also have permission to use this connection.
                </Label>
              ) : (
                <Label muted style={styles.cardDescription}>
                  Ask your agent to use {providerName}. For a specialist, first
                  select this connection in Agents & Crews → choose the agent →
                  Service permissions, then save.
                </Label>
              )}
            </Card>
            <Card>
              <View style={styles.statusRow}>
                <View style={styles.flex}>
                  <Label style={styles.sectionTitle}>Connection enabled</Label>
                  <Label muted style={styles.cardDescription}>
                    {connection.is_active
                      ? "On — NyxID may use this connection within the agent’s permissions."
                      : "Off — requests through this connection are paused."}
                  </Label>
                </View>
                <Switch
                  accessibilityLabel="Connection enabled"
                  value={connection.is_active}
                  disabled={
                    !editable ||
                    busy ||
                    (Boolean(connection.credential_missing) &&
                      !connection.is_active)
                  }
                  trackColor={{ true: colors.primary }}
                  onValueChange={(active) =>
                    void run(async () => {
                      await servicesApi.setActive(connection.id, active);
                      await reload();
                      setNotice(
                        active
                          ? "Connection enabled."
                          : "Connection paused. Your credential is kept.",
                      );
                    })
                  }
                />
              </View>
              <Label muted style={styles.cardDescription}>
                Turning this off keeps your saved credential. Turn it on again
                to resume use.
              </Label>
              {!editable && (
                <Label muted style={styles.cardDescription}>
                  {connection.auto_connected
                    ? "This connection is managed by the platform."
                    : "An organization admin manages this connection. Your role has read-only access."}
                </Label>
              )}
            </Card>
            {showPermissions && (
              <Card style={styles.permissionsCard}>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Provider permissions"
                  accessibilityHint="Review the access granted by the provider, separate from agent service permissions."
                  accessibilityState={{ expanded: permissionsOpen }}
                  onPress={() => setPermissionsOpen((value) => !value)}
                  style={styles.permissionsToggle}
                >
                  <View style={styles.flex}>
                    <Label style={styles.sectionTitle}>
                      Provider permissions
                    </Label>
                    <Label muted style={styles.cardMetadata}>
                      {scopes.length
                        ? `${scopes.length} granted permission${scopes.length === 1 ? "" : "s"}`
                        : "No permission list available"}
                    </Label>
                  </View>
                  {permissionsOpen ? (
                    <ChevronDown size={18} color={colors.textMuted} />
                  ) : (
                    <ChevronRight size={18} color={colors.textMuted} />
                  )}
                </Pressable>
                <View style={styles.permissionsContent}>
                  <Label muted style={styles.cardDescription}>
                    What {providerName} allows NyxID to access. This does not
                    decide which agents may use the connection.
                  </Label>
                  {permissionsOpen && (
                    <>
                      {scopes.length ? (
                        scopes.map((scope) => {
                          const info = entry?.scope_catalog?.find(
                            (item) => item.scope === scope,
                          );
                          return (
                            <View key={scope} style={styles.scopeItem}>
                              {info?.label && <Label>{info.label}</Label>}
                              {info?.description && (
                                <Label muted style={styles.cardDescription}>
                                  {info.description}
                                </Label>
                              )}
                              <Label
                                selectable
                                mono
                                muted
                                style={styles.cardMetadata}
                              >
                                {scope}
                              </Label>
                            </View>
                          );
                        })
                      ) : (
                        <Label muted style={styles.cardDescription}>
                          NyxID has no saved permission list for this
                          connection. This alone does not mean authorization
                          failed.
                        </Label>
                      )}
                      {canEditPermissions && (
                        <>
                          <Label muted style={styles.cardDescription}>
                            Review or change these permissions in NyxID Web.
                            Return here when finished.
                          </Label>
                          <ActionButton
                            label="Manage permissions in Web"
                            variant="secondary"
                            disabled={busy}
                            onPress={() => void openWeb("overview")}
                          />
                        </>
                      )}
                    </>
                  )}
                </View>
              </Card>
            )}
            <Card>
              <Label style={styles.sectionTitle}>Connection details</Label>
              <ConnectionDetail
                label="Last used"
                value={connectionTime(connection.last_used_at, "Never")}
              />
              {oauth && (
                <ConnectionDetail
                  label="Last authorized"
                  value={connectionTime(
                    connection.last_authorized_at,
                    "Not recorded",
                  )}
                />
              )}
              {connection.expires_at && (
                <ConnectionDetail
                  label="Credential expires"
                  value={connectionTime(connection.expires_at, "Not recorded")}
                />
              )}
              <ActionButton
                label="Refresh status"
                variant="secondary"
                disabled={busy || detail.isFetching}
                onPress={() => void run(reload)}
              />
            </Card>
            {(entry?.documentation_url ?? connection.docs_url) && (
              <ActionButton
                label="Read provider documentation"
                variant="secondary"
                onPress={() =>
                  void run(async () => {
                    await Linking.openURL(
                      validateBrowserUrl(
                        entry?.documentation_url ?? connection.docs_url!,
                      ),
                    );
                  })
                }
                disabled={busy}
              />
            )}
          </>
        )}
        {connection && managementTab === "advanced" && (
          <>
            <Card>
              <Label style={styles.sectionTitle}>Developer access</Label>
              <Label muted style={styles.cardDescription}>
                Use the NyxID proxy address when connecting an external app or
                client. Requests also need a NyxID Agent Key with access to this
                service.
              </Label>
              {connection.proxy_url ? (
                <>
                  <Label selectable mono muted style={styles.cardMetadata}>
                    {connection.proxy_url}
                  </Label>
                  <ActionButton
                    label="Copy proxy address"
                    variant="secondary"
                    disabled={busy}
                    onPress={() =>
                      void run(async () => {
                        await Clipboard.setStringAsync(connection.proxy_url!);
                        setNotice("Proxy address copied.");
                      })
                    }
                  />
                </>
              ) : (
                <Label muted style={styles.cardDescription}>
                  No proxy address was returned for this connection.
                </Label>
              )}
            </Card>
            <Card>
              <Label style={styles.sectionTitle}>More settings in Web</Label>
              <Label muted style={styles.cardDescription}>
                Configure routing, endpoints, request headers and other service
                settings in NyxID Web. These settings are not edited in the
                mobile app yet.
              </Label>
              <ActionButton
                label="Open advanced settings in Web"
                variant="secondary"
                disabled={busy}
                onPress={() => void openWeb("advanced")}
              />
            </Card>
            {replaceable && (
              <Card>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Replace API key or credential"
                  accessibilityState={{ expanded: credentialEditorOpen }}
                  disabled={busy}
                  onPress={() => {
                    setCredentialEditorOpen((value) => !value);
                    setReplacement("");
                  }}
                  style={styles.statusRow}
                >
                  <View style={styles.flex}>
                    <Label style={styles.sectionTitle}>
                      Replace credential
                    </Label>
                    <Label muted style={styles.cardDescription}>
                      Update the saved API key if it was rotated or stopped
                      working.
                    </Label>
                  </View>
                  {credentialEditorOpen ? (
                    <ChevronDown size={18} color={colors.textMuted} />
                  ) : (
                    <ChevronRight size={18} color={colors.textMuted} />
                  )}
                </Pressable>
                {credentialEditorOpen && (
                  <>
                    <Field
                      label="New credential"
                      value={focused && foreground ? replacement : ""}
                      onChangeText={setReplacement}
                      secureTextEntry
                      autoCapitalize="none"
                      autoCorrect={false}
                      editable={!busy}
                      placeholder="Enter a new credential"
                    />
                    <ActionButton
                      label="Save new credential"
                      disabled={busy || !replacement.trim()}
                      onPress={() =>
                        void run(async () => {
                          await servicesApi.replaceCredential(
                            connection.id,
                            replacement.trim(),
                          );
                          setReplacement("");
                          setCredentialEditorOpen(false);
                          await reload();
                          setNotice("Credential replaced.");
                        })
                      }
                    />
                  </>
                )}
              </Card>
            )}
            {editable && (
              <Card>
                <Label style={styles.sectionTitle}>Remove connection</Label>
                <Label muted style={styles.cardDescription}>
                  Agents using this connection will lose access. To pause use
                  temporarily, turn off Connection enabled in Overview instead.
                </Label>
                <Label muted style={styles.cardDescription}>
                  {revokesGrant
                    ? "Removal may also revoke the provider authorization. If it is shared, you can keep it and remove only this connection."
                    : "This removes the saved connection from NyxID. Where supported, NyxID also revokes its saved token at the provider."}
                </Label>
                {confirmRemove ? (
                  <>
                    <Notice
                      message={
                        tokenOnlyRemoval
                          ? "Only this connection will be removed. Provider authorization and other connections will be kept."
                          : `Remove “${connection.label}”? Agents using it will lose access.`
                      }
                    />
                    <ActionButton
                      label={
                        tokenOnlyRemoval
                          ? "Remove only this service"
                          : "Confirm removal"
                      }
                      variant="danger"
                      loading={busy}
                      onPress={() => void run(() => remove(connection))}
                    />
                    <ActionButton
                      label="Cancel"
                      variant="secondary"
                      disabled={busy}
                      onPress={() => {
                        setConfirmRemove(false);
                        setTokenOnlyRemoval(false);
                        setError(undefined);
                      }}
                    />
                  </>
                ) : (
                  <ActionButton
                    label="Remove connection"
                    variant="danger"
                    disabled={busy}
                    onPress={() => setConfirmRemove(true)}
                  />
                )}
              </Card>
            )}
          </>
        )}
      </WorkspacePage>
    );
  }

  if (initialCatalogSlug && !launchedCatalog.current) {
    return (
      <WorkspacePage showHeading={false} title="Connect service">
        {catalog.isPending ? (
          <ActivityIndicator
            accessibilityLabel="Loading service"
            color={colors.primary}
          />
        ) : (
          <Notice
            message={
              catalog.isError
                ? messageOf(catalog.error)
                : "This service is no longer available in the catalog."
            }
            tone="error"
          />
        )}
        <ActionButton
          label="Retry"
          variant="secondary"
          onPress={() => void catalog.refetch()}
          disabled={catalog.isFetching}
        />
      </WorkspacePage>
    );
  }

  const query = search.trim().toLowerCase();
  const filteredConnections = (connections.data ?? []).filter((item) =>
    `${item.label} ${item.slug} ${item.catalog_service_name ?? ""} ${connectionOwner(item)}`
      .toLowerCase()
      .includes(query),
  );
  const filteredCatalog = (catalog.data ?? []).filter((item) =>
    `${item.name} ${item.slug} ${item.description ?? ""}`
      .toLowerCase()
      .includes(query),
  );
  const current = tab === "connected" ? connections : catalog;
  return (
    <WorkspacePage title="Services" showHeading={false}>
      {notice && <Notice message={notice} tone="success" />}
      {error && <Notice message={error} tone="error" />}
      <View style={styles.search}>
        <Text style={[styles.searchLabel, { color: colors.textSecondary }]}>
          Search services
        </Text>
        <TextInput
          accessibilityLabel="Search services"
          value={search}
          onChangeText={setSearch}
          placeholder="Search by name or service"
          placeholderTextColor={colors.textMuted}
          selectionColor={colors.primary}
          autoCapitalize="none"
          autoCorrect={false}
          clearButtonMode="while-editing"
          returnKeyType="search"
          onSubmitEditing={Keyboard.dismiss}
          style={[
            styles.searchInput,
            {
              color: colors.textPrimary,
              borderColor: colors.border,
              backgroundColor: colors.cardSoft,
            },
          ]}
        />
      </View>
      <View
        style={[
          styles.tabs,
          { backgroundColor: colors.cardSoft, borderColor: colors.border },
        ]}
      >
        {(["connected", "catalog"] as const).map((value) => (
          <Pressable
            key={value}
            onPress={() => {
              setTab(value);
              setError(undefined);
            }}
            accessibilityRole="tab"
            hitSlop={{ top: 2, bottom: 2 }}
            accessibilityState={{ selected: tab === value }}
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
              {value === "connected" ? "Connected" : "Explore catalog"}
            </Text>
          </Pressable>
        ))}
      </View>
      {current.isPending && (
        <ActivityIndicator
          accessibilityLabel="Loading services"
          color={colors.primary}
        />
      )}
      {current.isError && (
        <>
          <Notice message={messageOf(current.error)} tone="error" />
          <ActionButton
            label="Retry loading"
            onPress={() => void current.refetch()}
          />
        </>
      )}
      <View style={styles.serviceList}>
        <Text
          accessibilityRole="header"
          style={[styles.sectionTitle, { color: colors.textPrimary }]}
        >
          {tab === "connected" ? "Connected services" : "Available to add"}
        </Text>
        {tab === "connected" &&
          filteredConnections.map((connection) => (
            <Pressable
              key={connection.id}
              accessibilityRole="button"
              accessibilityLabel={`Manage ${connection.label}`}
              onPress={() => {
                navigation.push("ServiceSetup", {
                  connectionId: connection.id,
                });
              }}
            >
              <Card style={styles.serviceCard}>
                <Text
                  style={[
                    styles.connectionStatus,
                    {
                      color:
                        connection.is_active && connection.status === "active"
                          ? colors.success
                          : colors.textSecondary,
                    },
                  ]}
                >
                  ●{" "}
                  {connection.is_active
                    ? connection.status.replaceAll("_", " ")
                    : "Disabled"}
                </Text>
                <View style={styles.row}>
                  <View
                    style={[styles.icon, { backgroundColor: colors.ghostBg }]}
                  >
                    <ServiceIcon
                      slug={connection.catalog_service_slug ?? connection.slug}
                      iconUrl={
                        connection.icon_url || connection.catalog_icon_url
                      }
                      size="md"
                    />
                  </View>
                  <View style={styles.flex}>
                    <Text style={[styles.title, { color: colors.textPrimary }]}>
                      {connection.label}
                    </Text>
                    <Label muted style={styles.cardDescription}>
                      {connectionOwner(connection)}
                    </Label>
                    <Label mono muted style={styles.cardMetadata}>
                      {connection.slug}
                    </Label>
                  </View>
                  <ChevronRight color={colors.textMuted} size={18} />
                </View>
              </Card>
            </Pressable>
          ))}
        {tab === "catalog" &&
          filteredCatalog.map((entry) => (
            <Pressable
              key={entry.slug}
              accessibilityRole="button"
              accessibilityLabel={`Connect ${entry.name}`}
              onPress={() => {
                navigation.push("ServiceSetup", { catalogSlug: entry.slug });
              }}
            >
              <Card style={styles.serviceCard}>
                <Text
                  style={[
                    styles.connectionStatus,
                    { color: colors.primaryOnTint },
                  ]}
                >
                  ●{" "}
                  {entry.platform_key?.available
                    ? "Platform key available"
                    : "Connect"}
                </Text>
                <View style={styles.row}>
                  <View
                    style={[styles.icon, { backgroundColor: colors.ghostBg }]}
                  >
                    <ServiceIcon
                      slug={entry.slug}
                      iconUrl={entry.icon_url}
                      size="md"
                    />
                  </View>
                  <View style={styles.flex}>
                    <Text style={[styles.title, { color: colors.textPrimary }]}>
                      {entry.name}
                    </Text>
                    <Label muted style={styles.cardDescription}>
                      {entry.description ?? entry.slug}
                    </Label>
                  </View>
                  <ChevronRight color={colors.textMuted} size={18} />
                </View>
              </Card>
            </Pressable>
          ))}
      </View>
      {!current.isPending &&
        !current.isError &&
        !(tab === "connected"
          ? filteredConnections.length
          : filteredCatalog.length) && (
          <Card>
            <Label>
              {search
                ? "No matching services"
                : tab === "connected"
                  ? "Connect your first service"
                  : "No services available"}
            </Label>
            <Label muted>
              {search
                ? "Try another name or clear your search."
                : "Explore the catalog to add a connection."}
            </Label>
            {tab === "connected" && (
              <ActionButton
                label="Explore catalog"
                onPress={() => {
                  setTab("catalog");
                  setSearch("");
                }}
              />
            )}
          </Card>
        )}
      <ActionButton
        label="Refresh services"
        variant="secondary"
        disabled={current.isFetching}
        onPress={() => void reload()}
      />
    </WorkspacePage>
  );
}

function ConnectForm({
  entry,
  busy,
  error,
  onRefresh,
  onSubmit,
  onDirty,
}: {
  entry: CatalogEntry;
  onDirty: (dirty: boolean) => void;
  busy: boolean;
  error?: string;
  onRefresh: () => Promise<void>;
  onSubmit: (values: {
    label: string;
    credential: string;
    endpointUrl: string;
    fields: Record<string, string>;
    platform: boolean;
  }) => void;
}) {
  const { colors } = useTheme();
  const [label, setLabel] = useState(entry.name);
  const [credential, setCredential] = useState("");
  const [endpointUrl, setEndpointUrl] = useState("");
  const [fields, setFields] = useState<Record<string, string>>({});
  const [platform, setPlatform] = useState(false);
  const dirty =
    label !== entry.name ||
    Boolean(credential || endpointUrl || platform) ||
    Object.values(fields).some(Boolean);
  useEffect(() => {
    onDirty(dirty);
    return () => onDirty(false);
  }, [dirty, onDirty]);
  const [linkError, setLinkError] = useState<string>();
  const [linkBusy, setLinkBusy] = useState(false);
  const linkLock = useRef(false);
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  useEffect(() => {
    if (!focused) {
      setCredential("");
      setFields({});
    }
  }, [focused]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state !== "active") {
        setCredential("");
        setFields({});
      }
    });
    return () => subscription.remove();
  }, []);
  const supported =
    platform || supportsCredentialForm(entry) || supportsManagedOAuth(entry);
  const oauth = !platform && supportsManagedOAuth(entry);
  async function openWebSetup() {
    if (linkLock.current || busy) return;
    linkLock.current = true;
    setLinkBusy(true);
    setLinkError(undefined);
    try {
      const url = new URL("/keys", validateBrowserUrl(NYXID_FRONTEND_URL));
      url.searchParams.set("slug", entry.slug);
      await WebBrowser.openBrowserAsync(url.href);
      await onRefresh();
    } catch (cause) {
      setLinkError(messageOf(cause));
    } finally {
      linkLock.current = false;
      setLinkBusy(false);
    }
  }
  return (
    <WorkspacePage title={`Connect ${entry.name}`} showHeading={false}>
      {error && <Notice message={error} tone="error" />}
      {linkError && <Notice message={linkError} tone="error" />}
      <ServiceIcon slug={entry.slug} iconUrl={entry.icon_url} size="lg" />
      {entry.description && <Label muted>{entry.description}</Label>}
      <Card>
        <Label style={styles.sectionTitle}>Owner</Label>
        <Label>Personal</Label>
        <Label muted style={styles.cardDescription}>
          This connection will be available within your personal account.
        </Label>
      </Card>
      <Card>
        <Field
          label="Connection name"
          value={label}
          onChangeText={setLabel}
          editable={!busy}
          maxLength={200}
        />
        {entry.platform_key?.available && (
          <View style={styles.statusRow}>
            <View style={styles.flex}>
              <Label>Use platform key</Label>
              <Label muted>
                Charges and service access follow the current platform policy.
              </Label>
            </View>
            <Switch
              accessibilityLabel="Use platform key"
              value={platform}
              onValueChange={(value) => {
                setPlatform(value);
                if (value) {
                  setCredential("");
                  setFields({});
                }
              }}
              disabled={busy}
              trackColor={{ true: colors.primary }}
            />
          </View>
        )}
        {!platform && entry.requires_gateway_url && (
          <Field
            label="Gateway URL"
            value={endpointUrl}
            onChangeText={setEndpointUrl}
            editable={!busy}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            placeholder="https://your-service.example"
          />
        )}
        {!platform &&
          supportsCredentialForm(entry) &&
          entry.token_exchange_credential_fields?.map((field) => (
            <Field
              key={field.name}
              label={field.label}
              value={focused && foreground ? (fields[field.name] ?? "") : ""}
              onChangeText={(value) =>
                setFields((previous) => ({ ...previous, [field.name]: value }))
              }
              secureTextEntry={field.secret}
              placeholder={field.placeholder ?? undefined}
              editable={!busy}
              autoCorrect={false}
              autoCapitalize="none"
            />
          ))}
        {!platform &&
          supportsCredentialForm(entry) &&
          !entry.token_exchange_credential_fields?.length &&
          entry.requires_credential && (
            <Field
              label="API key / credential"
              value={focused && foreground ? credential : ""}
              onChangeText={setCredential}
              secureTextEntry
              editable={!busy}
              autoCapitalize="none"
              autoCorrect={false}
              placeholder="Paste your service credential"
            />
          )}
        {!platform && entry.api_key_instructions && (
          <Label muted>{entry.api_key_instructions}</Label>
        )}
        {oauth && (
          <Notice message="Continue in your browser to authorize this connection. Return here to check its status." />
        )}
        {!supported ? (
          <>
            <Notice message="Continue in Web to complete this service's connection setup. Return here to refresh your connections." />
            <ActionButton
              label="Open Web setup"
              variant="secondary"
              loading={linkBusy}
              disabled={busy || !focused || !foreground}
              onPress={() => void openWebSetup()}
            />
            <ActionButton
              label="Refresh connections"
              variant="secondary"
              disabled={busy || linkBusy}
              onPress={() => {
                if (linkLock.current) return;
                linkLock.current = true;
                setLinkBusy(true);
                setLinkError(undefined);
                void onRefresh()
                  .catch((cause) => setLinkError(messageOf(cause)))
                  .finally(() => {
                    linkLock.current = false;
                    setLinkBusy(false);
                  });
              }}
            />
          </>
        ) : (
          <ActionButton
            label={oauth ? "Continue to authorization" : "Add connection"}
            loading={busy}
            disabled={!supported || !label.trim() || !focused || !foreground}
            onPress={() =>
              onSubmit({ label, credential, endpointUrl, fields, platform })
            }
          />
        )}
      </Card>
      {entry.api_key_url && (
        <ActionButton
          label="Get a credential"
          variant="secondary"
          disabled={busy}
          onPress={() => {
            setLinkError(undefined);
            void Promise.resolve()
              .then(() =>
                Linking.openURL(validateBrowserUrl(entry.api_key_url!)),
              )
              .catch((cause) => setLinkError(messageOf(cause)));
          }}
        />
      )}
    </WorkspacePage>
  );
}

function connectionTime(value: string | null | undefined, fallback: string) {
  if (!value) return fallback;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? fallback : date.toLocaleString();
}

function ConnectionDetail({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.detailRow}>
      <Label muted style={styles.cardMetadata}>
        {label}
      </Label>
      <Label selectable style={[styles.cardMetadata, styles.detailValue]}>
        {value}
      </Label>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  row: { flexDirection: "row", alignItems: "flex-start", gap: 10 },
  icon: {
    width: 34,
    height: 34,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 10,
  },
  title: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 20 },
  serviceList: { gap: 12 },
  sectionTitle: {
    fontFamily: fonts.bodySemi,
    fontSize: 13,
    lineHeight: 18,
    marginBottom: 4,
  },
  serviceCard: { minHeight: 114, borderRadius: 13, gap: 8 },
  connectionStatus: {
    fontFamily: fonts.bodySemi,
    fontSize: 10,
    lineHeight: 14,
    textTransform: "capitalize",
  },
  cardDescription: {
    fontFamily: fonts.bodyRegular,
    fontSize: 12,
    lineHeight: 19,
  },
  cardMetadata: { fontSize: 11, lineHeight: 16 },
  status: { fontFamily: fonts.mono, fontSize: 12, textTransform: "capitalize" },
  statusRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: 14,
  },
  tabs: {
    flexDirection: "row",
    padding: 3,
    minHeight: 48,
    borderRadius: 10,
    borderWidth: 1,
  },
  tab: {
    flex: 1,
    minHeight: 40,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 7,
  },
  tabLabel: { fontFamily: fonts.bodySemi, fontSize: 12, lineHeight: 18 },
  managementTab: { minHeight: 44 },
  permissionsCard: { padding: 0, gap: 0 },
  permissionsToggle: {
    minHeight: 48,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  permissionsContent: { gap: 12, paddingHorizontal: 14, paddingBottom: 14 },
  scopeItem: { gap: 4 },
  detailRow: { flexDirection: "row", justifyContent: "space-between", gap: 14 },
  detailValue: { flex: 1, textAlign: "right" },
  search: { gap: 8 },
  searchLabel: { fontFamily: fonts.bodySemi, fontSize: 12, lineHeight: 16 },
  searchInput: {
    minHeight: 48,
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 14,
    paddingVertical: 12,
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
  },
});
