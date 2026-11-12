import { ServiceIcon } from "../../components/ServiceIcon";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  AppState,
  Keyboard,
  Linking,
  StyleSheet,
  Switch,
  TextInput,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as WebBrowser from "expo-web-browser";
import * as Clipboard from "expo-clipboard";
import { ChevronRight, ChevronDown } from "lucide-react-native";
import {
  SettingsGroup,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { ApiError } from "../../lib/api/ApiError";
import { servicesApi } from "../../lib/api/servicesApi";
import { getFrontendUrl } from "../../lib/env";
import type { WorkspaceNavigationProp } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { SegmentControl } from "../../components/SegmentControl";
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
  isSavedCredential,
  isOAuthConnection,
  reconnectScopeOverride,
} from "./connectionPresentation";
import { PluginConnectionHistory } from "../plugins/PluginConnectionHistory";

const ROOT = ["workspace", "services"] as const;
const MANAGEMENT_TABS = ["overview", "advanced", "history"] as const;
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
          validateBrowserUrl(getFrontendUrl()),
        );
        url.searchParams.set("tab", target);
        await WebBrowser.openBrowserAsync(url.href);
        await reload();
      });
    // Removal is irreversible, so it is confirmed in a system alert. A shared
    // provider grant (11500) switches the next confirmation to token-only.
    const confirmRemoval = (target: ServiceConnection) =>
      Alert.alert(
        tokenOnlyRemoval
          ? "Remove only this service?"
          : `Remove “${target.label}”?`,
        tokenOnlyRemoval
          ? "Only this connection will be removed. Provider authorization and other connections will be kept."
          : "Agents using it will lose access.",
        [
          {
            text: "Cancel",
            style: "cancel",
            onPress: () => {
              setTokenOnlyRemoval(false);
              setError(undefined);
            },
          },
          {
            text: tokenOnlyRemoval ? "Remove only this service" : "Remove",
            style: "destructive",
            onPress: () => void run(() => remove(target)),
          },
        ],
      );
    const statusText =
      connection && health
        ? !connection.is_active
          ? "Paused"
          : health.tone === "success" && !isSavedCredential(connection)
            ? "Connected"
            : health.label
        : undefined;
    const healthText =
      connection && health
        ? !connection.is_active && health.tone !== "success"
          ? `${health.label}. ${health.description}`
          : health.description
        : undefined;
    const chatGuidance = !connection
      ? undefined
      : restricted
        ? "You can view this organization connection, but you cannot use it yet. Ask an organization admin to grant you access."
        : !connection.is_active
          ? editable
            ? "This connection is paused. Turn on Connection enabled above before using it in Chat."
            : "This connection is paused. Ask its administrator to enable it before using it in Chat."
          : isSavedCredential(connection) && connection.status === "active" && !connection.credential_missing
              && connection.credential_verification?.state !== "verified" && connection.credential_verification?.state !== "rejected"
            ? "You can try this saved connection in Chat. Provider authentication has not been verified, and the agent still needs permission to use it."
          : health?.tone === "error" || health?.tone === "warning"
            ? "Resolve the connection issue above before using it in Chat. The agent must also have permission to use this connection."
            : `Ask your agent to use ${providerName}. For a specialist, first select this connection in Agents & Crews → choose the agent → Service permissions, then save.`;
    return (
      <WorkspacePage
        title="Manage connection"
        showHeading={false}
        contentStyle={styles.grouped}
      >
        {connection && health && (
          <SettingsGroup
            footer={(entry?.description ?? connection.description) || undefined}
          >
            <SettingsRow
              leading={
                <ServiceIcon
                  slug={connection.catalog_service_slug ?? connection.slug}
                  iconUrl={connection.icon_url || connection.catalog_icon_url}
                  size="md"
                />
              }
              label={connection.label}
              detail={`${connectionOwner(connection)} · ${credentialKind(connection)}`}
            />
          </SettingsGroup>
        )}
        <SegmentControl
          segments={MANAGEMENT_TABS.map((value) => ({
            label: value.charAt(0).toUpperCase() + value.slice(1),
          }))}
          activeIndex={MANAGEMENT_TABS.indexOf(managementTab)}
          disabled={busy}
          onPress={(index) => {
            const value = MANAGEMENT_TABS[index];
            if (!value) return;
            setManagementTab(value);
            setReplacement("");
            setCredentialEditorOpen(false);
            setTokenOnlyRemoval(false);
          }}
          style={styles.tabs}
        />
        {notice && <Notice message={notice} tone="success" />}
        {error && <Notice message={error} tone="error" />}
        {managementTab !== "history" && detail.isPending && (
          <SkeletonRows
            rows={4}
            icon={false}
            title
            accessibilityLabel="Loading connection"
          />
        )}
        {managementTab !== "history" && detail.isError && (
          <>
            <Notice message={messageOf(detail.error)} tone="error" />
            <SettingsGroup inset={false}>
              <SettingsRow
                label="Retry"
                tone="action"
                onPress={() => void detail.refetch()}
              />
            </SettingsGroup>
          </>
        )}
        {connection && detailCatalog.isError && managementTab !== "history" && (
          <>
            <Notice
              message="Service setup details could not be loaded. Reload them to check available authorization and credential actions."
              tone="error"
            />
            <SettingsGroup inset={false}>
              <SettingsRow
                label="Reload service details"
                tone="action"
                onPress={() => void detailCatalog.refetch()}
              />
            </SettingsGroup>
          </>
        )}
        {managementTab === "history" && (
          <PluginConnectionHistory serviceId={selectedId} />
        )}
        {connection && health && managementTab === "overview" && (
          <>
            <SettingsGroup
              title="Status"
              inset={false}
              footer={
                canRecover
                  ? `${healthText} Continue in your browser, then return here to check the result.`
                  : healthText
              }
            >
              <SettingsRow
                label="Status"
                value={statusText}
                valueTone={health.tone === "error" ? "danger" : "default"}
              />
              {canRecover ? (
                <SettingsRow
                  label={
                    health.recovery === "continue"
                      ? "Continue authentication"
                      : "Reconnect"
                  }
                  tone="action"
                  disabled={busy}
                  onPress={() =>
                    entry &&
                    supportsManagedOAuth(entry) &&
                    !connection.credential_missing
                      ? void run(() => authorize(connection, entry, true))
                      : void openWeb("overview")
                  }
                />
              ) : null}
              {editable &&
              health.recovery &&
              !oauth &&
              connection.credential_binding !== "platform" ? (
                <SettingsRow
                  label="Repair connection in Web"
                  tone="action"
                  disabled={busy}
                  onPress={() => void openWeb("overview")}
                />
              ) : null}
            </SettingsGroup>
            {connection.error_message && (
              <Notice message={connection.error_message} tone="error" />
            )}
            <SettingsGroup
              title="Use in Chat"
              inset={false}
              footer={[
                chatGuidance,
                "Turning this off keeps your saved credential. Turn it on again to resume use.",
                !editable
                  ? connection.auto_connected
                    ? "This connection is managed by the platform."
                    : "An organization admin manages this connection. Your role has read-only access."
                  : undefined,
              ]
                .filter(Boolean)
                .join("\n\n")}
            >
              <SettingsRow
                label="Connection enabled"
                detail={
                  connection.is_active
                    ? "On — NyxID may use this connection within the agent’s permissions."
                    : "Off — requests through this connection are paused."
                }
                accessory={
                  <Switch
                    accessibilityLabel="Connection enabled"
                    value={connection.is_active}
                    disabled={
                      !editable ||
                      busy ||
                      (Boolean(connection.credential_missing) &&
                        !connection.is_active)
                    }
                    trackColor={{ true: colors.success }}
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
                }
              />
            </SettingsGroup>
            {showPermissions && (
              <SettingsGroup
                title="Provider permissions"
                inset={false}
                footer={`What ${providerName} allows NyxID to access. This does not decide which agents may use the connection.`}
              >
                <SettingsRow
                  label="Granted permissions"
                  value={
                    scopes.length
                      ? `${scopes.length} granted permission${scopes.length === 1 ? "" : "s"}`
                      : "None listed"
                  }
                  accessibilityLabel="Provider permissions"
                  accessibilityHint="Review the access granted by the provider, separate from agent service permissions."
                  accessibilityState={{ expanded: permissionsOpen }}
                  accessory={
                    permissionsOpen ? (
                      <ChevronDown size={18} color={colors.textMuted} />
                    ) : (
                      <ChevronRight size={18} color={colors.textMuted} />
                    )
                  }
                  onPress={() => setPermissionsOpen((value) => !value)}
                />
                {permissionsOpen && !scopes.length ? (
                  <SettingsRow
                    label="No saved permission list"
                    detail="NyxID has no saved permission list for this connection. This alone does not mean authorization failed."
                  />
                ) : null}
                {permissionsOpen
                  ? scopes.map((scope) => {
                      const info = entry?.scope_catalog?.find(
                        (item) => item.scope === scope,
                      );
                      return info?.label ? (
                        <SettingsRow
                          key={scope}
                          label={info.label}
                          detail={[info.description, scope]
                            .filter(Boolean)
                            .join("\n")}
                        />
                      ) : (
                        <SettingsRow
                          key={scope}
                          label={scope}
                          detail={info?.description ?? undefined}
                          mono
                          selectable
                        />
                      );
                    })
                  : null}
                {permissionsOpen && canEditPermissions ? (
                  <SettingsRow
                    label="Manage permissions in Web"
                    tone="action"
                    accessibilityHint="Review or change these permissions in NyxID Web, then return here."
                    disabled={busy}
                    onPress={() => void openWeb("overview")}
                  />
                ) : null}
              </SettingsGroup>
            )}
            <SettingsGroup title="Details" inset={false}>
              <SettingsRow
                label="Last used"
                value={connectionTime(connection.last_used_at, "Never")}
              />
              {oauth ? (
                <SettingsRow
                  label="Last authorized"
                  value={connectionTime(
                    connection.last_authorized_at,
                    "Not recorded",
                  )}
                />
              ) : null}
              {connection.expires_at ? (
                <SettingsRow
                  label="Credential expires"
                  value={connectionTime(connection.expires_at, "Not recorded")}
                />
              ) : null}
              {isSavedCredential(connection) && connection.credential_verification?.reason_code ? (
                <SettingsRow label="Check result" value={connection.credential_verification.reason_code} />
              ) : null}
              <SettingsRow
                label="Refresh status"
                tone="action"
                disabled={busy || detail.isFetching}
                onPress={() => void run(async () => {
                  if (editable && isSavedCredential(connection) && connection.credential_verification?.can_check)
                    await servicesApi.checkCredential(connection.id);
                  await reload();
                })}
              />
              {(entry?.documentation_url ?? connection.docs_url) ? (
                <SettingsRow
                  label="Read provider documentation"
                  tone="action"
                  disabled={busy}
                  onPress={() =>
                    void run(async () => {
                      await Linking.openURL(
                        validateBrowserUrl(
                          entry?.documentation_url ?? connection.docs_url!,
                        ),
                      );
                    })
                  }
                />
              ) : null}
            </SettingsGroup>
          </>
        )}
        {connection && managementTab === "advanced" && (
          <>
            <SettingsGroup
              title="Developer access"
              inset={false}
              footer="Use the NyxID proxy address when connecting an external app or client. Requests also need a NyxID Agent Key with access to this service."
            >
              {connection.proxy_url ? (
                <SettingsRow
                  label={connection.proxy_url}
                  detail="Proxy address"
                  mono
                  selectable
                />
              ) : (
                <SettingsRow label="Proxy address" value="Not returned" />
              )}
              {connection.proxy_url ? (
                <SettingsRow
                  label="Copy proxy address"
                  tone="action"
                  disabled={busy}
                  onPress={() =>
                    void run(async () => {
                      await Clipboard.setStringAsync(connection.proxy_url!);
                      setNotice("Proxy address copied.");
                    })
                  }
                />
              ) : null}
            </SettingsGroup>
            <SettingsGroup
              title="More settings"
              inset={false}
              footer="Configure routing, endpoints, request headers and other service settings in NyxID Web. These settings are not edited in the mobile app yet."
            >
              <SettingsRow
                label="Open advanced settings in Web"
                tone="action"
                disabled={busy}
                onPress={() => void openWeb("advanced")}
              />
            </SettingsGroup>
            {replaceable && (
              <SettingsGroup
                title="Credential"
                inset={false}
                footer="Update the saved API key if it was rotated or stopped working."
              >
                <SettingsRow
                  label="Replace credential"
                  accessibilityLabel="Replace API key or credential"
                  accessibilityState={{ expanded: credentialEditorOpen }}
                  disabled={busy}
                  accessory={
                    credentialEditorOpen ? (
                      <ChevronDown size={18} color={colors.textMuted} />
                    ) : (
                      <ChevronRight size={18} color={colors.textMuted} />
                    )
                  }
                  onPress={() => {
                    setCredentialEditorOpen((value) => !value);
                    setReplacement("");
                  }}
                />
                {credentialEditorOpen ? (
                  <SettingsTextField
                    accessibilityLabel="New credential"
                    value={focused && foreground ? replacement : ""}
                    onChangeText={setReplacement}
                    secureTextEntry
                    autoCapitalize="none"
                    autoCorrect={false}
                    editable={!busy}
                    placeholder="Enter a new credential"
                  />
                ) : null}
                {credentialEditorOpen ? (
                  <SettingsRow
                    label="Save new credential"
                    tone="action"
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
                ) : null}
              </SettingsGroup>
            )}
            {editable && (
              <SettingsGroup
                inset={false}
                footer={
                  tokenOnlyRemoval
                    ? "Only this connection will be removed. Provider authorization and other connections will be kept."
                    : `Agents using this connection will lose access. To pause use temporarily, turn off Connection enabled in Overview instead.\n\n${
                        revokesGrant
                          ? "Removal may also revoke the provider authorization. If it is shared, you can keep it and remove only this connection."
                          : "This removes the saved connection from NyxID. Where supported, NyxID also revokes its saved token at the provider."
                      }`
                }
              >
                <SettingsRow
                  label={
                    tokenOnlyRemoval
                      ? "Remove only this service"
                      : "Remove connection"
                  }
                  centered
                  destructive
                  disabled={busy}
                  onPress={() => confirmRemoval(connection)}
                />
              </SettingsGroup>
            )}
          </>
        )}
      </WorkspacePage>
    );
  }

  if (initialCatalogSlug && !launchedCatalog.current) {
    return (
      <WorkspacePage
        showHeading={false}
        title="Connect service"
        contentStyle={styles.grouped}
      >
        {catalog.isPending ? (
          <SkeletonRows rows={1} accessibilityLabel="Loading service" />
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
        <SettingsGroup inset={false}>
          <SettingsRow
            label="Retry"
            tone="action"
            onPress={() => void catalog.refetch()}
            disabled={catalog.isFetching}
          />
        </SettingsGroup>
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
  const listed =
    tab === "connected" ? filteredConnections.length : filteredCatalog.length;
  return (
    <WorkspacePage
      title="Services"
      showHeading={false}
      contentStyle={styles.grouped}
    >
      {notice && <Notice message={notice} tone="success" />}
      {error && <Notice message={error} tone="error" />}
      <View style={[styles.search, { backgroundColor: colors.card }]}>
        <TextInput
          accessibilityLabel="Search services"
          value={search}
          onChangeText={setSearch}
          placeholder="Search services"
          placeholderTextColor={colors.textMuted}
          selectionColor={colors.primary}
          autoCapitalize="none"
          autoCorrect={false}
          clearButtonMode="while-editing"
          returnKeyType="search"
          onSubmitEditing={Keyboard.dismiss}
          style={[styles.searchInput, { color: colors.textPrimary }]}
        />
      </View>
      <SegmentControl
        segments={[{ label: "Connected" }, { label: "Explore catalog" }]}
        activeIndex={tab === "connected" ? 0 : 1}
        onPress={(index) => {
          setTab(index === 0 ? "connected" : "catalog");
          setError(undefined);
        }}
        style={styles.tabs}
      />
      {current.isPending && (
        <SkeletonRows rows={5} title accessibilityLabel="Loading services" />
      )}
      {current.isError && (
        <>
          <Notice message={messageOf(current.error)} tone="error" />
          <SettingsGroup inset={false}>
            <SettingsRow
              label="Retry loading"
              tone="action"
              onPress={() => void current.refetch()}
            />
          </SettingsGroup>
        </>
      )}
      {listed ? (
        <SettingsGroup
          title={tab === "connected" ? "Connected services" : "Available to add"}
        >
          {tab === "connected"
            ? filteredConnections.map((connection) => (
                <SettingsRow
                  key={connection.id}
                  leading={
                    <ServiceIcon
                      slug={connection.catalog_service_slug ?? connection.slug}
                      iconUrl={
                        connection.icon_url || connection.catalog_icon_url
                      }
                      size="md"
                    />
                  }
                  label={connection.label}
                  detail={`${connectionOwner(connection)} · ${connection.slug}`}
                  value={
                    connection.is_active
                      ? sentence(connection.status.replaceAll("_", " "))
                      : "Disabled"
                  }
                  accessibilityLabel={`Manage ${connection.label}`}
                  onPress={() => {
                    navigation.push("ServiceSetup", {
                      connectionId: connection.id,
                    });
                  }}
                />
              ))
            : filteredCatalog.map((entry) => (
                <SettingsRow
                  key={entry.slug}
                  leading={
                    <ServiceIcon
                      slug={entry.slug}
                      iconUrl={entry.icon_url}
                      size="md"
                    />
                  }
                  label={entry.name}
                  detail={entry.description ?? entry.slug}
                  detailLines={2}
                  value={
                    entry.platform_key?.available ? "Platform key" : undefined
                  }
                  accessibilityLabel={`Connect ${entry.name}`}
                  onPress={() => {
                    navigation.push("ServiceSetup", { catalogSlug: entry.slug });
                  }}
                />
              ))}
        </SettingsGroup>
      ) : (
        !current.isPending &&
        !current.isError && (
          <SettingsGroup
            inset={false}
            title={
              search
                ? "No matching services"
                : tab === "connected"
                  ? "Connect your first service"
                  : "No services available"
            }
            footer={
              search
                ? "Try another name or clear your search."
                : "Explore the catalog to add a connection."
            }
          >
            {tab === "connected" ? (
              <SettingsRow
                label="Explore catalog"
                tone="action"
                onPress={() => {
                  setTab("catalog");
                  setSearch("");
                }}
              />
            ) : (
              <SettingsRow label="Nothing to show" />
            )}
          </SettingsGroup>
        )
      )}
      <SettingsGroup inset={false}>
        <SettingsRow
          label="Refresh services"
          tone="action"
          disabled={current.isFetching}
          onPress={() => void reload()}
        />
      </SettingsGroup>
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
  const credentialFields =
    !platform && supportsCredentialForm(entry)
      ? (entry.token_exchange_credential_fields ?? [])
      : [];
  const singleCredential =
    !platform &&
    supportsCredentialForm(entry) &&
    !entry.token_exchange_credential_fields?.length &&
    entry.requires_credential;
  const instructions = !platform ? entry.api_key_instructions : undefined;
  async function openWebSetup() {
    if (linkLock.current || busy) return;
    linkLock.current = true;
    setLinkBusy(true);
    setLinkError(undefined);
    try {
      const url = new URL("/keys", validateBrowserUrl(getFrontendUrl()));
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
  const getCredential = entry.api_key_url ? (
    <SettingsRow
      label="Get a credential"
      tone="action"
      disabled={busy}
      onPress={() => {
        setLinkError(undefined);
        void Promise.resolve()
          .then(() => Linking.openURL(validateBrowserUrl(entry.api_key_url!)))
          .catch((cause) => setLinkError(messageOf(cause)));
      }}
    />
  ) : null;
  return (
    <WorkspacePage
      title={`Connect ${entry.name}`}
      showHeading={false}
      contentStyle={styles.grouped}
    >
      {error && <Notice message={error} tone="error" />}
      {linkError && <Notice message={linkError} tone="error" />}
      <SettingsGroup footer={entry.description || undefined}>
        <SettingsRow
          leading={
            <ServiceIcon slug={entry.slug} iconUrl={entry.icon_url} size="md" />
          }
          label={entry.name}
        />
      </SettingsGroup>
      <SettingsGroup
        inset={false}
        footer="This connection will be available within your personal account."
      >
        <SettingsRow label="Owner" value="Personal" />
      </SettingsGroup>
      <SettingsGroup title="Connection name" inset={false}>
        <SettingsTextField
          accessibilityLabel="Connection name"
          value={label}
          onChangeText={setLabel}
          editable={!busy}
          maxLength={200}
          placeholder={entry.name}
        />
      </SettingsGroup>
      {entry.platform_key?.available && (
        <SettingsGroup
          inset={false}
          footer="Charges and service access follow the current platform policy."
        >
          <SettingsRow
            label="Use platform key"
            accessory={
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
                trackColor={{ true: colors.success }}
              />
            }
          />
        </SettingsGroup>
      )}
      {!platform && entry.requires_gateway_url && (
        <SettingsGroup title="Gateway URL" inset={false}>
          <SettingsTextField
            accessibilityLabel="Gateway URL"
            value={endpointUrl}
            onChangeText={setEndpointUrl}
            editable={!busy}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            placeholder="https://your-service.example"
          />
        </SettingsGroup>
      )}
      {credentialFields.map((field, index) => (
        <SettingsGroup
          key={field.name}
          title={field.label}
          inset={false}
          footer={
            index === credentialFields.length - 1
              ? (instructions ?? undefined)
              : undefined
          }
        >
          <SettingsTextField
            accessibilityLabel={field.label}
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
          {index === credentialFields.length - 1 ? getCredential : null}
        </SettingsGroup>
      ))}
      {singleCredential && (
        <SettingsGroup
          title="API key / credential"
          inset={false}
          footer={instructions ?? undefined}
        >
          <SettingsTextField
            accessibilityLabel="API key / credential"
            value={focused && foreground ? credential : ""}
            onChangeText={setCredential}
            secureTextEntry
            editable={!busy}
            autoCapitalize="none"
            autoCorrect={false}
            placeholder="Paste your service credential"
          />
          {getCredential}
        </SettingsGroup>
      )}
      {!credentialFields.length &&
        !singleCredential &&
        (instructions || getCredential) && (
          <SettingsGroup inset={false} footer={instructions ?? undefined}>
            {getCredential ?? (
              <SettingsRow label="Credential" value="Not required here" />
            )}
          </SettingsGroup>
        )}
      {!supported ? (
        <SettingsGroup
          inset={false}
          footer="Continue in Web to complete this service's connection setup. Return here to refresh your connections."
        >
          <SettingsRow
            label="Open Web setup"
            tone="action"
            disabled={busy || linkBusy || !focused || !foreground}
            accessory={
              linkBusy ? <ActivityIndicator color={colors.primary} /> : undefined
            }
            onPress={() => void openWebSetup()}
          />
          <SettingsRow
            label="Refresh connections"
            tone="action"
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
        </SettingsGroup>
      ) : (
        <SettingsGroup
          inset={false}
          footer={
            oauth
              ? "Continue in your browser to authorize this connection. Return here to check its status."
              : undefined
          }
        >
          <SettingsRow
            label={oauth ? "Continue to authorization" : "Add connection"}
            tone="action"
            disabled={
              busy || !supported || !label.trim() || !focused || !foreground
            }
            accessory={
              busy ? <ActivityIndicator color={colors.primary} /> : undefined
            }
            onPress={() =>
              onSubmit({ label, credential, endpointUrl, fields, platform })
            }
          />
        </SettingsGroup>
      )}
    </WorkspacePage>
  );
}

function connectionTime(value: string | null | undefined, fallback: string) {
  if (!value) return fallback;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? fallback : date.toLocaleString();
}

const sentence = (value: string) =>
  value.charAt(0).toUpperCase() + value.slice(1);

function credentialKind(connection: ServiceConnection) {
  return connection.credential_binding === "platform"
    ? "Platform key"
    : connection.credential_type === "none"
      ? "No authentication"
      : connection.credential_type === "oauth2"
        ? "OAuth"
        : connection.credential_type.replaceAll("_", " ");
}

const styles = StyleSheet.create({
  grouped: {
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.sm,
    paddingBottom: spacing.huge * 2,
    gap: spacing.huge,
  },
  tabs: { marginBottom: 0 },
  search: {
    borderRadius: radius.md + 2,
    paddingHorizontal: spacing.md,
  },
  searchInput: {
    minHeight: 40,
    paddingVertical: spacing.sm,
    ...typeScale.label,
  },
});
