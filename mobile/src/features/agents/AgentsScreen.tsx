import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { channelDetail } from "./channelPresentation";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import { isAgentAccessRefusal } from "./agentAccess";
import { OrgGuestChannel, OrgGuestGrantsAdoption } from "./OrgGuestControls";
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
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { useAppLanguage } from "../../i18n/LanguageProvider";
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
import type { GrantService } from "./agentContracts";
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
    ? (cause.issues[0]?.message ?? translateApp("Check the form fields."))
    : cause instanceof Error
      ? cause.message
      : translateApp("Could not complete this action. Try again.");
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
  onSelectionKept,
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
  onSelectionKept?: (agentId: string) => void;
} = {}) {
  const { t } = useAppLanguage();
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
  const [memoryDraft, setMemoryDraft] = useState({ agentId, text: "" });
  const memoryDirty =
    memoryDraft.agentId === selectedId && !!memoryDraft.text.trim();
  const [createdId, setCreatedId] = useState<string>();
  const [search, setSearch] = useState("");
  const [includeDestroyed, setIncludeDestroyed] = useState(false);
  const [showFilters, setShowFilters] = useState(false);
  const [pullRefreshing, setPullRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [closeRequested, setCloseRequested] = useState(false);
  const [error, setError] = useLocalizedState<string>();
  const [notice, setNotice] = useLocalizedState<string>();
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
  const grantOwner = !creating && detail.data?.agent.owner_kind === "org"
    ? detail.data.agent.owner_id : undefined;
  const connections = useQuery({
    queryKey: [...ROOT, "grant-services", grantOwner ?? null],
    queryFn: ({ signal }) => agentsApi.grantServices(grantOwner, signal),
    enabled: focused && foreground && (creating || Boolean(detail.data?.agent.can_maintain)),
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

  const agentSwitch = useRef(0);
  const requestedAgent = useRef(agentId);
  const deferredAgent = useRef<{ id: string | undefined } | null>(null);
  useEffect(() => {
    const revision = ++agentSwitch.current;
    if (requestedAgent.current !== agentId) {
      requestedAgent.current = agentId;
      deferredAgent.current = { id: agentId };
    }
    if (busy || !deferredAgent.current) return;
    const target = deferredAgent.current.id;
    deferredAgent.current = null;
    if (target === selectedId) return;
    if (memoryDirty) {
      Alert.alert(
        t("Discard memory draft?"),
        t("Your unsaved memory draft will be lost."),
        [
          {
            text: t("Keep editing"),
            style: "cancel",
            onPress: () => {
              if (agentSwitch.current === revision && selectedId)
                onSelectionKept?.(selectedId);
            },
          },
          {
            text: t("Discard"),
            style: "destructive",
            onPress: () => {
              if (agentSwitch.current === revision) setSelectedId(target);
            },
          },
        ],
      );
    } else setSelectedId(target);
    return () => {
      agentSwitch.current++;
    };
  }, [agentId, busy]);
  useEffect(() => {
    setMemoryDraft({ agentId: selectedId, text: "" });
  }, [selectedId]);
  useEffect(() => {
    if (!focused) return;
    const subscription = AppState.addEventListener("change", (state) => {
      setForeground(state === "active");
      if (state === "active") void reload();
    });
    return () => subscription.remove();
  }, [focused, reload]);
  function leave() {
    setMemoryDraft({ agentId: selectedId, text: "" });
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
    if (memoryDirty) {
      Alert.alert(
        t("Discard memory draft?"),
        t("Your unsaved memory draft will be lost."),
        [
          { text: t("Keep editing"), style: "cancel" },
          { text: t("Discard"), style: "destructive", onPress: leave },
        ],
      );
    } else if (creating && creationDirty) {
      Alert.alert(
        translateApp("Discard new agent?"),
        translateApp("Your agent has not been created."), [
        { text: translateApp("Keep editing"), style: "cancel" },
        { text: translateApp("Discard"), style: "destructive", onPress: leave },
      ]);
    } else leave();
  }
  useWorkspaceBack({
    active: onClose
      ? busy || ((memoryDirty || (creating && creationDirty)) && !closeRequested)
      : Boolean(selectedId) || creating,
    defaultRole: onClose ? "back" : "menu",
    disabled: busy,
    label: onClose ? translateApp("Back") : translateApp("Back to agents"),
    onBack: () => {
      if (!lock.current) back();
    },
    discardOnClose:
      memoryDirty && !closeRequested
        ? {
            title: t("Discard memory draft?"),
            message: t("Your unsaved memory draft will be lost."),
          }
        : creating && creationDirty && !closeRequested
          ? {
              title: translateApp("Discard new agent?"),
              message: translateApp("Your agent has not been created."),
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
    success?: () => string,
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
      if (isAgentAccessRefusal(cause)) await reload();
      setError(() => (errorMessage(cause)));
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
        throw localizedError("This agent can no longer be changed. Reload to review its current state.");
      if (profileHasConflict(agent, latest, patch))
        throw localizedError("This profile changed elsewhere. Reload the agent and review those changes before saving.");
      await agentsApi.update(agent.id, patch);
    }, () => (translateApp("Confirmation mode saved.")));
  }
  function confirmLifecycle(agent: AssistantAgent, destroy: boolean) {
    Alert.alert(
      destroy
        ? translateApp("Destroy “{{name}}”?", {
            name: agent.display_name ?? agent.name,
          })
        : translateApp(
            "Permanently delete this agent and all conversation history?",
          ),
      destroy
        ? translateApp(
            "Stops its running threads and disconnects channel bots. Conversation history remains available.",
          )
        : translateApp("This cannot be undone."),
      [
        { text: translateApp("Cancel"), style: "cancel" },
        {
          text: destroy ? translateApp("Destroy")
            : translateApp("Delete permanently"),
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
              () => (destroy ? translateApp("Agent destroyed.")
                : translateApp("Agent deleted.")),
            ),
        },
      ],
    );
  }

  if (creating)
    return (
      <WorkspacePage
        showHeading={false}
        title={translateApp("New agent")}
        subtitle={translateApp("Create a personal specialist")}
      >
        {error && <Notice message={error} tone="error" />}
        {!isConnected && (
          <Notice message={translateApp(
              "You're offline. Reconnect to create your agent. Your draft is kept.",
            )}
          />
        )}
        {creationAtLimit && (
          <Notice message={translateApp(
              "You've reached your personal agent limit. Destroy an unused specialist before creating another.",
            )}
          />
        )}
        {list.isError && (
          <Notice
            tone="error"
            message={translateApp(
              "Agent limits could not be checked. Go back and refresh your agents before creating another.",
            )}
          />
        )}
        {connections.isError && (
          <Notice
            message={translateApp(
              "Connections could not be loaded. You can create the agent without service access and grant it later.",
            )}
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
            }, () => (translateApp("Agent created.")));
          }}
        />
      </WorkspacePage>
    );

  if (selectedId) {
    const agent = isAgentAccessRefusal(detail.error) ? undefined : detail.data?.agent;
    const maintain = agent?.can_maintain === true;
    const alive = agent && agent.status !== "destroyed";
    const profileEditable = !!agent && maintain && !!alive && !busy;
    const editProfile = (field: AgentProfileField) =>
      agent && profileEditable && onEditProfile
        ? () => onEditProfile(agent.id, field)
        : undefined;
    const name = agent ? agent.display_name || agent.name : translateApp("Agent");
    const subtitle = agent?.share_role
      ? translateApp("{{owner}} · shared {{access}}", {
          owner: agent.owner_name ?? "Shared agent",
          access: agent.share_role,
        })
      : agent?.owner_kind === "org"
        ? translateApp("{{name}} · {{role}}", { name: agent.owner_name ?? translateApp("Organization"), role: agent.org_role ?? "member"})
        : translateApp("Personal agent");
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
          <SkeletonRows rows={4} title accessibilityLabel={translateApp("Loading agent")}
          />
        )}
        {detail.isError && (
          <>
            <Notice message={errorMessage(detail.error)} tone="error" />
            <SettingsGroup inset={false}>
              <SettingsRow
                tone="action"
                label={translateApp("Retry")}
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
                message={translateApp(
                  "{{name}} requests waiting for a decision.",
                  { name: agent.pending_acknowledgements },
                )}
              />
            )}
            {agent.share_role && (
              <Notice
                message={
                  agent.share_role === "manager"
                    ? translateApp(
                        "Shared Manager access can edit this agent's profile and published team memory. Its owner controls permissions and lifecycle.",
                      )
                    : translateApp(
                        "Shared User access can use this agent. Its owner controls permissions and lifecycle.",
                      )
                }
              />
            )}
            {agent.owner_kind === "org" && agent.can_use !== undefined && agent.can_maintain !== undefined && (!agent.can_use || !maintain) ? (
              <Notice message={agent.can_use
                ? translateApp("You can chat with this specialist. Its settings are read-only.")
                : maintain ? translateApp("You can maintain this specialist. Chat access is restricted.")
                  : translateApp("You can view this specialist's profile.")} />
            ) : !maintain && !agent.share_role && (
              <Notice message={translateApp(
                  "Your current role can view this agent. A maintainer can change its profile and permissions.",
                )}
              />
            )}
            {!alive && (
              <Notice message={translateApp(
                  "This agent is destroyed. Its threads are read only until you delete it permanently.",
                )}
              />
            )}
            {destinations && (
              <SettingsGroup inset={false}>
                {alive && agent.can_use === true && onOpenChat && (
                  <SettingsRow
                    tone="action"
                    label={translateApp("Chat with agent")}
                    onPress={() =>
                      onOpenChat({ agentId: agent.id, openLatest: true })
                    }
                  />
                )}
                {alive && maintain && !agent.share_role && onOpenSkills && (
                  <SettingsRow
                    label={translateApp("Skills")}
                    accessibilityLabel={translateApp("Agent skills")}
                    disabled={busy}
                    onPress={() => onOpenSkills(agent.id)}
                  />
                )}
                {alive && maintain && !agent.share_role && onOpenMachines && (
                  <SettingsRow
                    label={translateApp("Machine access")}
                    accessibilityLabel={translateApp("Agent machine access")}
                    disabled={busy}
                    onPress={() => onOpenMachines(agent.id)}
                  />
                )}
                {alive &&
                  !agent.share_role &&
                  agent.owner_kind !== "org" &&
                  onOpenAutomations && (
                    <SettingsRow
                      label={translateApp("Automations")}
                      accessibilityLabel={translateApp("Agent automations")}
                      disabled={busy}
                      onPress={() => onOpenAutomations(agent.id)}
                    />
                  )}
              </SettingsGroup>
            )}
            {!agent.share_role && (
              <SettingsGroup
                inset={false}
                title={translateApp("Profile")}
                footer={
                  confirmationMode === "autopilot"
                    ? translateApp(
                        "Autopilot may act without asking. Server policy and granted permissions still apply.",
                      )
                    : translateApp(
                        "Confirmation mode controls when this agent asks before acting.",
                      )
                }
              >
                <SettingsRow
                  label={translateApp("Handle")}
                  value={`@${agent.name}`}
                  onPress={
                    agent.kind === "nyxbot" ? undefined : editProfile("name")
                  }
                />
                <SettingsRow
                  label={translateApp("Display name")}
                  value={agent.display_name || translateApp("None")}
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
                  label={translateApp("Personality and tone")}
                  value={profilePreview(agent.persona)}
                  onPress={editProfile("persona")}
                />
                {profileEditable ? (
                  <SettingsMenuRow
                    label={translateApp("Confirmation mode")}
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
                    label={translateApp("Confirmation mode")}
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
                        throw localizedError("This agent can no longer be changed. Reload to review its current state.");
                      if (grantSignature(latest) !== grantSignature(baseline))
                        throw localizedError("The agent's permissions changed elsewhere. Reload and review them before saving.");
                      await agentsApi.grants(
                        agent.id,
                        changedGrants(baseline, services, accountRead, guests),
                      );
                    },
                    () => (translateApp("Permissions saved.")),
                    "grants",
                  )
                }
              />
            )}
            {agent.owner_kind === "org" && agent.org_role === "admin" && maintain && alive && (
              <OrgGuestGrantsAdoption key={`${getAuthEpoch()}:${agent.id}:guest-adoption`} agent={agent} disabled={busy} />
            )}
            <AgentCollaborationPanel
              key={`${getAuthEpoch()}:${agent.id}:sharing:${editorVersion}`}
              agent={agent}
            />
            <AgentOperationControls
              key={`${getAuthEpoch()}:${agent.id}:operations:${editorVersion}`}
              agent={agent}
              onOpenChat={
                onOpenChat && agent.can_use !== false
                  ? () => onOpenChat({ agentId: agent.id, openLatest: true })
                  : undefined
              }
            />
            {!agent.share_role && !(agent.owner_kind === "org" && agent.can_maintain === false) && (
              <SettingsGroup
                inset={false}
                title={translateApp("Memory")}
                footer={
                  maintain && alive && memory.length
                    ? translateApp(
                        "Notes this agent remembers across conversations. Tap a memory to forget it.",
                      )
                    : translateApp(
                        "Notes this agent remembers across conversations.",
                      )
                }
              >
                {memory.map((note) => (
                  <SettingsRow
                    key={note.id}
                    label={note.text}
                    chevron={false}
                    disabled={busy && maintain && !!alive}
                    accessibilityHint={
                      maintain && alive ? translateApp("Asks to forget this memory")
                        : undefined
                    }
                    onPress={
                      maintain && alive
                        ? () =>
                            Alert.alert(
                              translateApp("Forget this memory?"),
                              note.text,
                              [
                                { text: translateApp("Keep memory"), style: "cancel" },
                                {
                                  text: translateApp("Forget"),
                                  style: "destructive",
                                  onPress: () =>
                                    void run(async () => {
                                      await agentsApi.forget(agent.id, note.id);
                                    }, () => (translateApp("Memory forgotten."))),
                                },
                              ],
                            )
                        : undefined
                    }
                  />
                ))}
                {!memory.length && (
                  <SettingsRow label={translateApp("No memories yet")}
                    disabled />
                )}
              </SettingsGroup>
            )}
            {!agent.share_role && maintain && alive && (
              <RememberForm
                key={agent.id}
                text={memoryDraft.agentId === agent.id ? memoryDraft.text : ""}
                onChangeText={(text) =>
                  setMemoryDraft({ agentId: agent.id, text })
                }
                busy={busy}
                onRemember={(text) =>
                  run(async () => {
                    await agentsApi.remember(agent.id, text);
                  }, () => (translateApp("Memory saved.")))
                }
              />
            )}
            {!!agent.channels.length && (
              <SettingsGroup inset={false} title={translateApp("Connected channels")}>
                {agent.channels.map((channel) => {
                  const detail = channelDetail(channel);
                  return agent.owner_kind === "org" ? (
                  <OrgGuestChannel key={`${getAuthEpoch()}:${channel.id}`} agent={agent} channel={channel} />
                ) : (
                  <SettingsRow
                    key={channel.id}
                    label={channel.bot_label}
                    detail={detail.kind === "message" ? t(detail.value) : detail.value}
                    value={channel.status}
                  />
                ); })}
              </SettingsGroup>
            )}
            {agent.can_use !== false && <SettingsGroup inset={false} title={translateApp("Recent threads")}>
              {!threads.length && (
                <SettingsRow label={translateApp("No conversations yet")}
                  disabled />
              )}
              {threads.map((thread) => (
                <SettingsRow
                  key={thread.id}
                  label={thread.title}
                  detail={
                    thread.running
                      ? translateApp("Running")
                      : new Date(thread.last_message_at).toLocaleString(
                          appLocale(),
                        )
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
            </SettingsGroup>}
            {maintain && !agent.share_role && agent.kind === "specialist" && (
              <SettingsGroup
                inset={false}
                footer={
                  alive
                    ? translateApp(
                        "Stops its running threads and disconnects channel bots. Conversation history remains available.",
                      )
                    : translateApp(
                        "Deletes this destroyed agent and all its threads. This cannot be undone.",
                      )
                }
              >
                <SettingsRow
                  centered
                  destructive
                  label={alive ? translateApp("Destroy Agent")
                      : translateApp("Delete Permanently")
                  }
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
      title={translateApp("Your agents")}
      subtitle={translateApp("Personal & shared agents")}
      refreshing={pullRefreshing}
      onRefresh={refreshList}
    >
      {listHeader}
      {notice && <Notice message={notice} tone="success" />}
      {error && <Notice message={error} tone="error" />}
      {atLimit && (
        <Notice message={translateApp(
            "Your current agent limit is reached. Destroy an unused specialist or update your workspace limit in Settings.",
          )}
        />
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
            accessibilityLabel={translateApp("Search agents")}
            value={search}
            onChangeText={setSearch}
            placeholder={translateApp("Search agents…")}
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
              accessibilityLabel={translateApp("Clear agent search")}
              onPress={() => setSearch("")}
              style={styles.clearSearch}
            >
              <X size={17} color={colors.textMuted} />
            </Pressable>
          )}
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={translateApp("Filter agents")}
          accessibilityHint={
            includeDestroyed
              ? translateApp("Including destroyed agents. Open filters.")
              : translateApp("Open status filters.")
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
          label={translateApp("Agent filters")}
          title={translateApp("Filter agents")}
          options={[
            {
              value: "destroyed",
              label: translateApp("Include destroyed agents"),
              description: translateApp(
                "Show destroyed agents alongside active agents.",
              ),
            },
          ]}
          value={includeDestroyed ? ["destroyed"] : []}
          multiple
          confirmLabel={translateApp("Apply filters")}
          onChange={(selected) =>
            setIncludeDestroyed(selected.includes("destroyed"))
          }
          onClose={() => setShowFilters(false)}
        />
      )}
      {list.isPending && (
        <SkeletonCards accessibilityLabel={translateApp("Loading agents")} />
      )}
      {list.isError && (
        <>
          <Notice message={errorMessage(list.error)} tone="error" />
          <ActionButton label={translateApp("Retry")}
            onPress={() => void list.refetch()} />
        </>
      )}
      {filtered.map((agent) => (
        <Pressable
          key={agent.id}
          accessibilityRole="button"
          accessibilityLabel={translateApp("Manage {{slug}}", {
            slug: agent.display_name ?? agent.name,
          })}
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
                  {agent.description || translateApp("Your personal agent")}
                </Label>
              </View>
              <ChevronRight size={18} color={colors.textMuted} />
            </View>
            <Label muted style={{ fontSize: 11 }}>
              {(agent.status === "running"
                ? (agent.owner_kind === "org" ? (agent.pending_acknowledgements ? translateApp("● Working · {{name}} · {{pendingAcknowledgements}} pending", { name: agent.owner_name ?? translateApp("Organization"), pendingAcknowledgements: agent.pending_acknowledgements }) : translateApp("● Working · {{name}}", { name: agent.owner_name ?? translateApp("Organization") })) : (agent.pending_acknowledgements ? translateApp("● Working · Personal · {{pendingAcknowledgements}} pending", { pendingAcknowledgements: agent.pending_acknowledgements }) : translateApp("● Working · Personal"))) : (agent.status === "destroyed"
                  ? (agent.owner_kind === "org"
                ? (agent.pending_acknowledgements ? translateApp("Destroyed · {{name}} · {{pendingAcknowledgements}} pending", { name: agent.owner_name ?? translateApp("Organization"), pendingAcknowledgements: agent.pending_acknowledgements }) : translateApp("Destroyed · {{name}}", { name: agent.owner_name ?? translateApp("Organization") })) : (agent.pending_acknowledgements
                ? translateApp("Destroyed · Personal · {{pendingAcknowledgements}} pending", { pendingAcknowledgements: agent.pending_acknowledgements }) : translateApp("Destroyed · Personal"))) : (agent.owner_kind === "org" ? (agent.pending_acknowledgements ? translateApp("● Ready · {{name}} · {{pendingAcknowledgements}} pending", { name: agent.owner_name ?? translateApp("Organization"), pendingAcknowledgements: agent.pending_acknowledgements }) : translateApp("● Ready · {{name}}", { name: agent.owner_name ?? translateApp("Organization") })) : (agent.pending_acknowledgements ? translateApp("● Ready · Personal · {{pendingAcknowledgements}} pending", { pendingAcknowledgements: agent.pending_acknowledgements }) : translateApp("● Ready · Personal")))))}
            </Label>
          </Card>
        </Pressable>
      ))}
      {!list.isPending && !list.isError && !filtered.length && (
        <Card>
          <Label>{translateApp("No matching agents")}</Label>
          <Label muted>
            {translateApp("Change your search or create a new specialist.")}
          </Label>
        </Card>
      )}
      <ActionButton
        label={translateApp("Create an agent")}
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
          {translateApp("{{live}} / {{maxLiveSubagents}} personal specialists", { live: live, maxLiveSubagents: list.data.limits.max_live_subagents})}</Label>
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
  useAppLanguage();
  return (
    <>
      <Field
        label={translateApp("Handle")}
        value={value.name}
        onChangeText={(name) => onChange({ name: name.toLowerCase() })}
        editable={!disabled && !nyxbot}
        autoCapitalize="none"
        autoCorrect={false}
        maxLength={32}
        placeholder={translateApp("researcher")}
      />
      <Field
        label={translateApp("Display name")}
        value={value.display_name}
        onChangeText={(display_name) => onChange({ display_name })}
        editable={!disabled}
        maxLength={40}
        placeholder={translateApp("Optional friendly name")}
      />
      <Field
        label={nyxbot ? translateApp("Notes") : translateApp("Purpose")}
        value={value.description}
        onChangeText={(description) => onChange({ description })}
        editable={!disabled}
        multiline
        maxLength={2048}
        placeholder={translateApp("What should this agent do?")}
      />
      <Field
        label={translateApp("Personality and tone")}
        value={value.persona}
        onChangeText={(persona) => onChange({ persona })}
        editable={!disabled}
        multiline
        maxLength={2000}
        placeholder={translateApp("Warm, concise, explains its decisions")}
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
  connections: GrantService[];
  connectionsLoading: boolean;
  connectionsError: boolean;
  busy: boolean;
  onCreate: (values: AgentCreate) => void;
  available: boolean;
  onDirtyChange: (dirty: boolean) => void;
}) {
  useAppLanguage();
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
          {translateApp(
            "A specialist uses only the services you grant. Personality changes its tone, not its authority.",
          )}
        </Label>
      </Card>
      <AgentPermissionsField
        title={translateApp("Starting permissions")}
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
        label={translateApp("Create agent")}
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
  connections: GrantService[];
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
  useAppLanguage();
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
      allowGuestAccess={agent.owner_kind !== "org" || agent.org_role === "admin"}
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
  text,
  onChangeText,
}: {
  busy: boolean;
  text: string;
  onChangeText: (text: string) => void;
  onRemember: (text: string) => Promise<boolean>;
}) {
  useAppLanguage();
  return (
    <SettingsGroup inset={false} title={translateApp("Add a memory")}>
      <SettingsTextField
        accessibilityLabel={translateApp("Add a memory")}
        value={text}
        onChangeText={onChangeText}
        multiline
        clearButtonMode="never"
        editable={!busy}
        maxLength={2000}
        placeholder={translateApp("Something this agent should remember")}
        style={styles.memoryField}
      />
      <SettingsRow
        tone="action"
        label={translateApp("Remember")}
        disabled={busy || !text.trim()}
        onPress={() => {
          void onRemember(text.trim()).then((saved) => {
            if (saved) onChangeText("");
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
  get workspace() {
    return translateApp("Follow workspace");
  },
  get nyxmode() {
    return translateApp("NyxMode · sensitive actions");
  },
  get ask_every_time() {
    return translateApp("Ask · every change");
  },
  get autopilot() {
    return translateApp("Autopilot");
  },
} as const;
type ConfirmationChoice = keyof typeof CONFIRMATION_MODE_LABEL;

/** One line of a long profile value, as a row's value. */
function profilePreview(text: string | null | undefined, limit = 40) {
  const line = (text ?? "").replace(/\s+/g, " ").trim();
  if (!line) return translateApp("None");
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
  useAppLanguage();
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
      <Text style={[styles.identityMeta, { color: colors.textSecondary }]}>{agent.owner_kind === "org" && agent.can_maintain === false ? translateApp("@{{agent}} · {{status}}", { agent: agent.name, status: agent.status }) : (agent.kind === "nyxbot" ? translateApp("@{{agent}} · {{status}} · Your orchestrator · {{memoryCount}} memories", { agent: agent.name, status: agent.status, memoryCount: agent.memory_count}) : translateApp("@{{agent}} · {{status}} · Specialist · {{memoryCount}} memories", { agent: agent.name, status: agent.status, memoryCount: agent.memory_count }))}</Text>
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
