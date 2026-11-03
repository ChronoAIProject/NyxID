import { AgentAvatar } from "../../components/AgentAvatar";
import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Check, ChevronDown } from "lucide-react-native";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { pluginSkillsApi as api } from "../../lib/api/pluginSkillsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { AssistantAgent } from "../agents/agentContracts";
import type {
  ContentScreenProps,
  SkillPageParams,
  WorkspaceNavigationProp,
} from "../../app/navigationTypes";
import {
  useOpenWorkspaceChat,
  useWorkspaceBack,
} from "../../app/WorkspaceHeader";
import { newAssistantChatParams } from "../../app/assistantChatParams";
import { SkillLibrary, type SkillLibraryTab } from "./SkillLibrary";
import {
  assertSkillRevision,
  buildSkillSelection,
  SKILL_HISTORY_LABELS,
  type SkillChange,
  type SkillVersion,
} from "./pluginSkillsTypes";

function LatestBadge() {
  const { colors } = useTheme();
  return (
    <View
      style={{
        borderRadius: 6,
        paddingHorizontal: 7,
        paddingVertical: 2,
        backgroundColor: colors.cardSoft,
      }}
    >
      <Label style={{ fontSize: 11, color: colors.textSecondary }}>
        Latest
      </Label>
    </View>
  );
}

const ROOT = ["workspace", "plugins", "skills"] as const;
const titleOf = (agent: AssistantAgent) => agent.display_name || agent.name;
const failure = (cause: unknown) =>
  cause instanceof Error
    ? cause.message
    : "The request could not be completed. Try again.";
/** The catalog and every pin belong to the explicitly selected agent. */
export function PluginSkills({ agentId }: { agentId?: string }) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const epoch = getAuthEpoch();
  const [chosen, setChosen] = useState(agentId ?? "");
  const [picking, setPicking] = useState(false);
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: focused,
    retry: false,
  });

  useEffect(() => {
    setChosen(agentId ?? "");
  }, [agentId, epoch]);
  const manageable = (agents.data?.agents ?? []).filter(
    (agent) => agent.status !== "destroyed" && agent.can_maintain !== false,
  );
  const agent = chosen
    ? manageable.find((item) => item.id === chosen)
    : (manageable.find((item) => item.kind === "nyxbot") ?? manageable[0]);
  const selectionScope = JSON.stringify([
    epoch,
    agentId,
    agent?.id,
    agent?.owner_kind,
    agent?.owner_id,
  ]);
  useEffect(() => {
    setPicking(false);
  }, [selectionScope, focused]);
  return (
    <View style={styles.stack}>
      <View style={styles.stackSmall}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Agent for skills: ${agent ? titleOf(agent) : "Choose an agent"}`}
          accessibilityState={{
            expanded: picking,
            disabled: !manageable.length,
          }}
          disabled={!manageable.length}
          onPress={() => setPicking((value) => !value)}
          style={[styles.agentContext, { backgroundColor: colors.cardSoft }]}
        >
          {agent ? <AgentAvatar agent={agent} size="lg" /> : null}
          <View style={styles.flex}>
            <Label muted style={styles.caption}>
              Skills for
            </Label>
            <Label numberOfLines={2} style={styles.agentName}>
              {agent ? titleOf(agent) : "Choose an agent"}
            </Label>
            {agent?.owner_kind === "org" && (
              <Label muted style={styles.caption}>
                {agent.owner_name ?? "Organization"}
              </Label>
            )}
          </View>
          <ChevronDown color={colors.textSecondary} size={18} />
        </Pressable>
        {picking && (
          <WorkspaceSelectionSheet
            label="Agent for skills"
            title="Choose an agent"
            options={manageable.map((item) => ({
              value: item.id,
              label: titleOf(item),
              leading: <AgentAvatar agent={item} size="lg" />,
              description:
                item.owner_kind === "org"
                  ? (item.owner_name ?? "Organization")
                  : "Personal",
            }))}
            value={agent ? [agent.id] : []}
            allowEmpty={false}
            scopeKey={selectionScope}
            onChange={(values) => {
              if (
                values.length === 1 &&
                manageable.some((item) => item.id === values[0])
              )
                setChosen(values[0]!);
            }}
            onClose={() => setPicking(false)}
          />
        )}
      </View>
      {agents.isPending && (
        <ActivityIndicator
          accessibilityLabel="Loading agents for skills"
          color={colors.primary}
        />
      )}
      {agents.isError && (
        <>
          <Notice tone="error" message={failure(agents.error)} />
          <ActionButton
            label="Retry agents"
            variant="secondary"
            onPress={() => void agents.refetch()}
          />
        </>
      )}
      {!agents.isPending && !agents.isError && !agent && (
        <Notice
          message={
            chosen
              ? "Requested agent unavailable. It may have been deleted or your maintenance access changed. Choose another agent to manage its skills."
              : "No agent you can maintain is available."
          }
        />
      )}
      {agent && <AgentSkillsManager key={agent.id} agent={agent} />}
    </View>
  );
}

type SkillView = "detail" | "versions" | "history";
const skillRoutes = {
  detail: "SkillDetail",
  versions: "SkillVersions",
  history: "SkillHistory",
} as const;

export function PluginSkillPage({
  route,
}: ContentScreenProps<"SkillDetail" | "SkillVersions" | "SkillHistory">) {
  const focused = useIsFocused();
  const agent = useQuery({
    queryKey: ["workspace", "agents", "detail", route.params.agentId],
    queryFn: ({ signal }) => agentsApi.detail(route.params.agentId, signal),
    enabled: focused,
    retry: false,
  });

  const view = (Object.keys(skillRoutes) as SkillView[]).find(
    (key) => skillRoutes[key] === route.name,
  )!;
  return (
    <WorkspacePage title={route.params.name} showHeading={false}>
      {agent.isPending && (
        <ActivityIndicator accessibilityLabel="Loading skill agent" />
      )}
      {agent.isError && <Notice tone="error" message={failure(agent.error)} />}
      {agent.data &&
        (agent.data.agent.can_maintain === false ||
          agent.data.agent.status === "destroyed") && (
          <Notice message="This agent is unavailable or your maintenance access changed." />
        )}
      {agent.data &&
        agent.data.agent.can_maintain !== false &&
        agent.data.agent.status !== "destroyed" && (
          <AgentSkillsManager
            key={route.key}
            agent={agent.data.agent}
            skillPage={route.params}
            view={view}
          />
        )}
    </WorkspacePage>
  );
}

function SkillPageGuard({
  busy,
  confirmDetach,
  onBack,
}: {
  busy: boolean;
  confirmDetach: boolean;
  onBack: () => void;
}) {
  useWorkspaceBack({
    active: confirmDetach,
    onBack,
    defaultRole: "back",
    disabled: busy,
  });
  return null;
}

function AgentSkillsManager({
  agent,
  skillPage,
  view = "detail",
}: {
  agent: AssistantAgent;
  skillPage?: SkillPageParams;
  view?: SkillView;
}) {
  const { colors } = useTheme();
  const client = useQueryClient();
  const focused = useIsFocused();
  const [libraryTab, setLibraryTab] = useState<SkillLibraryTab>("added");
  const [addedQuery, setAddedQuery] = useState("");
  const [catalogInput, setCatalogInput] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [page, setPage] = useState(1);
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const openChat = useOpenWorkspaceChat();
  const detail = skillPage
    ? {
        id: skillPage.skillId,
        name: skillPage.name,
        description: skillPage.description,
      }
    : undefined;
  const [versionPage, setVersionPage] = useState(1);
  const [selectedVersion, setSelectedVersion] = useState<SkillVersion>();
  const [testing, setTesting] = useState(false);
  const [confirmDetach, setConfirmDetach] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const lock = useRef(false);
  const authEpoch = useRef(getAuthEpoch());
  const currentKey = [...ROOT, agent.id] as const;
  const current = useQuery({
    queryKey: currentKey,
    queryFn: ({ signal }) => api.current(agent.id, signal),
    enabled: focused,
    retry: false,
  });
  const currentReady = Boolean(current.data) && !current.isError;
  useEffect(() => {
    const timer = setTimeout(() => {
      setSearchQuery(catalogInput.trim().slice(0, 200));
      setPage(1);
    }, 300);
    return () => clearTimeout(timer);
  }, [catalogInput]);
  const waitingForQuery = catalogInput.trim().slice(0, 200) !== searchQuery;
  const catalog = useQuery({
    queryKey: [...ROOT, "catalog", searchQuery, page],
    queryFn: ({ signal }) => api.search(searchQuery, page, signal),
    enabled: focused && libraryTab === "discover" && !waitingForQuery,
    retry: false,
  });
  const versions = useQuery({
    queryKey: [...ROOT, "versions", detail?.id, versionPage],
    queryFn: ({ signal }) => api.versions(detail!.id, versionPage, signal),
    enabled: focused && Boolean(detail),
    retry: false,
  });
  const initialVersionApplied = useRef(false);
  useEffect(() => {
    if (initialVersionApplied.current || !skillPage?.version || !versions.data)
      return;
    initialVersionApplied.current = true;
    setSelectedVersion(
      versions.data.items.find((item) => item.version === skillPage.version),
    );
  }, [skillPage?.version, versions.data]);
  const preview = useQuery({
    queryKey: [...ROOT, "preview", detail?.id, selectedVersion?.version],
    queryFn: ({ signal }) =>
      api.preview(detail!.id, selectedVersion!.version, signal),
    enabled:
      focused && Boolean(detail && selectedVersion) && view === "versions",
    retry: false,
  });
  const tested = useQuery({
    queryKey: [...ROOT, "test", agent.id, detail?.id],
    queryFn: ({ signal }) => api.test(agent.id, detail!.id, signal),
    enabled: focused && testing && Boolean(detail),
    retry: false,
    staleTime: 0,
  });
  const history = useQuery({
    queryKey: [...ROOT, "history", agent.id],
    queryFn: ({ signal }) => api.history(agent.id, signal),
    enabled: focused && Boolean(detail) && view === "history",
    retry: false,
  });
  function openSkill(id: string, name: string, description?: string | null) {
    navigation.push("SkillDetail", {
      agentId: agent.id,
      skillId: id,
      name,
      description,
    });
  }
  function openView(next: SkillView, version?: string) {
    if (skillPage)
      navigation.push(skillRoutes[next], {
        ...skillPage,
        version,
      });
  }
  async function refresh() {
    await Promise.all([
      client.invalidateQueries({ queryKey: currentKey }),
      client.invalidateQueries({ queryKey: [...ROOT, "history", agent.id] }),
      client.invalidateQueries({ queryKey: [...ROOT, "test", agent.id] }),
    ]);
  }
  async function run(action: () => Promise<void>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      if (authEpoch.current !== getAuthEpoch())
        throw new Error(
          "Your account session changed. Reopen Plugins to continue.",
        );
      await action();
    } catch (cause) {
      setError(failure(cause));
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function change(change: SkillChange) {
    const baseline = current.data;
    if (!baseline || !currentReady) return;
    await run(async () => {
      const [latest, latestAgent] = await Promise.all([
        api.current(agent.id),
        agentsApi.detail(agent.id),
      ]);
      if (
        latestAgent.agent.can_maintain === false ||
        latestAgent.agent.status === "destroyed"
      )
        throw new Error(
          "This agent can no longer be changed. Reload to review its current state.",
        );
      assertSkillRevision(agent.id, baseline, latest);
      const selection = buildSkillSelection(latest, change);
      if (authEpoch.current !== getAuthEpoch())
        throw new Error(
          "Your account session changed. Reopen Plugins to continue.",
        );
      await api.write(agent.id, selection);
      await refresh();
      setConfirmDetach(false);
      setTesting(false);
      setNotice(
        change.type === "detach"
          ? "Skill removed from agent."
          : "Exact skill version attached.",
      );
      if (change.type === "detach") navigation.goBack();
      else if (change.type === "attach")
        navigation.popTo("SkillDetail", skillPage!);
    });
  }
  const pins = current.data?.skills ?? [];
  const pin = pins.find((item) => item.skill_id === detail?.id);
  const metadata = detail ? current.data?.metadata[detail.id] : undefined;
  const latestVersion =
    versionPage === 1
      ? versions.data?.items.find((item) => !item.deprecated)
      : undefined;
  const pinnedLatest = Boolean(
    pin &&
    latestVersion &&
    pin.version === latestVersion.version &&
    pin.sha256 === latestVersion.sha256,
  );

  return (
    <View style={styles.stack}>
      {!detail && notice && <Notice tone="success" message={notice} />}
      {!detail && error && <Notice tone="error" message={error} />}
      {!skillPage && (
        <SkillLibrary
          agentName={titleOf(agent)}
          tab={libraryTab}
          onTabChange={setLibraryTab}
          query={libraryTab === "added" ? addedQuery : catalogInput}
          onQueryChange={
            libraryTab === "added" ? setAddedQuery : setCatalogInput
          }
          added={{
            items: pins.map((item) => ({
              id: item.skill_id,
              name: item.name,
              description:
                current.data?.metadata[item.skill_id]?.description ?? "",
              version: item.version,
              disabled: item.disabled,
            })),
            known: Boolean(current.data),
            loading: current.isPending,
            error: current.isError ? failure(current.error) : undefined,
            onRetry: () => {
              void current.refetch();
            },
          }}
          catalog={{
            items: waitingForQuery ? [] : (catalog.data?.items ?? []),
            known: !waitingForQuery && Boolean(catalog.data),
            loading: waitingForQuery || catalog.isPending || catalog.isFetching,
            error:
              !waitingForQuery && catalog.isError
                ? failure(catalog.error)
                : undefined,
            onRetry: () => {
              void catalog.refetch();
            },
          }}
          page={page}
          totalPages={catalog.data?.total_pages ?? 1}
          onPageChange={setPage}
          onOpenSkill={openSkill}
        />
      )}
      {skillPage && (
        <View style={styles.stack}>
          <SkillPageGuard
            busy={busy}
            confirmDetach={confirmDetach}
            onBack={() =>
              confirmDetach ? setConfirmDetach(false) : navigation.goBack()
            }
          />
          {error && (
            <>
              <Notice tone="error" message={error} />
              <ActionButton
                label="Reload current skills"
                variant="secondary"
                disabled={busy}
                onPress={() => void refresh().then(() => setError(undefined))}
              />
            </>
          )}
          {notice && <Notice tone="success" message={notice} />}
          {current.isError && (
            <>
              <Notice tone="error" message={failure(current.error)} />
              {current.data && (
                <Label muted>
                  Showing last loaded attached skills. Retry to confirm before
                  changing versions or removing a skill.
                </Label>
              )}
              <ActionButton
                label="Retry attached skills"
                variant="secondary"
                loading={current.isFetching}
                disabled={busy || !focused}
                onPress={() => void current.refetch()}
              />
            </>
          )}
          {view === "detail" && detail && (
            <>
              <View style={styles.detailIdentity}>
                <View
                  style={[
                    styles.icon,
                    { backgroundColor: colors.primaryTone.bg },
                  ]}
                >
                  <BookOpen size={26} color={colors.primaryOnTint} />
                </View>
                <View style={styles.flex}>
                  <Label style={styles.cardTitle}>{detail.name}</Label>
                  <Label muted>
                    {pin ? `Attached to ${titleOf(agent)}` : "Ornn catalog"}
                  </Label>
                </View>
              </View>
              {(metadata?.description ?? detail.description) && (
                <Label muted>
                  {metadata?.description ?? detail.description}
                </Label>
              )}
              <Card>
                <Label muted style={styles.caption}>
                  {current.isError && current.data
                    ? "Last loaded version"
                    : "Pinned version"}
                </Label>
                <View style={styles.versionHeading}>
                  <Label mono>
                    {pin?.version ??
                      (current.isPending
                        ? "Loading attached version…"
                        : currentReady
                          ? "Not attached"
                          : "Attachment status unavailable")}
                  </Label>
                  {pinnedLatest && <LatestBadge />}
                </View>
                {pin && (
                  <Label muted style={styles.caption}>
                    {metadata?.size_bytes !== undefined
                      ? `${metadata.size_bytes.toLocaleString()} bytes · `
                      : ""}
                    {pin.dependencies.length} pinned dependencies
                  </Label>
                )}
              </Card>
              <Card>
                <Label muted>To change this skill, ask NyxBot.</Label>
                <ActionButton
                  label="Ask NyxBot"
                  variant="secondary"
                  disabled={busy || !focused}
                  onPress={() => {
                    if (
                      !lock.current &&
                      focused &&
                      authEpoch.current === getAuthEpoch()
                    )
                      openChat(newAssistantChatParams());
                  }}
                />
              </Card>
              {pin?.disabled && (
                <Notice message="This skill is disabled. Ask NyxBot to enable it before changing its version." />
              )}
              {latestVersion && !pinnedLatest && (
                <ActionButton
                  label={`Use latest (v${latestVersion.version})`}
                  variant="secondary"
                  disabled={busy || !currentReady || Boolean(pin?.disabled)}
                  onPress={() => {
                    openView("versions", latestVersion.version);
                  }}
                />
              )}
              <ActionButton
                label={pin ? "Choose exact version" : "Attach exact version"}
                variant="secondary"
                disabled={busy || !currentReady}
                onPress={() => {
                  openView("versions");
                  setVersionPage(1);
                  setSelectedVersion(undefined);
                }}
              />
              {pin && (
                <>
                  <Card>
                    <Label>Test attached skill</Label>
                    <Label muted style={styles.caption}>
                      Read the exact package this agent uses.
                    </Label>
                    <ActionButton
                      label={
                        tested.isFetching && testing
                          ? "Testing attached skill…"
                          : "Read pinned package"
                      }
                      variant="secondary"
                      loading={testing && tested.isFetching}
                      disabled={busy}
                      onPress={() => {
                        if (testing) void tested.refetch();
                        else setTesting(true);
                      }}
                    />
                    {testing && tested.isError && (
                      <Notice
                        tone="error"
                        message={`Test failed: ${failure(tested.error)}`}
                      />
                    )}
                    {testing &&
                      tested.data &&
                      !tested.isFetching &&
                      !tested.isError && (
                        <>
                          <Notice
                            tone={tested.data.readable ? "success" : "info"}
                            message={
                              tested.data.readable
                                ? `Test passed: the agent reads ${tested.data.name} ${tested.data.version} (${tested.data.size_bytes.toLocaleString()} bytes).`
                                : `${tested.data.name} ${tested.data.version} is disabled, so the agent does not read it.`
                            }
                          />
                          {tested.data.excerpt ? (
                            <Label
                              selectable
                              mono
                              style={[
                                styles.excerpt,
                                { backgroundColor: colors.cardSoft },
                              ]}
                            >
                              {tested.data.excerpt}
                            </Label>
                          ) : null}
                        </>
                      )}
                  </Card>
                  <ActionButton
                    label="Skill history"
                    variant="secondary"
                    disabled={busy}
                    onPress={() => openView("history")}
                  />
                  {confirmDetach ? (
                    <Card>
                      <Notice
                        message={`Remove ${pin.name} from ${titleOf(agent)}? Other attached skills stay in place. The skill stays in Ornn.`}
                      />
                      <ActionButton
                        label="Confirm remove from agent"
                        variant="danger"
                        loading={busy}
                        disabled={!currentReady}
                        onPress={() =>
                          void change({
                            type: "detach",
                            skillId: pin.skill_id,
                          })
                        }
                      />
                      <ActionButton
                        label="Cancel"
                        variant="secondary"
                        disabled={busy}
                        onPress={() => setConfirmDetach(false)}
                      />
                    </Card>
                  ) : (
                    <ActionButton
                      label="Remove from agent"
                      variant="danger"
                      disabled={busy || !currentReady}
                      onPress={() => setConfirmDetach(true)}
                    />
                  )}
                </>
              )}
            </>
          )}
          {view === "versions" && detail && (
            <>
              <Label muted>
                Choose which version of {detail.name} this agent uses. Skills
                change how the agent works.
              </Label>
              {versions.isFetching && (
                <ActivityIndicator
                  accessibilityLabel="Loading skill versions"
                  color={colors.primary}
                />
              )}
              {versions.isError && (
                <>
                  <Notice tone="error" message={failure(versions.error)} />
                  <ActionButton
                    label="Retry versions"
                    variant="secondary"
                    onPress={() => void versions.refetch()}
                  />
                </>
              )}
              {versions.data?.items.length === 0 && (
                <Label muted>
                  No published versions are visible through your Ornn access.
                </Label>
              )}
              {versions.data?.items.map((item) => (
                <Pressable
                  key={item.version}
                  accessibilityRole="radio"
                  accessibilityState={{
                    checked: selectedVersion?.version === item.version,
                  }}
                  accessibilityLabel={`Select version ${item.version}${item.deprecated ? ", deprecated" : ""}`}
                  disabled={busy}
                  onPress={() => setSelectedVersion(item)}
                  style={[
                    styles.versionRow,
                    {
                      backgroundColor: colors.card,
                      borderColor:
                        selectedVersion?.version === item.version
                          ? colors.primary
                          : colors.border,
                    },
                  ]}
                >
                  <View style={styles.flex}>
                    <View style={styles.versionHeading}>
                      <Label mono>{item.version}</Label>
                      {item.version === latestVersion?.version && (
                        <LatestBadge />
                      )}
                    </View>
                    <Label muted style={styles.caption}>
                      {item.deprecated
                        ? "Deprecated version"
                        : pin?.version === item.version
                          ? "Current pinned version"
                          : "Published version"}
                    </Label>
                  </View>
                  {selectedVersion?.version === item.version && (
                    <Check color={colors.primary} size={20} />
                  )}
                </Pressable>
              ))}
              {versions.data && versions.data.total_pages > 1 && (
                <Pagination
                  page={versionPage}
                  total={versions.data.total_pages}
                  disabled={busy || versions.isFetching}
                  onChange={(value) => {
                    setVersionPage(value);
                    setSelectedVersion(undefined);
                  }}
                />
              )}
              {selectedVersion && preview.isFetching && (
                <Label muted>Verifying package…</Label>
              )}
              {preview.isError && (
                <>
                  <Notice tone="error" message={failure(preview.error)} />
                  <ActionButton
                    label="Retry package preview"
                    variant="secondary"
                    onPress={() => void preview.refetch()}
                  />
                </>
              )}
              {preview.data && selectedVersion && !preview.isFetching && (
                <Card>
                  <Label muted>{preview.data.description}</Label>
                  <Label muted style={styles.caption}>
                    {preview.data.size_bytes.toLocaleString()} bytes ·{" "}
                    {preview.data.reference.dependencies.length} pinned
                    dependencies
                  </Label>
                  <Label mono style={styles.caption} selectable>
                    Skill ID: {preview.data.reference.skill_id}
                  </Label>
                  <Label mono style={styles.caption} selectable>
                    SHA-256: {preview.data.reference.sha256}
                  </Label>
                  {selectedVersion.deprecated && (
                    <Notice message="This version is deprecated. Choose a supported version unless you intentionally need this exact package." />
                  )}
                </Card>
              )}
              <ActionButton
                label={pin ? "Update exact version" : "Attach skill"}
                loading={busy}
                disabled={
                  !currentReady ||
                  Boolean(pin?.disabled) ||
                  !selectedVersion ||
                  !preview.data ||
                  preview.isFetching ||
                  (!pin && pins.length >= 16) ||
                  Boolean(
                    pin &&
                    preview.data &&
                    pin.version === preview.data.reference.version &&
                    pin.sha256 === preview.data.reference.sha256,
                  )
                }
                onPress={() => {
                  if (detail && selectedVersion && preview.data)
                    void change({
                      type: "attach",
                      skillId: detail.id,
                      selected: selectedVersion,
                      preview: preview.data,
                    });
                }}
              />
              {pin?.disabled && (
                <Notice message="Ask NyxBot to enable this skill before updating its exact version." />
              )}
              {!pin && pins.length >= 16 && (
                <Notice message="This agent already has 16 skills. Remove one from this agent before attaching another." />
              )}
            </>
          )}
          {view === "history" && (
            <>
              {history.isFetching && (
                <ActivityIndicator
                  accessibilityLabel="Loading skill history"
                  color={colors.primary}
                />
              )}
              {history.isError && (
                <>
                  <Notice tone="error" message={failure(history.error)} />
                  <ActionButton
                    label="Retry skill history"
                    variant="secondary"
                    onPress={() => void history.refetch()}
                  />
                </>
              )}
              {history.data?.items.length === 0 && (
                <Label muted>No skill history yet.</Label>
              )}
              {history.data?.items.map((item) => (
                <Card key={item.id}>
                  <Label>
                    {SKILL_HISTORY_LABELS[item.event_type] ?? "Skill activity"}
                  </Label>
                  <Label muted style={styles.caption}>
                    {new Date(item.created_at).toLocaleString()}
                    {item.revision !== null
                      ? ` · Revision ${item.revision}`
                      : ""}
                  </Label>
                  {item.skills.map((skill) => (
                    <Label key={skill.skill_id} muted>
                      {skill.name} · {skill.version}
                      {skill.disabled ? " · Disabled" : ""}
                    </Label>
                  ))}
                </Card>
              ))}
            </>
          )}
        </View>
      )}
    </View>
  );
}

function Pagination({
  page,
  total,
  disabled,
  onChange,
}: {
  page: number;
  total: number;
  disabled?: boolean;
  onChange: (page: number) => void;
}) {
  return (
    <View style={styles.row}>
      <View style={styles.flex}>
        <ActionButton
          label="Previous"
          variant="secondary"
          disabled={disabled || page <= 1}
          onPress={() => onChange(page - 1)}
        />
      </View>
      <Label muted style={styles.caption}>
        {page} / {total}
      </Label>
      <View style={styles.flex}>
        <ActionButton
          label="Next"
          variant="secondary"
          disabled={disabled || page >= total}
          onPress={() => onChange(page + 1)}
        />
      </View>
    </View>
  );
}
const styles = StyleSheet.create({
  versionHeading: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    flexWrap: "wrap",
  },
  stack: { gap: 16 },
  stackSmall: { gap: 8 },
  flex: { flex: 1, minWidth: 0 },
  caption: { fontSize: 11, lineHeight: 17 },
  section: { fontFamily: fonts.bodySemi, fontSize: 11, letterSpacing: 0.8 },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  agentContext: {
    minHeight: 64,
    borderRadius: 12,
    padding: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  agentName: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 20 },
  cardTitle: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 22 },
  icon: {
    width: 44,
    height: 44,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  detailIdentity: { flexDirection: "row", alignItems: "center", gap: 14 },
  excerpt: { padding: 12, borderRadius: 8, fontSize: 11, lineHeight: 18 },
  versionRow: {
    borderWidth: 1,
    borderRadius: 12,
    minHeight: 68,
    padding: 16,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
});
