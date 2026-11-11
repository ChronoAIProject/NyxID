import { AgentAvatar } from "../../components/AgentAvatar";
import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Check, ChevronsUpDown } from "lucide-react-native";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import {
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { pluginSkillsApi as api } from "../../lib/api/pluginSkillsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { spacing, typeScale } from "../../theme/designTokens";
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
import { SkillLibrary } from "./SkillLibrary";
import {
  assertSkillRevision,
  buildSkillSelection,
  SKILL_HISTORY_LABELS,
  type SkillChange,
  type SkillVersion,
} from "./pluginSkillsTypes";

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
      <SettingsGroup title="Agent">
        <SettingsRow
          leading={agent ? <AgentAvatar agent={agent} size="md" /> : undefined}
          label={agent ? titleOf(agent) : "Choose an agent"}
          detail={
            agent
              ? agent.owner_kind === "org"
                ? (agent.owner_name ?? "Organization")
                : "Personal"
              : undefined
          }
          accessibilityLabel={`Agent for skills: ${agent ? titleOf(agent) : "Choose an agent"}`}
          accessibilityHint="Opens a list of agents whose skills you can manage"
          accessibilityState={{
            expanded: picking,
            disabled: !manageable.length,
          }}
          disabled={!manageable.length}
          accessory={<ChevronsUpDown size={16} color={colors.textMuted} />}
          onPress={() => setPicking((value) => !value)}
        />
      </SettingsGroup>
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
      {agents.isPending && (
        <SkeletonRows
          rows={3}
          title
          accessibilityLabel="Loading agents for skills"
        />
      )}
      {agents.isError && (
        <>
          <Notice tone="error" message={failure(agents.error)} />
          <SettingsGroup inset={false}>
            <SettingsRow
              label="Retry agents"
              tone="action"
              onPress={() => void agents.refetch()}
            />
          </SettingsGroup>
        </>
      )}
      {!agents.isPending && !agents.isError && !agent && (
        <Text style={[styles.note, { color: colors.textMuted }]}>
          {chosen
            ? "Requested agent unavailable. It may have been deleted or your maintenance access changed. Choose another agent to manage its skills."
            : "No agent you can maintain is available."}
        </Text>
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
  const { colors } = useTheme();
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
    <WorkspacePage
      title={route.params.name}
      showHeading={false}
      contentStyle={styles.grouped}
    >
      {agent.isPending && (
        <SkeletonRows rows={3} title accessibilityLabel="Loading skill agent" />
      )}
      {agent.isError && <Notice tone="error" message={failure(agent.error)} />}
      {agent.data &&
        (agent.data.agent.can_maintain === false ||
          agent.data.agent.status === "destroyed") && (
          <Text style={[styles.note, { color: colors.textMuted }]}>
            This agent is unavailable or your maintenance access changed.
          </Text>
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

/** Back stays native, but cannot leave while a pin write is in flight. */
function SkillPageGuard({
  busy,
  onBack,
}: {
  busy: boolean;
  onBack: () => void;
}) {
  useWorkspaceBack({
    active: false,
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
    enabled: focused && !waitingForQuery,
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
          query={catalogInput}
          onQueryChange={setCatalogInput}
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
            // A background refetch keeps the cached results; only a new
            // search or page (no data yet) shows the result skeleton.
            loading: waitingForQuery || catalog.isPending,
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
          <SkillPageGuard busy={busy} onBack={() => navigation.goBack()} />
          {error && (
            <>
              <Notice tone="error" message={error} />
              <SettingsGroup inset={false}>
                <SettingsRow
                  label="Reload current skills"
                  tone="action"
                  disabled={busy}
                  onPress={() => void refresh().then(() => setError(undefined))}
                />
              </SettingsGroup>
            </>
          )}
          {notice && <Notice tone="success" message={notice} />}
          {current.isError && (
            <>
              <Notice tone="error" message={failure(current.error)} />
              <SettingsGroup
                inset={false}
                footer={
                  current.data
                    ? "Showing last loaded attached skills. Retry to confirm before changing versions or removing a skill."
                    : undefined
                }
              >
                <SettingsRow
                  label="Retry attached skills"
                  tone="action"
                  disabled={busy || !focused || current.isFetching}
                  accessory={
                    current.isFetching ? (
                      <ActivityIndicator color={colors.primary} />
                    ) : undefined
                  }
                  onPress={() => void current.refetch()}
                />
              </SettingsGroup>
            </>
          )}
          {view === "detail" && detail && (
            <>
              <SettingsGroup
                footer={metadata?.description || detail.description || undefined}
              >
                <SettingsRow
                  icon={BookOpen}
                  label={detail.name}
                  detail={pin ? `Attached to ${titleOf(agent)}` : "Ornn catalog"}
                />
              </SettingsGroup>
              <SettingsGroup
                title="Version"
                inset={false}
                footer={
                  pin?.disabled
                    ? "This skill is disabled. Ask NyxBot to enable it before changing its version."
                    : undefined
                }
              >
                <SettingsRow
                  label={
                    current.isError && current.data
                      ? "Last loaded version"
                      : "Pinned version"
                  }
                  detail={pinnedLatest ? "Latest version" : undefined}
                  value={
                    pin?.version ??
                    (current.isPending
                      ? "Loading attached version…"
                      : currentReady
                        ? "Not attached"
                        : "Attachment status unavailable")
                  }
                />
                {pin && metadata?.size_bytes !== undefined && (
                  <SettingsRow
                    label="Package size"
                    value={`${metadata.size_bytes.toLocaleString()} bytes`}
                  />
                )}
                {pin && (
                  <SettingsRow
                    label="Pinned dependencies"
                    value={String(pin.dependencies.length)}
                  />
                )}
                {latestVersion && !pinnedLatest && (
                  <SettingsRow
                    label={`Use latest (v${latestVersion.version})`}
                    tone="action"
                    disabled={busy || !currentReady || Boolean(pin?.disabled)}
                    onPress={() => {
                      openView("versions", latestVersion.version);
                    }}
                  />
                )}
                <SettingsRow
                  label={pin ? "Choose exact version" : "Attach exact version"}
                  disabled={busy || !currentReady}
                  onPress={() => {
                    openView("versions");
                    setVersionPage(1);
                    setSelectedVersion(undefined);
                  }}
                />
              </SettingsGroup>
              <SettingsGroup
                inset={false}
                footer="To change this skill, ask NyxBot."
              >
                <SettingsRow
                  label="Ask NyxBot"
                  tone="action"
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
              </SettingsGroup>
              {pin && (
                <>
                  <SettingsGroup
                    title="Test attached skill"
                    inset={false}
                    footer="Read the exact package this agent uses."
                  >
                    <SettingsRow
                      label={
                        tested.isFetching && testing
                          ? "Testing attached skill…"
                          : "Read pinned package"
                      }
                      tone="action"
                      disabled={busy || (testing && tested.isFetching)}
                      accessory={
                        testing && tested.isFetching ? (
                          <ActivityIndicator color={colors.primary} />
                        ) : undefined
                      }
                      onPress={() => {
                        if (testing) void tested.refetch();
                        else setTesting(true);
                      }}
                    />
                    {testing &&
                      tested.data &&
                      !tested.isFetching &&
                      !tested.isError &&
                      tested.data.excerpt ? (
                      <View style={styles.excerpt}>
                        <Label selectable mono style={styles.excerptText}>
                          {tested.data.excerpt}
                        </Label>
                      </View>
                    ) : null}
                  </SettingsGroup>
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
                      <Notice
                        tone={tested.data.readable ? "success" : "info"}
                        message={
                          tested.data.readable
                            ? `Test passed: the agent reads ${tested.data.name} ${tested.data.version} (${tested.data.size_bytes.toLocaleString()} bytes).`
                            : `${tested.data.name} ${tested.data.version} is disabled, so the agent does not read it.`
                        }
                      />
                    )}
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label="Skill history"
                      disabled={busy}
                      onPress={() => openView("history")}
                    />
                  </SettingsGroup>
                  <SettingsGroup
                    inset={false}
                    footer="Other attached skills stay in place. The skill stays in Ornn."
                  >
                    <SettingsRow
                      label="Remove from agent"
                      tone="danger"
                      disabled={busy || !currentReady}
                      onPress={() =>
                        confirmRemoval(pin.name, titleOf(agent), () =>
                          void change({
                            type: "detach",
                            skillId: pin.skill_id,
                          }),
                        )
                      }
                    />
                  </SettingsGroup>
                </>
              )}
            </>
          )}
          {view === "versions" && detail && (
            <>
              {versions.isPending && (
                <SkeletonRows
                  rows={4}
                  icon={false}
                  title
                  accessibilityLabel="Loading skill versions"
                />
              )}
              {versions.isError && (
                <>
                  <Notice tone="error" message={failure(versions.error)} />
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label="Retry versions"
                      tone="action"
                      onPress={() => void versions.refetch()}
                    />
                  </SettingsGroup>
                </>
              )}
              {versions.data?.items.length === 0 && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  No published versions are visible through your Ornn access.
                </Text>
              )}
              {versions.data?.items.length ? (
                <SettingsGroup
                  title="Versions"
                  inset={false}
                  footer={`Choose which version of ${detail.name} this agent uses. Skills change how the agent works.`}
                >
                  {versions.data.items.map((item) => {
                    const selected = selectedVersion?.version === item.version;
                    return (
                      <SettingsRow
                        key={item.version}
                        label={item.version}
                        mono
                        detail={
                          item.deprecated
                            ? "Deprecated version"
                            : pin?.version === item.version
                              ? "Current pinned version"
                              : "Published version"
                        }
                        value={
                          item.version === latestVersion?.version
                            ? "Latest"
                            : undefined
                        }
                        chevron={false}
                        accessibilityRole="radio"
                        accessibilityState={{ checked: selected, selected }}
                        accessibilityLabel={`Select version ${item.version}${item.deprecated ? ", deprecated" : ""}`}
                        accessibilityHint={selected ? "Selected" : undefined}
                        disabled={busy}
                        onPress={() => setSelectedVersion(item)}
                        accessory={
                          selected ? (
                            <Check
                              color={colors.primary}
                              size={18}
                              strokeWidth={2.5}
                            />
                          ) : (
                            <View style={styles.checkSpace} />
                          )
                        }
                      />
                    );
                  })}
                </SettingsGroup>
              ) : null}
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
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  Verifying package…
                </Text>
              )}
              {preview.isError && (
                <>
                  <Notice tone="error" message={failure(preview.error)} />
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label="Retry package preview"
                      tone="action"
                      onPress={() => void preview.refetch()}
                    />
                  </SettingsGroup>
                </>
              )}
              {preview.data && selectedVersion && !preview.isFetching && (
                <SettingsGroup
                  title="Package"
                  inset={false}
                  footer={
                    selectedVersion.deprecated
                      ? "This version is deprecated. Choose a supported version unless you intentionally need this exact package."
                      : preview.data.description || undefined
                  }
                >
                  <SettingsRow
                    label="Size"
                    value={`${preview.data.size_bytes.toLocaleString()} bytes`}
                  />
                  <SettingsRow
                    label="Pinned dependencies"
                    value={String(preview.data.reference.dependencies.length)}
                  />
                  <SettingsRow
                    label={preview.data.reference.skill_id}
                    detail="Skill ID"
                    mono
                    selectable
                  />
                  <SettingsRow
                    label={preview.data.reference.sha256}
                    detail="SHA-256"
                    mono
                    selectable
                  />
                </SettingsGroup>
              )}
              <SettingsGroup
                inset={false}
                footer={
                  pin?.disabled
                    ? "Ask NyxBot to enable this skill before updating its exact version."
                    : !pin && pins.length >= 16
                      ? "This agent already has 16 skills. Remove one from this agent before attaching another."
                      : undefined
                }
              >
                <SettingsRow
                  label={pin ? "Update exact version" : "Attach skill"}
                  tone="action"
                  accessory={
                    busy ? <ActivityIndicator color={colors.primary} /> : undefined
                  }
                  disabled={
                    busy ||
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
              </SettingsGroup>
            </>
          )}
          {view === "history" && (
            <>
              {history.isPending && (
                <SkeletonRows
                  rows={4}
                  title
                  accessibilityLabel="Loading skill history"
                />
              )}
              {history.isError && (
                <>
                  <Notice tone="error" message={failure(history.error)} />
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label="Retry skill history"
                      tone="action"
                      onPress={() => void history.refetch()}
                    />
                  </SettingsGroup>
                </>
              )}
              {history.data?.items.length === 0 && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  No skill history yet.
                </Text>
              )}
              {history.data?.items.length ? (
                <SettingsGroup title="History" inset={false}>
                  {history.data.items.map((item) => (
                    <SettingsRow
                      key={item.id}
                      label={
                        SKILL_HISTORY_LABELS[item.event_type] ?? "Skill activity"
                      }
                      detail={[
                        `${new Date(item.created_at).toLocaleString()}${
                          item.revision !== null
                            ? ` · Revision ${item.revision}`
                            : ""
                        }`,
                        ...item.skills.map(
                          (skill) =>
                            `${skill.name} · ${skill.version}${skill.disabled ? " · Disabled" : ""}`,
                        ),
                      ].join("\n")}
                    />
                  ))}
                </SettingsGroup>
              ) : null}
            </>
          )}
        </View>
      )}
    </View>
  );
}

/** iOS confirms removal in a system alert; the destructive choice is explicit. */
function confirmRemoval(skill: string, agent: string, onConfirm: () => void) {
  Alert.alert(
    "Remove from agent?",
    `Remove ${skill} from ${agent}? Other attached skills stay in place. The skill stays in Ornn.`,
    [
      { text: "Cancel", style: "cancel" },
      { text: "Remove", style: "destructive", onPress: onConfirm },
    ],
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
    <SettingsGroup title={`Page ${page} / ${total}`} inset={false}>
      <SettingsRow
        label="Previous"
        tone="action"
        disabled={disabled || page <= 1}
        onPress={() => onChange(page - 1)}
      />
      <SettingsRow
        label="Next"
        tone="action"
        disabled={disabled || page >= total}
        onPress={() => onChange(page + 1)}
      />
    </SettingsGroup>
  );
}
const styles = StyleSheet.create({
  grouped: {
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.sm,
    paddingBottom: spacing.huge * 2,
    gap: spacing.huge,
  },
  stack: { gap: spacing.huge },
  note: { ...typeScale.small, paddingHorizontal: spacing.xxl },
  checkSpace: { width: 18 },
  excerpt: {
    paddingHorizontal: spacing.xxl,
    paddingVertical: spacing.md,
  },
  excerptText: { fontSize: 11, lineHeight: 18 },
});
