import { AgentAvatar } from "../../components/AgentAvatar";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Alert,
  AppState,
  Keyboard,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronRight,
  Search,
  SlidersHorizontal,
  X,
} from "lucide-react-native";
import { ZodError } from "zod";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { SkeletonCards, SkeletonRows } from "../../components/Skeleton";
import { agentsApi } from "../../lib/api/agentsApi";
import {
  AgentCollaborationPanel,
  AgentOperationControls,
} from "./AgentCollaborationPanel";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { servicesApi } from "../../lib/api/servicesApi";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { fonts } from "../../theme/designTokens";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import {
  SettingsGroup,
  SettingsMenuRow,
  SettingsPage,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import {
  agentProfileFieldTitle,
  type AgentProfileField,
} from "./AgentProfileEditScreen";
import {
  AgentPermissionsField,
  type AgentPermissions,
} from "./AgentPermissionsField";
import { useTheme } from "../../theme/ThemeContext";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import type { ServiceConnection } from "../services/serviceContracts";
import {
  agentProfile,
  buildAgentPatch,
  changedGrants,
  grantOptions,
  grantSignature,
  profileHasConflict,
  type AgentCreate,
  type AgentProfile,
  type AssistantAgent,
} from "./agentContracts";

const ROOT = ["workspace", "agents"] as const;
const errorMessage = (cause: unknown) =>
  cause instanceof ZodError
    ? (cause.issues[0]?.message ?? "Check the form fields.")
    : cause instanceof Error
      ? cause.message
      : "Could not complete this action. Try again.";
type OpenChat = (context: {
  agentId: string;
  conversationId?: string;
  openLatest?: boolean;
}) => void;

export function AgentsScreen({
  agentId,
  onOpenChat,
  onOpenSkills,
  onOpenAutomations,
  onOpenMachines,
  onEditProfile,
  onClose,
  listHeader,
  initialCreate = false,
  onCreate,
  onCreated,
}: {
  agentId?: string;
  onOpenChat?: OpenChat;
  onOpenSkills?: (agentId: string) => void;
  onOpenAutomations?: (agentId: string) => void;
  onOpenMachines?: (agentId: string) => void;
  /** Opens the focused edit page for one profile value. */
  onEditProfile?: (agentId: string, field: AgentProfileField) => void;
  onClose?: () => void;
  listHeader?: ReactNode;
  initialCreate?: boolean;
  onCreate?: () => void;
  onCreated?: (agentId: string) => void;
} = {}) {
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [selectedId, setSelectedId] = useState<string | undefined>(agentId);
  const [creating, setCreating] = useState(initialCreate);
  const [creationDirty, setCreationDirty] = useState(false);
  const [createdId, setCreatedId] = useState<string>();
  const [search, setSearch] = useState("");
  const [includeDestroyed, setIncludeDestroyed] = useState(false);
  const [showFilters, setShowFilters] = useState(false);
  const [pullRefreshing, setPullRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [closeRequested, setCloseRequested] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [editorVersion, setEditorVersion] = useState(0);
  const [grantsVersion, setGrantsVersion] = useState(0);
  const lock = useRef(false);
  const refreshLock = useRef(false);
  const list = useQuery({
    queryKey: [...ROOT, "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    retry: false,
    enabled: focused && foreground,
    refetchInterval: (query) =>
      focused &&
      foreground &&
      query.state.data?.agents.some((agent) => agent.status === "running")
        ? 5000
        : false,
  });
  const detail = useQuery({
    queryKey: [...ROOT, "detail", selectedId],
    queryFn: ({ signal }) => agentsApi.detail(selectedId!, signal),
    retry: false,
    enabled: focused && foreground && Boolean(selectedId),
    refetchInterval: (query) =>
      focused && foreground && query.state.data?.agent.status === "running"
        ? 5000
        : false,
  });
  const connections = useQuery({
    queryKey: ["workspace", "services", "connections"],
    queryFn: ({ signal }) => servicesApi.connections(signal),
    enabled: focused && foreground && (creating || Boolean(selectedId)),
    retry: false,
  });
  const creationAtLimit =
    !!list.data &&
    list.data.agents.filter(
      (agent) =>
        agent.owner_kind !== "org" &&
        !agent.share_role &&
        agent.kind === "specialist" &&
        agent.status !== "destroyed",
    ).length >= list.data.limits.max_live_subagents;
  const reload = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ROOT });
  }, [queryClient]);
  const refreshList = useCallback(async () => {
    if (refreshLock.current) return;
    refreshLock.current = true;
    Keyboard.dismiss();
    setPullRefreshing(true);
    try {
      await list.refetch({ cancelRefetch: false });
    } finally {
      refreshLock.current = false;
      setPullRefreshing(false);
    }
  }, [list.refetch]);

  useEffect(() => {
    setSelectedId(agentId);
  }, [agentId]);
  useEffect(() => {
    if (!focused) return;
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state === "active") void reload();
    });
    return () => subscription.remove();
  }, [focused, reload]);
  function leave() {
    if (onClose) {
      setCreationDirty(false);
      setCloseRequested(true);
      return;
    }
    setSelectedId(undefined);
    setCreating(false);
    setCreationDirty(false);
    setError(undefined);
  }
  function back() {
    if (creating && creationDirty) {
      Alert.alert("Discard new agent?", "Your agent has not been created.", [
        { text: "Keep editing", style: "cancel" },
        { text: "Discard", style: "destructive", onPress: leave },
      ]);
    } else leave();
  }
  useWorkspaceBack({
    active: onClose
      ? busy || (creating && creationDirty && !closeRequested)
      : Boolean(selectedId) || creating,
    defaultRole: onClose ? "back" : "menu",
    disabled: busy,
    label: onClose ? "Back" : "Back to agents",
    onBack: () => {
      if (!lock.current) back();
    },
    discardOnClose:
      creating && creationDirty && !closeRequested
        ? {
            title: "Discard new agent?",
            message: "Your agent has not been created.",
          }
        : undefined,
  });
  useEffect(() => {
    if (busy || !closeRequested) return;
    setCloseRequested(false);
    onClose?.();
  }, [busy, closeRequested, onClose]);
  useEffect(() => {
    if (createdId && !busy && !creating && onCreated) {
      setCreatedId(undefined);
      onCreated(createdId);
    }
  }, [createdId, busy, creating, onCreated]);

  async function run(
    action: () => Promise<void>,
    success?: string,
    refreshForm?: "grants",
  ): Promise<boolean> {
    if (lock.current) return false;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await action();
      await reload();
      if (refreshForm === "grants") setGrantsVersion((value) => value + 1);
      if (success) setNotice(success);
      return true;
    } catch (cause) {
      setError(errorMessage(cause));
      return false;
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function refreshEditor() {
    await reload();
    setEditorVersion((value) => value + 1);
    setError(undefined);
    setNotice(undefined);
  }
  // A choice applies at once, with the same live-state and conflict checks
  // as the profile form had before it was split into rows.
  function changeConfirmationMode(
    agent: AssistantAgent,
    mode: AgentProfile["access_mode_override"],
  ) {
    if ((agent.access_mode_override ?? null) === mode) return;
    void run(async () => {
      const patch = buildAgentPatch(agent, {
        ...agentProfile(agent),
        access_mode_override: mode,
      });
      if (!Object.keys(patch).length) return;
      const latest = (await agentsApi.detail(agent.id)).agent;
      if (latest.can_maintain !== true || latest.status === "destroyed")
        throw new Error(
          "This agent can no longer be changed. Reload to review its current state.",
        );
      if (profileHasConflict(agent, latest, patch))
        throw new Error(
          "This profile changed elsewhere. Reload the agent and review those changes before saving.",
        );
      await agentsApi.update(agent.id, patch);
    }, "Confirmation mode saved.");
  }
  function confirmLifecycle(agent: AssistantAgent, destroy: boolean) {
    Alert.alert(
      destroy
        ? `Destroy “${agent.display_name ?? agent.name}”?`
        : "Permanently delete this agent and all conversation history?",
      destroy
        ? "Stops its running threads and disconnects channel bots. Conversation history remains available."
        : "This cannot be undone.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: destroy ? "Destroy" : "Delete permanently",
          style: "destructive",
          onPress: () =>
            void run(
              async () => {
                if (destroy) await agentsApi.destroy(agent.id);
                else {
                  await agentsApi.remove(agent.id);
                  back();
                }
              },
              destroy ? "Agent destroyed." : "Agent deleted.",
            ),
        },
      ],
    );
  }

  if (creating)
    return (
      <WorkspacePage
        showHeading={false}
        title="New agent"
        subtitle="Create a personal specialist"
      >
        {error && <Notice message={error} tone="error" />}
        {!isConnected && (
          <Notice message="You're offline. Reconnect to create your agent. Your draft is kept." />
        )}
        {creationAtLimit && (
          <Notice message="You've reached your personal agent limit. Destroy an unused specialist before creating another." />
        )}
        {list.isError && (
          <Notice
            tone="error"
            message="Agent limits could not be checked. Go back and refresh your agents before creating another."
          />
        )}
        {connections.isError && (
          <Notice
            message="Connections could not be loaded. You can create the agent without service access and grant it later."
            tone="error"
          />
        )}
        <CreateAgentForm
          connections={connections.data ?? []}
          connectionsLoading={connections.isPending}
          connectionsError={connections.isError}
          busy={busy}
          available={
            isConnected &&
            focused &&
            foreground &&
            !list.isPending &&
            !list.isError &&
            !creationAtLimit
          }
          onDirtyChange={setCreationDirty}
          onCreate={(values) => {
            if (
              !isConnected ||
              !focused ||
              !foreground ||
              list.isPending ||
              list.isError ||
              creationAtLimit
            )
              return;
            void run(async () => {
              const created = await agentsApi.create(values);
              setCreationDirty(false);
              setCreating(false);
              setSelectedId(created.id);
              setCreatedId(created.id);
              setEditorVersion((value) => value + 1);
            }, "Agent created.");
          }}
        />
      </WorkspacePage>
    );

  if (selectedId) {
    const agent = detail.data?.agent;
    const maintain = agent?.can_maintain === true;
    const alive = agent && agent.status !== "destroyed";
    const profileEditable = !!agent && maintain && !!alive && !busy;
    const editProfile = (field: AgentProfileField) =>
      agent && profileEditable && onEditProfile
        ? () => onEditProfile(agent.id, field)
        : undefined;
    const name = agent ? agent.display_name || agent.name : "Agent";
    const subtitle = agent?.share_role
      ? `${agent.owner_name ?? "Shared agent"} · shared ${agent.share_role}`
      : agent?.owner_kind === "org"
        ? `${agent.owner_name ?? "Organization"} · ${agent.org_role ?? "member"}`
        : "Personal agent";
    const confirmationMode = agent?.access_mode_override ?? null;
    const memory = detail.data?.memory ?? [];
    const threads = detail.data?.threads ?? [];
    const destinations =
      !!agent &&
      !!alive &&
      ((agent.can_use === true && !!onOpenChat) ||
        (maintain && !agent.share_role && !!(onOpenSkills || onOpenMachines)) ||
        (!agent.share_role && agent.owner_kind !== "org" && !!onOpenAutomations));
    return (
      <SettingsPage
        refreshing={pullRefreshing}
        onRefresh={() => {
          if (busy || detail.isFetching || refreshLock.current) return;
          refreshLock.current = true;
          setPullRefreshing(true);
          void refreshEditor().finally(() => {
            refreshLock.current = false;
            setPullRefreshing(false);
          });
        }}
      >
        {notice && <Notice message={notice} tone="success" />}
        {error && <Notice message={error} tone="error" />}
        {detail.isPending && (
          <SkeletonRows rows={4} title accessibilityLabel="Loading agent" />
        )}
        {detail.isError && (
          <>
            <Notice message={errorMessage(detail.error)} tone="error" />
            <SettingsGroup inset={false}>
              <SettingsRow
                tone="action"
                label="Retry"
                onPress={() => void detail.refetch()}
              />
            </SettingsGroup>
          </>
        )}
        {agent && (
          <>
            <AgentIdentityHeader
              agent={agent}
              title={name}
              subtitle={subtitle}
            />
            {agent.pending_acknowledgements > 0 && (
              <Notice
                message={`${agent.pending_acknowledgements} requests waiting for a decision.`}
              />
            )}
            {agent.share_role && (
              <Notice
                message={
                  agent.share_role === "manager"
                    ? "Shared Manager access can edit this agent's profile and published team memory. Its owner controls permissions and lifecycle."
                    : "Shared User access can use this agent. Its owner controls permissions and lifecycle."
                }
              />
            )}
            {!maintain && !agent.share_role && (
              <Notice message="Your current role can view this agent. A maintainer can change its profile and permissions." />
            )}
            {!alive && (
              <Notice message="This agent is destroyed. Its threads are read only until you delete it permanently." />
            )}
            {destinations && (
              <SettingsGroup inset={false}>
                {alive && agent.can_use === true && onOpenChat && (
                  <SettingsRow
                    tone="action"
                    label="Chat with agent"
                    onPress={() =>
                      onOpenChat({ agentId: agent.id, openLatest: true })
                    }
                  />
                )}
                {alive && maintain && !agent.share_role && onOpenSkills && (
                  <SettingsRow
                    label="Skills"
                    accessibilityLabel="Agent skills"
                    disabled={busy}
                    onPress={() => onOpenSkills(agent.id)}
                  />
                )}
                {alive && maintain && !agent.share_role && onOpenMachines && (
                  <SettingsRow
                    label="Machine access"
                    accessibilityLabel="Agent machine access"
                    disabled={busy}
                    onPress={() => onOpenMachines(agent.id)}
                  />
                )}
                {alive &&
                  !agent.share_role &&
                  agent.owner_kind !== "org" &&
                  onOpenAutomations && (
                    <SettingsRow
                      label="Automations"
                      accessibilityLabel="Agent automations"
                      disabled={busy}
                      onPress={() => onOpenAutomations(agent.id)}
                    />
                  )}
              </SettingsGroup>
            )}
            {!agent.share_role && (
              <SettingsGroup
                inset={false}
                title="Profile"
                footer={
                  confirmationMode === "autopilot"
                    ? "Autopilot may act without asking. Server policy and granted permissions still apply."
                    : "Confirmation mode controls when this agent asks before acting."
                }
              >
                <SettingsRow
                  label="Handle"
                  value={`@${agent.name}`}
                  onPress={
                    agent.kind === "nyxbot" ? undefined : editProfile("name")
                  }
                />
                <SettingsRow
                  label="Display name"
                  value={agent.display_name || "None"}
                  onPress={editProfile("display_name")}
                />
                <SettingsRow
                  label={agentProfileFieldTitle(
                    "description",
                    agent.kind === "nyxbot",
                  )}
                  value={profilePreview(agent.description)}
                  onPress={editProfile("description")}
                />
                <SettingsRow
                  label="Personality and tone"
                  value={profilePreview(agent.persona)}
                  onPress={editProfile("persona")}
                />
                {profileEditable ? (
                  <SettingsMenuRow
                    label="Confirmation mode"
                    value={confirmationMode ?? "workspace"}
                    options={CONFIRMATION_MODES.map((mode) => ({
                      value: (mode ?? "workspace") as ConfirmationChoice,
                      label: CONFIRMATION_MODE_LABEL[mode ?? "workspace"],
                    }))}
                    onChange={(mode) =>
                      changeConfirmationMode(
                        agent,
                        mode === "workspace" ? null : mode,
                      )
                    }
                  />
                ) : (
                  <SettingsRow
                    label="Confirmation mode"
                    value={CONFIRMATION_MODE_LABEL[confirmationMode ?? "workspace"]}
                  />
                )}
              </SettingsGroup>
            )}
            {agent.kind === "specialist" && !agent.share_role && (
              <AgentGrantsForm
                key={`grants:${agent.id}:${editorVersion}:${grantsVersion}`}
                agent={agent}
                connections={connections.data ?? []}
                connectionsLoading={connections.isPending}
                connectionsError={connections.isError}
                saveError={error}
                busy={busy}
                onSave={(baseline, services, accountRead, guests) =>
                  run(
                    async () => {
                      const latest = (await agentsApi.detail(agent.id)).agent;
                      if (
                        latest.can_maintain !== true ||
                        latest.status === "destroyed"
                      )
                        throw new Error(
                          "This agent can no longer be changed. Reload to review its current state.",
                        );
                      if (grantSignature(latest) !== grantSignature(baseline))
                        throw new Error(
                          "The agent's permissions changed elsewhere. Reload and review them before saving.",
                        );
                      await agentsApi.grants(
                        agent.id,
                        changedGrants(baseline, services, accountRead, guests),
                      );
                    },
                    "Permissions saved.",
                    "grants",
                  )
                }
              />
            )}
            <AgentCollaborationPanel
              key={`${getAuthEpoch()}:${agent.id}:sharing:${editorVersion}`}
              agent={agent}
            />
            <AgentOperationControls
              key={`${getAuthEpoch()}:${agent.id}:operations:${editorVersion}`}
              agent={agent}
              onOpenChat={
                onOpenChat
                  ? () => onOpenChat({ agentId: agent.id, openLatest: true })
                  : undefined
              }
            />
            {!agent.share_role && (
              <SettingsGroup
                inset={false}
                title="Memory"
                footer={
                  maintain && alive && memory.length
                    ? "Notes this agent remembers across conversations. Tap a memory to forget it."
                    : "Notes this agent remembers across conversations."
                }
              >
                {memory.map((note) => (
                  <SettingsRow
                    key={note.id}
                    label={note.text}
                    chevron={false}
                    disabled={busy && maintain && !!alive}
                    accessibilityHint={
                      maintain && alive ? "Asks to forget this memory" : undefined
                    }
                    onPress={
                      maintain && alive
                        ? () =>
                            Alert.alert(
                              "Forget this memory?",
                              note.text,
                              [
                                { text: "Keep memory", style: "cancel" },
                                {
                                  text: "Forget",
                                  style: "destructive",
                                  onPress: () =>
                                    void run(async () => {
                                      await agentsApi.forget(agent.id, note.id);
                                    }, "Memory forgotten."),
                                },
                              ],
                            )
                        : undefined
                    }
                  />
                ))}
                {!memory.length && <SettingsRow label="No memories yet" disabled />}
              </SettingsGroup>
            )}
            {!agent.share_role && maintain && alive && (
              <RememberForm
                busy={busy}
                onRemember={(text) =>
                  run(async () => {
                    await agentsApi.remember(agent.id, text);
                  }, "Memory saved.")
                }
              />
            )}
            {!!agent.channels.length && (
              <SettingsGroup inset={false} title="Connected channels">
                {agent.channels.map((channel) => (
                  <SettingsRow
                    key={channel.id}
                    label={channel.bot_label}
                    detail={channel.platform}
                    value={channel.status}
                  />
                ))}
              </SettingsGroup>
            )}
            <SettingsGroup inset={false} title="Recent threads">
              {!threads.length && (
                <SettingsRow label="No conversations yet" disabled />
              )}
              {threads.map((thread) => (
                <SettingsRow
                  key={thread.id}
                  label={thread.title}
                  detail={
                    thread.running
                      ? "Running"
                      : new Date(thread.last_message_at).toLocaleString()
                  }
                  chevron
                  disabled={!onOpenChat || agent.can_use !== true}
                  onPress={() =>
                    onOpenChat?.({
                      agentId: agent.id,
                      conversationId: thread.id,
                    })
                  }
                />
              ))}
            </SettingsGroup>
            {maintain && !agent.share_role && agent.kind === "specialist" && (
              <SettingsGroup
                inset={false}
                footer={
                  alive
                    ? "Stops its running threads and disconnects channel bots. Conversation history remains available."
                    : "Deletes this destroyed agent and all its threads. This cannot be undone."
                }
              >
                <SettingsRow
                  centered
                  destructive
                  label={alive ? "Destroy Agent" : "Delete Permanently"}
                  disabled={busy}
                  onPress={() => confirmLifecycle(agent, !!alive)}
                />
              </SettingsGroup>
            )}
          </>
        )}
      </SettingsPage>
    );
  }

  const filtered = (list.data?.agents ?? []).filter(
    (agent) =>
      (includeDestroyed || agent.status !== "destroyed") &&
      `${agent.name} ${agent.display_name ?? ""} ${agent.description} ${agent.owner_name ?? ""}`
        .toLowerCase()
        .includes(search.trim().toLowerCase()),
  );
  const live =
    list.data?.agents.filter(
      (agent) =>
        agent.owner_kind !== "org" &&
        !agent.share_role &&
        agent.kind === "specialist" &&
        agent.status !== "destroyed",
    ).length ?? 0;
  const atLimit = list.data && live >= list.data.limits.max_live_subagents;
  return (
    <WorkspacePage
      showHeading={false}
      title="Your agents"
      subtitle="Personal & shared agents"
      refreshing={pullRefreshing}
      onRefresh={refreshList}
    >
      {listHeader}
      {notice && <Notice message={notice} tone="success" />}
      {error && <Notice message={error} tone="error" />}
      {atLimit && (
        <Notice message="Your current agent limit is reached. Destroy an unused specialist or update your workspace limit in Settings." />
      )}
      <View style={styles.searchToolbar}>
        <View
          style={[
            styles.searchBox,
            { backgroundColor: colors.cardSoft, borderColor: colors.border },
          ]}
        >
          <Search size={18} color={colors.textMuted} accessible={false} />
          <TextInput
            accessibilityLabel="Search agents"
            value={search}
            onChangeText={setSearch}
            placeholder="Search agents…"
            placeholderTextColor={colors.textMuted}
            selectionColor={colors.primary}
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="search"
            onSubmitEditing={Keyboard.dismiss}
            style={[styles.searchInput, { color: colors.textPrimary }]}
          />
          {search.length > 0 && (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Clear agent search"
              onPress={() => setSearch("")}
              style={styles.clearSearch}
            >
              <X size={17} color={colors.textMuted} />
            </Pressable>
          )}
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Filter agents"
          accessibilityHint={
            includeDestroyed
              ? "Including destroyed agents. Open filters."
              : "Open status filters."
          }
          accessibilityState={{
            expanded: showFilters,
            selected: includeDestroyed,
          }}
          onPress={() => {
            Keyboard.dismiss();
            setShowFilters(true);
          }}
          style={[
            styles.listTool,
            {
              backgroundColor: includeDestroyed
                ? colors.primaryTone.bg
                : colors.cardSoft,
              borderColor: includeDestroyed
                ? colors.primaryTone.border
                : colors.border,
            },
          ]}
        >
          <SlidersHorizontal
            size={18}
            color={
              includeDestroyed ? colors.primaryOnTint : colors.textSecondary
            }
          />
          {includeDestroyed && (
            <View
              style={[styles.filterDot, { backgroundColor: colors.primary }]}
            />
          )}
        </Pressable>
      </View>
      {showFilters && (
        <WorkspaceSelectionSheet
          label="Agent filters"
          title="Filter agents"
          options={[
            {
              value: "destroyed",
              label: "Include destroyed agents",
              description: "Show destroyed agents alongside active agents.",
            },
          ]}
          value={includeDestroyed ? ["destroyed"] : []}
          multiple
          confirmLabel="Apply filters"
          onChange={(selected) =>
            setIncludeDestroyed(selected.includes("destroyed"))
          }
          onClose={() => setShowFilters(false)}
        />
      )}
      {list.isPending && <SkeletonCards accessibilityLabel="Loading agents" />}
      {list.isError && (
        <>
          <Notice message={errorMessage(list.error)} tone="error" />
          <ActionButton label="Retry" onPress={() => void list.refetch()} />
        </>
      )}
      {filtered.map((agent) => (
        <Pressable
          key={agent.id}
          accessibilityRole="button"
          accessibilityLabel={`Manage ${agent.display_name ?? agent.name}`}
          onPress={() => {
            Keyboard.dismiss();
            setSelectedId(agent.id);
            setError(undefined);
            setNotice(undefined);
            setEditorVersion((value) => value + 1);
          }}
        >
          <Card>
            <View style={styles.row}>
              <AgentAvatar agent={agent} size="xl" />
              <View style={styles.flex}>
                <Text style={[styles.title, { color: colors.textPrimary }]}>
                  {agent.display_name ?? agent.name}
                </Text>
                <Label muted numberOfLines={2}>
                  {agent.description || "Your personal agent"}
                </Label>
              </View>
              <ChevronRight size={18} color={colors.textMuted} />
            </View>
            <Label muted style={{ fontSize: 11 }}>
              {agent.status === "running"
                ? "● Working"
                : agent.status === "destroyed"
                  ? "Destroyed"
                  : "● Ready"}
              {agent.owner_kind === "org"
                ? ` · ${agent.owner_name ?? "Organization"}`
                : " · Personal"}
              {agent.pending_acknowledgements
                ? ` · ${agent.pending_acknowledgements} pending`
                : ""}
            </Label>
          </Card>
        </Pressable>
      ))}
      {!list.isPending && !list.isError && !filtered.length && (
        <Card>
          <Label>No matching agents</Label>
          <Label muted>Change your search or create a new specialist.</Label>
        </Card>
      )}
      <ActionButton
        label="Create an agent"
        disabled={
          !isConnected ||
          !foreground ||
          !focused ||
          Boolean(atLimit) ||
          list.isPending ||
          list.isError
        }
        onPress={() => {
          Keyboard.dismiss();
          if (onCreate) onCreate();
          else setCreating(true);
          setError(undefined);
          setNotice(undefined);
        }}
      />
      {list.data && (
        <Label muted style={{ fontSize: 11 }}>
          {live} / {list.data.limits.max_live_subagents} personal specialists
        </Label>
      )}
    </WorkspacePage>
  );
}

function ProfileFields({
  value,
  onChange,
  disabled,
  nyxbot,
}: {
  value: AgentProfile;
  onChange: (patch: Partial<AgentProfile>) => void;
  disabled: boolean;
  nyxbot?: boolean;
}) {
  return (
    <>
      <Field
        label="Handle"
        value={value.name}
        onChangeText={(name) => onChange({ name: name.toLowerCase() })}
        editable={!disabled && !nyxbot}
        autoCapitalize="none"
        autoCorrect={false}
        maxLength={32}
        placeholder="researcher"
      />
      <Field
        label="Display name"
        value={value.display_name}
        onChangeText={(display_name) => onChange({ display_name })}
        editable={!disabled}
        maxLength={40}
        placeholder="Optional friendly name"
      />
      <Field
        label={nyxbot ? "Notes" : "Purpose"}
        value={value.description}
        onChangeText={(description) => onChange({ description })}
        editable={!disabled}
        multiline
        maxLength={2048}
        placeholder="What should this agent do?"
      />
      <Field
        label="Personality and tone"
        value={value.persona}
        onChangeText={(persona) => onChange({ persona })}
        editable={!disabled}
        multiline
        maxLength={2000}
        placeholder="Warm, concise, explains its decisions"
      />
    </>
  );
}

function CreateAgentForm({
  connections,
  connectionsLoading,
  connectionsError,
  busy,
  onCreate,
  available,
  onDirtyChange,
}: {
  connections: ServiceConnection[];
  connectionsLoading: boolean;
  connectionsError: boolean;
  busy: boolean;
  onCreate: (values: AgentCreate) => void;
  available: boolean;
  onDirtyChange: (dirty: boolean) => void;
}) {
  const [value, setValue] = useState<AgentCreate>({
    name: "",
    display_name: "",
    description: "",
    persona: "",
    services: [],
    account_read: false,
  });
  useEffect(() => {
    onDirtyChange(
      Boolean(
        value.name ||
          value.display_name ||
          value.description ||
          value.persona ||
          value.services.length ||
          value.account_read,
      ),
    );
  }, [value, onDirtyChange]);
  return (
    <>
      <Card>
        <ProfileFields
          value={value}
          onChange={(patch) => setValue((old) => ({ ...old, ...patch }))}
          disabled={busy}
        />
        <Label muted>
          A specialist uses only the services you grant. Personality changes its
          tone, not its authority.
        </Label>
      </Card>
      <AgentPermissionsField
        title="Starting permissions"
        options={grantOptions(connections)}
        value={{
          services: value.services,
          account_read: value.account_read,
          guest_access: {},
        }}
        disabled={busy}
        allowGuestAccess={false}
        connectionsLoading={connectionsLoading}
        connectionsError={connectionsError}
        creating
        onSave={async (permissions) => {
          setValue((old) => ({
            ...old,
            services: permissions.services,
            account_read: permissions.account_read,
          }));
          return true;
        }}
      />
      <ActionButton
        label="Create agent"
        loading={busy}
        disabled={!available || !value.name.trim() || !value.description.trim()}
        onPress={() => onCreate(value)}
      />
    </>
  );
}

function AgentGrantsForm({
  agent,
  connections,
  connectionsLoading,
  connectionsError,
  saveError,
  busy,
  onSave,
}: {
  agent: AssistantAgent;
  connections: ServiceConnection[];
  connectionsLoading: boolean;
  connectionsError: boolean;
  saveError?: string;
  busy: boolean;
  onSave: (
    baseline: AssistantAgent,
    services: string[],
    accountRead: boolean,
    guests: AgentPermissions["guest_access"],
  ) => Promise<boolean>;
}) {
  const baseline = useRef(agent).current;
  return (
    <AgentPermissionsField
      options={grantOptions(connections, agent).map((option) => ({
        ...option,
        detail:
          agent.owner_kind === "org"
            ? connections.find((connection) => connection.id === option.value)
                ?.slug
            : option.value,
      }))}
      value={baseline}
      disabled={
        busy || agent.can_maintain !== true || agent.status === "destroyed"
      }
      readOnly={agent.can_maintain !== true || agent.status === "destroyed"}
      allowAccountRead={agent.owner_kind !== "org"}
      allowGuestAccess={agent.owner_kind !== "org"}
      connectionsLoading={connectionsLoading}
      connectionsError={connectionsError}
      saveError={saveError}
      onSave={(permissions) =>
        onSave(
          baseline,
          permissions.services,
          permissions.account_read,
          permissions.guest_access,
        )
      }
    />
  );
}

function RememberForm({
  busy,
  onRemember,
}: {
  busy: boolean;
  onRemember: (text: string) => Promise<boolean>;
}) {
  const [text, setText] = useState("");
  return (
    <SettingsGroup inset={false} title="Add a memory">
      <SettingsTextField
        accessibilityLabel="Add a memory"
        value={text}
        onChangeText={setText}
        multiline
        clearButtonMode="never"
        editable={!busy}
        maxLength={2000}
        placeholder="Something this agent should remember"
        style={styles.memoryField}
      />
      <SettingsRow
        tone="action"
        label="Remember"
        disabled={busy || !text.trim()}
        onPress={() => {
          void onRemember(text.trim()).then((saved) => {
            if (saved) setText("");
          });
        }}
      />
    </SettingsGroup>
  );
}

const CONFIRMATION_MODES = [
  null,
  "nyxmode",
  "ask_every_time",
  "autopilot",
] as const;
const CONFIRMATION_MODE_LABEL = {
  workspace: "Follow workspace",
  nyxmode: "NyxMode · sensitive actions",
  ask_every_time: "Ask · every change",
  autopilot: "Autopilot",
} as const;
type ConfirmationChoice = keyof typeof CONFIRMATION_MODE_LABEL;

/** One line of a long profile value, as a row's value. */
function profilePreview(text: string | null | undefined, limit = 40) {
  const line = (text ?? "").replace(/\s+/g, " ").trim();
  if (!line) return "None";
  return line.length > limit ? `${line.slice(0, limit - 1).trimEnd()}…` : line;
}

/** The agent's avatar and name, shown once at the top of its page. */
function AgentIdentityHeader({
  agent,
  title,
  subtitle,
}: {
  agent: AssistantAgent;
  title: string;
  subtitle: string;
}) {
  const { colors } = useTheme();
  return (
    <View style={styles.identity}>
      <AgentAvatar agent={agent} size="xl" />
      <Text
        accessibilityRole="header"
        style={[styles.identityName, { color: colors.textPrimary }]}
      >
        {title}
      </Text>
      <Text style={[styles.identityMeta, { color: colors.textSecondary }]}>
        @{agent.name} · {agent.status} ·{" "}
        {agent.kind === "nyxbot" ? "Your orchestrator" : "Specialist"} ·{" "}
        {agent.memory_count} memories
      </Text>
      <Text style={[styles.identityMeta, { color: colors.textSecondary }]}>
        {subtitle}
      </Text>
      {agent.share_role && agent.description ? (
        <Text
          style={[styles.identityAbout, { color: colors.textSecondary }]}
        >
          {agent.description}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  searchToolbar: { flexDirection: "row", alignItems: "center", gap: 8 },
  searchBox: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingLeft: 12,
    borderWidth: 1,
    borderRadius: 12,
  },
  searchInput: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    paddingVertical: 0,
    paddingRight: 8,
    fontFamily: fonts.body,
    fontSize: 14,
    textAlignVertical: "center",
  },
  clearSearch: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  listTool: {
    alignItems: "center",
    minHeight: 44,
    minWidth: 44,
    justifyContent: "center",
    borderRadius: 12,
    borderWidth: 1,
  },
  filterDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    position: "absolute",
    top: 6,
    right: 6,
  },
  title: { fontSize: 18, fontFamily: fonts.displaySemi },
  memoryField: { minHeight: 88, textAlignVertical: "top" },
  identity: {
    alignItems: "center",
    gap: 4,
    paddingTop: 8,
    paddingHorizontal: 16,
  },
  identityName: {
    marginTop: 8,
    fontSize: 22,
    lineHeight: 28,
    fontFamily: fonts.displaySemi,
    textAlign: "center",
  },
  identityMeta: {
    fontSize: 13,
    lineHeight: 18,
    fontFamily: fonts.body,
    textAlign: "center",
  },
  identityAbout: {
    marginTop: 4,
    fontSize: 14,
    lineHeight: 20,
    fontFamily: fonts.body,
    textAlign: "center",
  },
});
