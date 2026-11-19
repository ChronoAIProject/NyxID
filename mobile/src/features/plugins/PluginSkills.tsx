import { localizedError } from "../../i18n/localizedError";
import { useLocalizedState } from "../../i18n/useLocalizedState";
import { Pager } from "../../components/settings/Pager";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { AgentAvatar } from "../../components/AgentAvatar";
import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, Alert, StyleSheet, Text, View } from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Check, ChevronsUpDown } from "lucide-react-native";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
import { OfflineBanner } from "../../components/OfflineBanner";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
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
    : translateApp("The request could not be completed. Try again.");
/** The catalog and every pin belong to the explicitly selected agent. */
export function PluginSkills({ agentId }: { agentId?: string }) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const canFetch = focused && isConnected;
  const currentCanFetch = useRef(canFetch);
  currentCanFetch.current = canFetch;
  const epoch = getAuthEpoch();
  const [chosen, setChosen] = useState(agentId ?? "");
  const [picking, setPicking] = useState(false);
  const agents = useQuery({
    queryKey: ["workspace", "agents", "list"],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: canFetch,
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
      {!isConnected && !agent && (
        <OfflineBanner
          subtitle={t("Reconnect to load agents and their skills.")}
        />
      )}
      <SettingsGroup title={translateApp("Agent")}>
        <SettingsRow
          leading={agent ? <AgentAvatar agent={agent} size="md" /> : undefined}
          label={agent ? titleOf(agent) : translateApp("Choose an agent")}
          detail={
            agent
              ? agent.owner_kind === "org"
                ? (agent.owner_name ?? translateApp("Organization"))
                : translateApp("Personal")
              : undefined
          }
          accessibilityLabel={(agent ? translateApp("Agent for skills: {{agent}}", { agent: titleOf(agent) }) : translateApp("Agent for skills: Choose an agent"))}
          accessibilityHint={translateApp(
            "Opens a list of agents whose skills you can manage",
          )}
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
          label={translateApp("Agent for skills")}
          title={translateApp("Choose an agent")}
          options={manageable.map((item) => ({
            value: item.id,
            label: titleOf(item),
            leading: <AgentAvatar agent={item} size="lg" />,
            description:
              item.owner_kind === "org"
                ? (item.owner_name ?? translateApp("Organization"))
                : translateApp("Personal"),
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
      {isConnected && agents.isPending && (
        <SkeletonRows
          rows={3}
          title
          accessibilityLabel={translateApp("Loading agents for skills")}
        />
      )}
      {agents.isError && (
        <>
          <Notice tone="error" message={failure(agents.error)} />
          <SettingsGroup inset={false}>
            <SettingsRow
              label={translateApp("Retry agents")}
              tone="action"
              disabled={!canFetch}
              onPress={() => {
                if (currentCanFetch.current) void agents.refetch();
              }}
            />
          </SettingsGroup>
        </>
      )}
      {isConnected && !agents.isPending && !agents.isError && !agent && (
        <Text style={[styles.note, { color: colors.textMuted }]}>
          {chosen
            ? translateApp(
                "Requested agent unavailable. It may have been deleted or your maintenance access changed. Choose another agent to manage its skills.",
              )
            : translateApp("No agent you can maintain is available.")}
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
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const agent = useQuery({
    queryKey: ["workspace", "agents", "detail", route.params.agentId],
    queryFn: ({ signal }) => agentsApi.detail(route.params.agentId, signal),
    enabled: focused && isConnected,
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
      {!isConnected && !agent.data && (
        <OfflineBanner subtitle={t("Reconnect to load this skill's agent.")} />
      )}
      {isConnected && agent.isPending && (
        <SkeletonRows rows={3} title accessibilityLabel={translateApp("Loading skill agent")}
        />
      )}
      {agent.isError && <Notice tone="error" message={failure(agent.error)} />}
      {agent.data &&
        (agent.data.agent.can_maintain === false ||
          agent.data.agent.status === "destroyed") && (
          <Text style={[styles.note, { color: colors.textMuted }]}>
            {translateApp(
              "This agent is unavailable or your maintenance access changed.",
            )}
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
  useAppLanguage();
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
  const { t } = useAppLanguage();
  const currentTranslation = useRef(t);
  currentTranslation.current = t;
  const { colors } = useTheme();
  const client = useQueryClient();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const canFetch = focused && isConnected;
  // Native callbacks and confirmation alerts may outlive the render that
  // created them; recheck the current connection before issuing requests.
  const currentCanFetch = useRef(canFetch);
  currentCanFetch.current = canFetch;
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
  const [error, setError] = useLocalizedState<string>();
  const [notice, setNotice] = useLocalizedState<string>();
  const lock = useRef(false);
  const authEpoch = useRef(getAuthEpoch());
  const currentKey = [...ROOT, agent.id] as const;
  const current = useQuery({
    queryKey: currentKey,
    queryFn: ({ signal }) => api.current(agent.id, signal),
    enabled: canFetch,
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
    enabled: canFetch && !waitingForQuery,
    retry: false,
  });
  const versions = useQuery({
    queryKey: [...ROOT, "versions", detail?.id, versionPage],
    queryFn: ({ signal }) => api.versions(detail!.id, versionPage, signal),
    enabled: canFetch && Boolean(detail),
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
      canFetch && Boolean(detail && selectedVersion) && view === "versions",
    retry: false,
  });
  const tested = useQuery({
    queryKey: [...ROOT, "test", agent.id, detail?.id],
    queryFn: ({ signal }) => api.test(agent.id, detail!.id, signal),
    enabled: canFetch && testing && Boolean(detail),
    retry: false,
    staleTime: 0,
  });
  const history = useQuery({
    queryKey: [...ROOT, "history", agent.id],
    queryFn: ({ signal }) => api.history(agent.id, signal),
    enabled: canFetch && Boolean(detail) && view === "history",
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
    if (!currentCanFetch.current) return;
    await Promise.all([
      client.invalidateQueries({ queryKey: currentKey }),
      client.invalidateQueries({ queryKey: [...ROOT, "history", agent.id] }),
      client.invalidateQueries({ queryKey: [...ROOT, "test", agent.id] }),
    ]);
  }
  async function run(action: () => Promise<void>) {
    if (lock.current || !currentCanFetch.current) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      if (authEpoch.current !== getAuthEpoch())
        throw localizedError("Your account session changed. Reopen Plugins to continue.");
      await action();
    } catch (cause) {
      setError(() => (failure(cause)));
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
        throw localizedError("This agent can no longer be changed. Reload to review its current state.");
      assertSkillRevision(agent.id, baseline, latest);
      const selection = buildSkillSelection(latest, change);
      if (authEpoch.current !== getAuthEpoch())
        throw localizedError("Your account session changed. Reopen Plugins to continue.");
      if (!currentCanFetch.current)
        throw new Error(
          currentTranslation.current(
            translateApp("Reconnect before changing this agent's skills."),
          ),
        );
      await api.write(agent.id, selection);
      await refresh();
      setTesting(false);
      setNotice(
        () => (change.type === "detach"
          ? translateApp("Skill removed from agent.")
          : translateApp("Exact skill version attached.")),
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
  const cached = Boolean(
    current.data ||
    (!skillPage
      ? catalog.data
      : view === "versions"
        ? versions.data || preview.data
        : view === "history"
          ? history.data
          : versions.data),
  );
  function retry(refetch: () => unknown) {
    if (currentCanFetch.current) void refetch();
  }

  return (
    <View style={styles.stack}>
      {!isConnected && (
        <OfflineBanner
          subtitle={
            cached
              ? t(
                  "Showing cached skill information. Reconnect to refresh or make changes.",
                )
              : t("Reconnect to load skill information.")
          }
        />
      )}
      {!detail && notice && <Notice tone="success" message={notice} />}
      {!detail && error && <Notice tone="error" message={error} />}
      {!skillPage && (
        <SkillLibrary
          agentName={titleOf(agent)}
          offline={!isConnected}
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
            loading: isConnected && current.isPending,
            error:
              isConnected && current.isError
                ? failure(current.error)
                : undefined,
            onRetry: () => {
              retry(() => current.refetch());
            },
          }}
          catalog={{
            items: waitingForQuery ? [] : (catalog.data?.items ?? []),
            known: !waitingForQuery && Boolean(catalog.data),
            // A background refetch keeps the cached results; only a new
            // search or page (no data yet) shows the result skeleton.
            loading: isConnected && (waitingForQuery || catalog.isPending),
            error:
              isConnected && !waitingForQuery && catalog.isError
                ? failure(catalog.error)
                : undefined,
            onRetry: () => {
              retry(() => catalog.refetch());
            },
          }}
          page={page}
          totalPages={catalog.data?.total_pages ?? 1}
          onPageChange={(value) => {
            if (currentCanFetch.current) setPage(value);
          }}
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
                  label={translateApp("Reload current skills")}
                  tone="action"
                  disabled={busy || !canFetch}
                  onPress={() => {
                    if (currentCanFetch.current)
                      void refresh().then(() => setError(undefined));
                  }}
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
                    ? translateApp(
                        "Showing last loaded attached skills. Retry to confirm before changing versions or removing a skill.",
                      )
                    : undefined
                }
              >
                <SettingsRow
                  label={translateApp("Retry attached skills")}
                  tone="action"
                  disabled={busy || !canFetch || current.isFetching}
                  accessory={
                    isConnected && current.isFetching ? (
                      <ActivityIndicator color={colors.primary} />
                    ) : undefined
                  }
                  onPress={() => retry(() => current.refetch())}
                />
              </SettingsGroup>
            </>
          )}
          {view === "detail" && detail && (
            <>
              <SettingsGroup
                footer={
                  metadata?.description || detail.description || undefined
                }
              >
                <SettingsRow
                  icon={BookOpen}
                  label={detail.name}
                  detail={
                    pin ? translateApp("Attached to {{title}}", {
                          title: titleOf(agent),
                        })
                      : translateApp("Ornn catalog")
                  }
                />
              </SettingsGroup>
              <SettingsGroup
                title={translateApp("Version")}
                inset={false}
                footer={
                  pin?.disabled
                    ? translateApp(
                        "This skill is disabled. Ask NyxBot to enable it before changing its version.",
                      )
                    : undefined
                }
              >
                <SettingsRow
                  label={
                    current.isError && current.data
                      ? translateApp("Last loaded version")
                      : translateApp("Pinned version")
                  }
                  detail={pinnedLatest ? translateApp("Latest version") : undefined}
                  value={
                    pin?.version ??
                    (isConnected && current.isPending
                      ? translateApp("Loading attached version…")
                      : currentReady
                        ? translateApp("Not attached")
                        : translateApp("Attachment status unavailable"))
                  }
                />
                {pin && metadata?.size_bytes !== undefined && (
                  <SettingsRow
                    label={translateApp("Package size")}
                    value={translateApp("{{size}} bytes", {
                      size: metadata.size_bytes.toLocaleString(appLocale()),
                    })}
                  />
                )}
                {pin && (
                  <SettingsRow
                    label={translateApp("Pinned dependencies")}
                    value={String(pin.dependencies.length)}
                  />
                )}
                {latestVersion && !pinnedLatest && (
                  <SettingsRow
                    label={translateApp("Use latest (v{{version}})", {
                      version: latestVersion.version,
                    })}
                    tone="action"
                    disabled={busy || !currentReady || Boolean(pin?.disabled)}
                    onPress={() => {
                      openView("versions", latestVersion.version);
                    }}
                  />
                )}
                <SettingsRow
                  label={pin ? translateApp("Choose exact version")
                      : translateApp("Attach exact version")
                  }
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
                footer={translateApp("To change this skill, ask NyxBot.")}
              >
                <SettingsRow
                  label={translateApp("Ask NyxBot")}
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
                    title={translateApp("Test attached skill")}
                    inset={false}
                    footer={translateApp(
                      "Read the exact package this agent uses.",
                    )}
                  >
                    <SettingsRow
                      label={
                        isConnected && tested.isFetching && testing
                          ? translateApp("Testing attached skill…")
                          : translateApp("Read pinned package")
                      }
                      tone="action"
                      disabled={
                        busy || !canFetch || (testing && tested.isFetching)
                      }
                      accessory={
                        isConnected && testing && tested.isFetching ? (
                          <ActivityIndicator color={colors.primary} />
                        ) : undefined
                      }
                      onPress={() => {
                        if (!currentCanFetch.current) return;
                        if (testing) void tested.refetch();
                        else setTesting(true);
                      }}
                    />
                    {testing &&
                    tested.data &&
                    (!tested.isFetching || !isConnected) &&
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
                      message={translateApp("Test failed: {{failure}}", {
                        failure: failure(tested.error),
                      })}
                    />
                  )}
                  {testing &&
                    tested.data &&
                    (!tested.isFetching || !isConnected) &&
                    !tested.isError && (
                      <Notice
                        tone={tested.data.readable ? "success" : "info"}
                        message={
                          tested.data.readable
                            ? translateApp(
                                "Test passed: the agent reads {{name}} {{version}} ({{bytes}} bytes).",
                                {
                                  name: tested.data.name,
                                  version: tested.data.version,
                                  bytes:
                                    tested.data.size_bytes.toLocaleString(
                                      appLocale(),
                                    ),
                                },
                              )
                            : translateApp(
                                "{{name}} {{version}} is disabled, so the agent does not read it.",
                                {
                                  name: tested.data.name,
                                  version: tested.data.version,
                                },
                              )
                        }
                      />
                    )}
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label={translateApp("Skill history")}
                      disabled={busy}
                      onPress={() => openView("history")}
                    />
                  </SettingsGroup>
                  <SettingsGroup
                    inset={false}
                    footer={translateApp(
                      "Other attached skills stay in place. The skill stays in Ornn.",
                    )}
                  >
                    <SettingsRow
                      label={translateApp("Remove from agent")}
                      tone="danger"
                      disabled={busy || !canFetch || !currentReady}
                      onPress={() =>
                        confirmRemoval(
                          pin.name,
                          titleOf(agent),
                          () =>
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
              {isConnected && versions.isPending && (
                <SkeletonRows
                  rows={4}
                  icon={false}
                  title
                  accessibilityLabel={translateApp("Loading skill versions")}
                />
              )}
              {versions.isError && (
                <>
                  <Notice tone="error" message={failure(versions.error)} />
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label={translateApp("Retry versions")}
                      tone="action"
                      disabled={!canFetch}
                      onPress={() => retry(() => versions.refetch())}
                    />
                  </SettingsGroup>
                </>
              )}
              {!isConnected && !versions.data && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  {t("Reconnect to load published skill versions.")}
                </Text>
              )}
              {versions.data?.items.length === 0 && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  {translateApp(
                    "No published versions are visible through your Ornn access.",
                  )}
                </Text>
              )}
              {versions.data?.items.length ? (
                <SettingsGroup
                  title={translateApp("Versions")}
                  inset={false}
                  footer={translateApp(
                    "Choose which version of {{name}} this agent uses. Skills change how the agent works.",
                    { name: detail.name},
                  )}
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
                            ? translateApp("Deprecated version")
                            : pin?.version === item.version
                              ? translateApp("Current pinned version")
                              : translateApp("Published version")
                        }
                        value={
                          item.version === latestVersion?.version
                            ? translateApp("Latest")
                            : undefined
                        }
                        chevron={false}
                        accessibilityRole="radio"
                        accessibilityState={{ checked: selected, selected }}
                        accessibilityLabel={(item.deprecated ? translateApp("Select version {{version}}, deprecated", { version: item.version }) : translateApp("Select version {{version}}", { version: item.version }))}
                        accessibilityHint={selected ? translateApp("Selected") : undefined}
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
                <SettingsGroup inset={false}>
                  <Pager
                    page={versionPage}
                    total={versions.data.total_pages}
                    disabled={busy || !canFetch || versions.isFetching}
                    onChange={(value) => {
                      if (!currentCanFetch.current) return;
                      setVersionPage(value);
                      setSelectedVersion(undefined);
                    }}
                  />
                </SettingsGroup>
              )}
              {isConnected && selectedVersion && preview.isFetching && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  {translateApp("Verifying package…")}
                </Text>
              )}
              {preview.isError && (
                <>
                  <Notice tone="error" message={failure(preview.error)} />
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label={translateApp("Retry package preview")}
                      tone="action"
                      disabled={!canFetch}
                      onPress={() => retry(() => preview.refetch())}
                    />
                  </SettingsGroup>
                </>
              )}
              {!isConnected && selectedVersion && !preview.data && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  {t("Reconnect to load this package preview.")}
                </Text>
              )}
              {preview.data &&
                selectedVersion &&
                (!preview.isFetching || !isConnected) && (
                  <SettingsGroup
                    title={translateApp("Package")}
                    inset={false}
                    footer={
                      selectedVersion.deprecated
                        ? translateApp(
                            "This version is deprecated. Choose a supported version unless you intentionally need this exact package.",
                          )
                        : preview.data.description || undefined
                    }
                  >
                    <SettingsRow
                      label={translateApp("Size")}
                      value={translateApp("{{size}} bytes", {
                        size:
                          preview.data.size_bytes.toLocaleString(appLocale()),
                      })}
                    />
                    <SettingsRow
                      label={translateApp("Pinned dependencies")}
                      value={String(preview.data.reference.dependencies.length)}
                    />
                    <SettingsRow
                      label={preview.data.reference.skill_id}
                      detail={translateApp("Skill ID")}
                      mono
                      selectable
                    />
                    <SettingsRow
                      label={preview.data.reference.sha256}
                      detail={"SHA-256"}
                      mono
                      selectable
                    />
                  </SettingsGroup>
                )}
              <SettingsGroup
                inset={false}
                footer={
                  pin?.disabled
                    ? translateApp(
                        "Ask NyxBot to enable this skill before updating its exact version.",
                      )
                    : !pin && pins.length >= 16
                      ? translateApp(
                          "This agent already has 16 skills. Remove one from this agent before attaching another.",
                        )
                      : undefined
                }
              >
                <SettingsRow
                  label={pin ? translateApp("Update exact version")
                      : translateApp("Attach skill")
                  }
                  tone="action"
                  accessory={
                    busy ? (
                      <ActivityIndicator color={colors.primary} />
                    ) : undefined
                  }
                  disabled={
                    busy ||
                    !canFetch ||
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
              {isConnected && history.isPending && (
                <SkeletonRows
                  rows={4}
                  title
                  accessibilityLabel={translateApp("Loading skill history")}
                />
              )}
              {history.isError && (
                <>
                  <Notice tone="error" message={failure(history.error)} />
                  <SettingsGroup inset={false}>
                    <SettingsRow
                      label={translateApp("Retry skill history")}
                      tone="action"
                      disabled={!canFetch}
                      onPress={() => retry(() => history.refetch())}
                    />
                  </SettingsGroup>
                </>
              )}
              {!isConnected && !history.data && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  {t("Reconnect to load skill history.")}
                </Text>
              )}
              {history.data?.items.length === 0 && (
                <Text style={[styles.note, { color: colors.textMuted }]}>
                  {translateApp("No skill history yet.")}
                </Text>
              )}
              {history.data?.items.length ? (
                <SettingsGroup title={translateApp("History")} inset={false}>
                  {history.data.items.map((item) => (
                    <SettingsRow
                      key={item.id}
                      label={
                        SKILL_HISTORY_LABELS[item.event_type] ??
                        translateApp("Skill activity")
                      }
                      detail={[
                        (item.revision !== null ? translateApp("{{time}} · Revision {{count}}", { time: new Date(item.created_at).toLocaleString(appLocale()), count: item.revision}) : new Date(item.created_at).toLocaleString(appLocale())),
                        ...item.skills.map(
                          (skill) =>
                            (skill.disabled ? translateApp("{{name}} · {{version}} · Disabled", { name: skill.name, version: skill.version}) : translateApp("{{name}} · {{version}}", { name: skill.name, version: skill.version })),
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
    translateApp("Remove from agent?"),
    translateApp(
      "Remove {{skill}} from {{agent}}? Other attached skills stay in place. The skill stays in Ornn.",
      {skill: skill, agent: agent },
    ),
    [
      { text: translateApp("Cancel"), style: "cancel" },
      { text: translateApp("Remove"), style: "destructive", onPress: onConfirm },
    ],
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
