import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
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
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  ChevronRight,
  Search,
  SlidersHorizontal,
  Users,
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
import { agentsApi } from "../../lib/api/agentsApi";
import { servicesApi } from "../../lib/api/servicesApi";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { fonts } from "../../theme/designTokens";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import { AgentModelField } from "./AgentModelField";
import {
  AgentPermissionsField,
  type AgentPermissions,
} from "./AgentPermissionsField";
import { useTheme } from "../../theme/ThemeContext";
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
  onClose,
  listHeader,
}: {
  agentId?: string;
  onOpenChat?: OpenChat;
  onOpenSkills?: (agentId: string) => void;
  onOpenAutomations?: (agentId: string) => void;
  onClose?: () => void;
  listHeader?: ReactNode;
} = {}) {
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [selectedId, setSelectedId] = useState<string | undefined>(agentId);
  const [creating, setCreating] = useState(false);
  const [search, setSearch] = useState("");
  const [includeDestroyed, setIncludeDestroyed] = useState(false);
  const [showFilters, setShowFilters] = useState(false);
  const [pullRefreshing, setPullRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [closeRequested, setCloseRequested] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [confirmation, setConfirmation] = useState<"destroy" | "delete">();
  const [editorVersion, setEditorVersion] = useState(0);
  const [profileVersion, setProfileVersion] = useState(0);
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
  const models = useQuery({
    queryKey: [...ROOT, "models"],
    queryFn: ({ signal }) => agentsApi.models(signal),
    enabled: focused && foreground && Boolean(selectedId),
    retry: false,
    staleTime: 60_000,
  });
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
  function back() {
    if (onClose) {
      setCloseRequested(true);
      return;
    }
    setSelectedId(undefined);
    setCreating(false);
    setError(undefined);
    setConfirmation(undefined);
  }
  useWorkspaceBack({
    active: onClose ? busy : Boolean(selectedId) || creating,
    defaultRole: onClose ? "back" : "menu",
    disabled: busy,
    label: onClose ? "Back" : "Back to agents",
    onBack: () => {
      if (!lock.current) back();
    },
  });
  useEffect(() => {
    if (busy || !closeRequested) return;
    setCloseRequested(false);
    onClose?.();
  }, [busy, closeRequested, onClose]);

  async function run(
    action: () => Promise<void>,
    success?: string,
    refreshForm?: "profile" | "grants",
  ): Promise<boolean> {
    if (lock.current) return false;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await action();
      await reload();
      if (refreshForm === "profile") setProfileVersion((value) => value + 1);
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

  if (creating)
    return (
      <WorkspacePage title="New agent" subtitle="Create a personal specialist">
        {error && <Notice message={error} tone="error" />}
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
          onCreate={(values) =>
            void run(async () => {
              const created = await agentsApi.create(values);
              setCreating(false);
              setSelectedId(created.id);
              setEditorVersion((value) => value + 1);
            }, "Agent created.")
          }
        />
      </WorkspacePage>
    );

  if (selectedId) {
    const agent = detail.data?.agent;
    const maintain = agent?.can_maintain === true;
    const alive = agent && agent.status !== "destroyed";
    return (
      <WorkspacePage
        title={agent?.display_name ?? agent?.name ?? "Agent"}
        subtitle={
          agent?.owner_kind === "org"
            ? `${agent.owner_name ?? "Organization"} · ${agent.org_role ?? "member"}`
            : "Personal agent"
        }
      >
        {notice && <Notice message={notice} tone="success" />}
        {error && <Notice message={error} tone="error" />}
        {detail.isPending && (
          <ActivityIndicator
            accessibilityLabel="Loading agent"
            color={colors.primary}
          />
        )}
        {detail.isError && (
          <>
            <Notice message={errorMessage(detail.error)} tone="error" />
            <ActionButton label="Retry" onPress={() => void detail.refetch()} />
          </>
        )}
        {agent && (
          <>
            <Card>
              <View style={styles.row}>
                <View
                  style={[
                    styles.avatar,
                    { backgroundColor: colors.primaryTone.bg },
                  ]}
                >
                  <Bot color={colors.primaryOnTint} size={26} />
                </View>
                <View style={styles.flex}>
                  <Text style={[styles.title, { color: colors.textPrimary }]}>
                    {agent.display_name ?? agent.name}
                  </Text>
                  <Label mono muted>
                    @{agent.name} · {agent.status}
                  </Label>
                  <Label muted>
                    {agent.kind === "nyxbot"
                      ? "Your orchestrator"
                      : "Specialist"}{" "}
                    · {agent.memory_count} memories
                  </Label>
                </View>
              </View>
              <Label muted>
                {agent.description || "Your assistant for everyday work."}
              </Label>
              {agent.pending_acknowledgements > 0 && (
                <Notice
                  message={`${agent.pending_acknowledgements} requests waiting for a decision.`}
                />
              )}
              {alive && agent.can_use === true && onOpenChat && (
                <ActionButton
                  label="Chat with agent"
                  onPress={() =>
                    onOpenChat({
                      agentId: agent.id,
                      openLatest: true,
                    })
                  }
                />
              )}
              {alive && maintain && onOpenSkills && (
                <ActionButton
                  label="Agent skills"
                  variant="secondary"
                  onPress={() => onOpenSkills(agent.id)}
                  disabled={busy}
                />
              )}
              {alive && agent.owner_kind !== "org" && onOpenAutomations && (
                <ActionButton
                  label="Agent automations"
                  variant="secondary"
                  onPress={() => onOpenAutomations(agent.id)}
                  disabled={busy}
                />
              )}
            </Card>
            {!maintain && (
              <Notice message="Your current role can view this agent. A maintainer can change its profile and permissions." />
            )}
            {!alive && (
              <Notice message="This agent is destroyed. Its threads are read only until you delete it permanently." />
            )}
            <AgentProfileForm
              key={`profile:${agent.id}:${editorVersion}:${profileVersion}`}
              agent={agent}
              models={models.data ?? []}
              modelsError={models.isError}
              busy={busy}
              onSave={(baseline, values) =>
                void run(
                  async () => {
                    const patch = buildAgentPatch(baseline, values);
                    if (!Object.keys(patch).length) return;
                    const latest = (await agentsApi.detail(agent.id)).agent;
                    if (
                      latest.can_maintain !== true ||
                      latest.status === "destroyed"
                    )
                      throw new Error(
                        "This agent can no longer be changed. Reload to review its current state.",
                      );
                    if (profileHasConflict(baseline, latest, patch))
                      throw new Error(
                        "This profile changed elsewhere. Reload the agent and review those changes before saving.",
                      );
                    await agentsApi.update(agent.id, patch);
                  },
                  "Profile saved.",
                  "profile",
                )
              }
            />
            {agent.kind === "specialist" && (
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
            <Card>
              <Label>Memory</Label>
              <Label muted>
                Notes this agent remembers across conversations.
              </Label>
              {detail.data?.memory.map((note) => (
                <MemoryNote
                  key={note.id}
                  text={note.text}
                  busy={busy}
                  editable={maintain && Boolean(alive)}
                  onForget={() =>
                    void run(async () => {
                      await agentsApi.forget(agent.id, note.id);
                    }, "Memory forgotten.")
                  }
                />
              ))}
              {!detail.data?.memory.length && (
                <Label muted>No memories yet.</Label>
              )}
              {maintain && alive && (
                <RememberForm
                  busy={busy}
                  onRemember={(text) =>
                    run(async () => {
                      await agentsApi.remember(agent.id, text);
                    }, "Memory saved.")
                  }
                />
              )}
            </Card>
            {!!agent.channels.length && (
              <Card>
                <Label>Connected channels</Label>
                {agent.channels.map((channel) => (
                  <Label key={channel.id} muted>
                    {channel.bot_label} · {channel.platform} · {channel.status}
                  </Label>
                ))}
              </Card>
            )}
            <Card>
              <Label>Recent threads</Label>
              {!detail.data?.threads.length && (
                <Label muted>No conversations yet.</Label>
              )}
              {detail.data?.threads.map((thread) => (
                <Pressable
                  key={thread.id}
                  accessibilityRole="button"
                  disabled={!onOpenChat || agent.can_use !== true}
                  style={[styles.thread, { borderColor: colors.borderSoft }]}
                  onPress={() =>
                    onOpenChat?.({
                      agentId: agent.id,
                      conversationId: thread.id,
                    })
                  }
                >
                  <View style={styles.flex}>
                    <Label>{thread.title}</Label>
                    <Label muted>
                      {thread.running
                        ? "Running"
                        : new Date(thread.last_message_at).toLocaleString()}
                    </Label>
                  </View>
                  <ChevronRight size={18} color={colors.textMuted} />
                </Pressable>
              ))}
            </Card>
            {maintain && agent.kind === "specialist" && (
              <Card>
                <Label>
                  {alive ? "Destroy agent" : "Permanently delete agent"}
                </Label>
                <Label muted>
                  {alive
                    ? "Stops its running threads and disconnects channel bots. Conversation history remains available."
                    : "Deletes this destroyed agent and all its threads. This cannot be undone."}
                </Label>
                {confirmation ? (
                  <>
                    <Notice
                      message={
                        confirmation === "destroy"
                          ? `Destroy “${agent.display_name ?? agent.name}”?`
                          : "Permanently delete this agent and all conversation history?"
                      }
                    />
                    <ActionButton
                      label={
                        confirmation === "destroy"
                          ? "Confirm destroy"
                          : "Delete permanently"
                      }
                      variant="danger"
                      loading={busy}
                      onPress={() =>
                        void run(
                          async () => {
                            if (confirmation === "destroy")
                              await agentsApi.destroy(agent.id);
                            else {
                              await agentsApi.remove(agent.id);
                              back();
                            }
                            setConfirmation(undefined);
                          },
                          confirmation === "destroy"
                            ? "Agent destroyed."
                            : "Agent deleted.",
                        )
                      }
                    />
                    <ActionButton
                      label="Cancel"
                      variant="secondary"
                      disabled={busy}
                      onPress={() => setConfirmation(undefined)}
                    />
                  </>
                ) : (
                  <ActionButton
                    label={alive ? "Destroy agent" : "Delete permanently"}
                    variant="danger"
                    disabled={busy}
                    onPress={() =>
                      setConfirmation(alive ? "destroy" : "delete")
                    }
                  />
                )}
              </Card>
            )}
            <ActionButton
              label="Reload agent and discard form changes"
              variant="secondary"
              disabled={busy || detail.isFetching}
              onPress={() => void refreshEditor()}
            />
          </>
        )}
      </WorkspacePage>
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
        agent.kind === "specialist" &&
        agent.status !== "destroyed",
    ).length ?? 0;
  const atLimit = list.data && live >= list.data.limits.max_live_subagents;
  return (
    <WorkspacePage
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
      {list.isPending && (
        <ActivityIndicator
          color={colors.primary}
          accessibilityLabel="Loading agents"
        />
      )}
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
              <View
                style={[
                  styles.avatar,
                  { backgroundColor: colors.primaryTone.bg },
                ]}
              >
                {agent.owner_kind === "org" ? (
                  <Users color={colors.primaryOnTint} size={24} />
                ) : (
                  <Bot color={colors.primaryOnTint} size={24} />
                )}
              </View>
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
        disabled={Boolean(atLimit) || list.isPending || list.isError}
        onPress={() => {
          Keyboard.dismiss();
          setCreating(true);
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
}: {
  connections: ServiceConnection[];
  connectionsLoading: boolean;
  connectionsError: boolean;
  busy: boolean;
  onCreate: (values: AgentCreate) => void;
}) {
  const [value, setValue] = useState<AgentCreate>({
    name: "",
    display_name: "",
    description: "",
    persona: "",
    services: [],
    account_read: false,
  });
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
        disabled={!value.name.trim() || !value.description.trim()}
        onPress={() => onCreate(value)}
      />
    </>
  );
}

function AgentProfileForm({
  agent,
  models,
  modelsError,
  busy,
  onSave,
}: {
  agent: AssistantAgent;
  models: { id: string; label: string; available: boolean }[];
  modelsError: boolean;
  busy: boolean;
  onSave: (baseline: AssistantAgent, values: AgentProfile) => void;
}) {
  const baseline = useRef(agent).current;
  const [value, setValue] = useState(() => agentProfile(agent));
  const disabled =
    busy || agent.can_maintain !== true || agent.status === "destroyed";
  const dirty =
    JSON.stringify(value) !== JSON.stringify(agentProfile(baseline));
  return (
    <Card>
      <Label>Profile</Label>
      <ProfileFields
        value={value}
        disabled={disabled}
        nyxbot={agent.kind === "nyxbot"}
        onChange={(patch) => setValue((old) => ({ ...old, ...patch }))}
      />
      {modelsError && (
        <Notice
          message="Could not load the model list. Your current model is preserved."
          tone="error"
        />
      )}
      <AgentModelField
        value={value.model}
        models={models}
        disabled={disabled}
        onChange={(model) => setValue((old) => ({ ...old, model }))}
      />
      <Label>Confirmation mode</Label>
      <Label muted>Controls when this agent asks before acting.</Label>
      {([null, "nyxmode", "ask_every_time", "autopilot"] as const).map(
        (mode) => (
          <Choice
            key={mode ?? "workspace"}
            label={
              mode === null
                ? "Follow workspace"
                : mode === "nyxmode"
                  ? "NyxMode · sensitive actions"
                  : mode === "ask_every_time"
                    ? "Ask · every change"
                    : "Autopilot"
            }
            selected={(value.access_mode_override ?? null) === mode}
            disabled={disabled}
            onPress={() =>
              setValue((old) => ({ ...old, access_mode_override: mode }))
            }
          />
        ),
      )}
      {value.access_mode_override === "autopilot" && (
        <Notice message="Autopilot may act without asking. Server policy and granted permissions still apply." />
      )}
      <ActionButton
        label="Save profile"
        loading={busy}
        disabled={disabled || !dirty}
        onPress={() => onSave(baseline, value)}
      />
    </Card>
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

function Choice({
  label,
  selected,
  disabled,
  checkbox,
  onPress,
}: {
  label: string;
  selected: boolean;
  disabled: boolean;
  checkbox?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole={checkbox ? "checkbox" : "radio"}
      accessibilityState={{ checked: selected, disabled }}
      onPress={onPress}
      disabled={disabled}
      style={[
        styles.choice,
        {
          backgroundColor: selected ? colors.primaryTone.bg : colors.cardSoft,
          borderColor: selected ? colors.primaryTone.border : colors.border,
          opacity: disabled ? 0.55 : 1,
        },
      ]}
    >
      <View
        style={[
          styles.check,
          {
            borderRadius: checkbox ? 4 : 10,
            borderColor: selected ? colors.primary : colors.textMuted,
            backgroundColor: selected ? colors.primary : "transparent",
          },
        ]}
      />
      <Text style={[styles.choiceLabel, { color: colors.textPrimary }]}>
        {label}
      </Text>
    </Pressable>
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
    <>
      <Field
        label="Add a memory"
        value={text}
        onChangeText={setText}
        multiline
        editable={!busy}
        maxLength={2000}
        placeholder="Something this agent should remember"
      />
      <ActionButton
        label="Remember"
        disabled={busy || !text.trim()}
        onPress={() => {
          void onRemember(text.trim()).then((saved) => {
            if (saved) setText("");
          });
        }}
      />
    </>
  );
}

function MemoryNote({
  text,
  busy,
  editable,
  onForget,
}: {
  text: string;
  busy: boolean;
  editable: boolean;
  onForget: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  return (
    <View style={styles.memory}>
      <Label>{text}</Label>
      {editable &&
        (confirm ? (
          <>
            <Label muted>Forget this memory?</Label>
            <ActionButton
              label="Confirm forget"
              variant="danger"
              disabled={busy}
              onPress={onForget}
            />
            <ActionButton
              label="Keep memory"
              variant="secondary"
              disabled={busy}
              onPress={() => setConfirm(false)}
            />
          </>
        ) : (
          <ActionButton
            label="Forget"
            variant="secondary"
            disabled={busy}
            onPress={() => setConfirm(true)}
          />
        ))}
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
  avatar: {
    width: 48,
    height: 48,
    borderRadius: 14,
    justifyContent: "center",
    alignItems: "center",
  },
  thread: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 12,
    borderBottomWidth: 1,
  },
  choice: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    padding: 12,
    minHeight: 44,
    borderRadius: 10,
    borderWidth: 1,
  },
  choiceLabel: { flex: 1, fontFamily: fonts.body, fontSize: 12 },
  check: { width: 16, height: 16, borderWidth: 1 },
  memory: { gap: 8, paddingVertical: 10 },
});
