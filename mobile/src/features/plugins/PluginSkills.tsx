import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  AppState,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import NetInfo from "@react-native-community/netinfo";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  BookOpen,
  Bot,
  Check,
  ChevronDown,
  ChevronLeft,
  X,
} from "lucide-react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { WorkspaceSelectionSheet } from "../../components/workspace/WorkspaceSelectionField";
import {
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { assistantApi } from "../../lib/api/assistantApi";
import { pluginSkillsApi as api } from "../../lib/api/pluginSkillsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { AssistantAgent } from "../agents/agentContracts";
import { submitSkillDraft } from "./skillDraftSubmission";
import { skillModalBackAction } from "./skillModalNavigation";
import { SkillLibrary, type SkillLibraryTab } from "./SkillLibrary";
import {
  assertSkillRevision,
  buildSkillSelection,
  SKILL_HISTORY_LABELS,
  type SkillChange,
  type SkillDraft,
  type SkillProposal,
  type SkillVersion,
} from "./pluginSkillsTypes";
import {
  skillDraftDiagnostics,
  skillDraftDiagnosticText,
  type SkillDraftDiagnostic,
} from "./authoredSkillReviewState";

const ROOT = ["workspace", "plugins", "skills"] as const;
const titleOf = (agent: AssistantAgent) => agent.display_name || agent.name;
const failure = (cause: unknown) =>
  cause instanceof Error
    ? cause.message
    : "The request could not be completed. Try again.";
type OpenChat = (context: {
  agentId: string;
  conversationId?: string;
  acknowledgementId?: string;
  openLatest?: boolean;
}) => void;

/** The catalog and every pin belong to the explicitly selected agent. */
export function PluginSkills({
  agentId,
  onOpenChat,
  onDetailChange,
}: {
  agentId?: string;
  onOpenChat?: OpenChat;
  onDetailChange?: (open: boolean) => void;
}) {
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
  const features = useQuery({
    queryKey: [...ROOT, "features"],
    queryFn: ({ signal }) => api.features(signal),
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
          <View
            style={[
              styles.agentIcon,
              { backgroundColor: colors.primaryTone.bg },
            ]}
          >
            <Bot size={18} color={colors.primaryOnTint} />
          </View>
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
                setChosen(values[0]);
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
      {features.isError && (
        <Notice message="Optional skill controls could not be loaded. Attachments and exact versions remain available." />
      )}
      {agent && (
        <AgentSkillsManager
          key={agent.id}
          agent={agent}
          features={features.data ?? []}
          onOpenChat={onOpenChat}
          onDetailChange={onDetailChange}
        />
      )}
    </View>
  );
}

function AgentSkillsManager({
  agent,
  features,
  onOpenChat,
  onDetailChange,
}: {
  agent: AssistantAgent;
  features: string[];
  onOpenChat?: OpenChat;
  onDetailChange?: (open: boolean) => void;
}) {
  const { colors } = useTheme();
  const client = useQueryClient();
  const focused = useIsFocused();
  const [libraryTab, setLibraryTab] = useState<SkillLibraryTab>("added");
  const [addedQuery, setAddedQuery] = useState("");
  const [catalogInput, setCatalogInput] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [page, setPage] = useState(1);
  const [detail, setDetail] = useState<{
    id: string;
    name: string;
    description?: string | null;
  }>();
  const [view, setView] = useState<"detail" | "versions" | "history" | "draft">(
    "detail",
  );
  const [versionPage, setVersionPage] = useState(1);
  const [selectedVersion, setSelectedVersion] = useState<SkillVersion>();
  const [testing, setTesting] = useState(false);
  const [confirmDetach, setConfirmDetach] = useState(false);
  const [proposalId, setProposalId] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const lock = useRef(false);
  const editorLock = useRef(false);
  const authEpoch = useRef(getAuthEpoch());
  const currentKey = [...ROOT, agent.id] as const;
  const current = useQuery({
    queryKey: currentKey,
    queryFn: ({ signal }) => api.current(agent.id, signal),
    enabled: focused,
    retry: false,
  });
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
    enabled: focused && Boolean(detail) && view !== "draft",
    retry: false,
  });
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
  const proposals = useQuery({
    queryKey: [...ROOT, "proposals", agent.id],
    queryFn: ({ signal }) => api.proposals(agent.id, signal),
    enabled: focused && Boolean(detail) && view === "draft",
    retry: false,
  });
  useEffect(() => {
    onDetailChange?.(Boolean(detail));
    return () => onDetailChange?.(false);
  }, [Boolean(detail), onDetailChange]);

  function openSkill(id: string, name: string, description?: string | null) {
    setDetail({ id, name, description });
    setView("detail");
    setSelectedVersion(undefined);
    setVersionPage(1);
    setTesting(false);
    setConfirmDetach(false);
    setProposalId(undefined);
    setError(undefined);
    setNotice(undefined);
  }
  function close() {
    if (!lock.current && !editorLock.current && !busy) setDetail(undefined);
  }
  function back() {
    const action = skillModalBackAction(
      view !== "detail",
      lock.current || editorLock.current || busy,
    );
    if (action === "close") close();
    if (action === "parent") {
      setView("detail");
      setConfirmDetach(false);
    }
  }
  function editorBusy(value: boolean) {
    editorLock.current = value;
    setBusy(value);
  }
  async function refresh() {
    await Promise.all([
      client.invalidateQueries({ queryKey: currentKey }),
      client.invalidateQueries({ queryKey: [...ROOT, "history", agent.id] }),
      client.invalidateQueries({ queryKey: [...ROOT, "test", agent.id] }),
    ]);
  }
  async function run(action: () => Promise<void>) {
    if (lock.current || editorLock.current) return;
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
    if (!baseline) return;
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
          ? "Skill detached."
          : change.type === "toggle"
            ? "Skill status updated."
            : "Exact skill version attached.",
      );
      if (change.type === "detach") setDetail(undefined);
      else if (change.type === "attach") setView("detail");
    });
  }
  const pins = current.data?.skills ?? [];
  const pin = pins.find((item) => item.skill_id === detail?.id);
  const metadata = detail ? current.data?.metadata[detail.id] : undefined;
  const canDisable = features.includes("assistant:skill-pin-disable");
  const canImprove = features.includes("assistant:agent-learning");
  const proposal = proposals.data?.proposals.find(
    (item) => item.id === proposalId,
  );

  return (
    <View style={styles.stack}>
      {!detail && notice && <Notice tone="success" message={notice} />}
      {!detail && error && <Notice tone="error" message={error} />}
      <SkillLibrary
        agentName={titleOf(agent)}
        tab={libraryTab}
        onTabChange={setLibraryTab}
        query={libraryTab === "added" ? addedQuery : catalogInput}
        onQueryChange={libraryTab === "added" ? setAddedQuery : setCatalogInput}
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
      <Modal
        visible={Boolean(detail)}
        animationType="slide"
        presentationStyle="pageSheet"
        onRequestClose={back}
      >
        <SafeAreaProvider>
          <SafeAreaView style={[styles.modal, { backgroundColor: colors.bg }]}>
            <View style={[styles.modalHeader, { borderColor: colors.border }]}>
              {view !== "detail" ? (
                <Pressable
                  style={styles.iconButton}
                  accessibilityRole="button"
                  accessibilityLabel="Back to skill"
                  disabled={busy}
                  onPress={back}
                >
                  <ChevronLeft size={22} color={colors.textPrimary} />
                </Pressable>
              ) : (
                <View style={styles.iconButton} accessible={false} />
              )}
              <Text
                numberOfLines={1}
                accessibilityRole="header"
                style={[styles.modalTitle, { color: colors.textPrimary }]}
              >
                {view === "versions"
                  ? "Skill versions"
                  : view === "history"
                    ? "Skill history"
                    : view === "draft"
                      ? "Edit skill draft"
                      : detail?.name}
              </Text>
              <Pressable
                style={styles.iconButton}
                accessibilityRole="button"
                accessibilityLabel="Close skill"
                disabled={busy}
                onPress={close}
              >
                <X size={21} color={colors.textSecondary} />
              </Pressable>
            </View>
            <ScrollView
              keyboardShouldPersistTaps="handled"
              automaticallyAdjustKeyboardInsets
              contentContainerStyle={styles.modalContent}
            >
              {error && (
                <>
                  <Notice tone="error" message={error} />
                  <ActionButton
                    label="Reload current skills"
                    variant="secondary"
                    disabled={busy}
                    onPress={() =>
                      void refresh().then(() => setError(undefined))
                    }
                  />
                </>
              )}
              {notice && <Notice tone="success" message={notice} />}
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
                      Pinned version
                    </Label>
                    <Label mono>{pin?.version ?? "Not attached"}</Label>
                    {pin && (
                      <Label muted style={styles.caption}>
                        {metadata?.size_bytes !== undefined
                          ? `${metadata.size_bytes.toLocaleString()} bytes · `
                          : ""}
                        {pin.dependencies.length} pinned dependencies
                      </Label>
                    )}
                  </Card>
                  {pin?.disabled && (
                    <Notice
                      message={
                        canDisable
                          ? "Enable this skill before updating its version. Changing a version will never turn it on automatically."
                          : "This skill is disabled. Version updates require it to be enabled first; enabling is unavailable for your current rollout."
                      }
                    />
                  )}
                  <ActionButton
                    label={
                      pin ? "Choose exact version" : "Attach exact version"
                    }
                    variant="secondary"
                    disabled={busy || !current.data}
                    onPress={() => {
                      setView("versions");
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
                      {canDisable && (
                        <Card>
                          <View style={styles.row}>
                            <View style={styles.flex}>
                              <Label>
                                {pin.disabled ? "Disabled" : "Enabled"}
                              </Label>
                              <Label muted style={styles.caption}>
                                Use this skill in future agent turns
                              </Label>
                            </View>
                            <Switch
                              accessibilityLabel={`Use ${pin.name}`}
                              value={!pin.disabled}
                              disabled={busy || !current.data}
                              onValueChange={(enabled) =>
                                void change({
                                  type: "toggle",
                                  skillId: pin.skill_id,
                                  disabled: !enabled,
                                })
                              }
                            />
                          </View>
                        </Card>
                      )}
                      {canImprove && (
                        <Card>
                          <Label>Edit / improve skill</Label>
                          <Label muted style={styles.caption}>
                            Edit a copy. The current attached version stays in
                            use until you publish from its confirmation card.
                          </Label>
                          <ActionButton
                            label="Review draft"
                            variant="secondary"
                            loading={busy}
                            onPress={() =>
                              void run(async () => {
                                const result = await api.improve(
                                  agent.id,
                                  pin.skill_id,
                                );
                                setProposalId(result.proposal_id);
                                setView("draft");
                                await client.invalidateQueries({
                                  queryKey: [...ROOT, "proposals", agent.id],
                                });
                              })
                            }
                          />
                        </Card>
                      )}
                      <ActionButton
                        label="Skill history"
                        variant="secondary"
                        disabled={busy}
                        onPress={() => setView("history")}
                      />
                      {confirmDetach ? (
                        <Card>
                          <Notice
                            message={`Detach ${pin.name} from ${titleOf(agent)}? Other attached skills stay in place.`}
                          />
                          <ActionButton
                            label="Confirm detach skill"
                            variant="danger"
                            loading={busy}
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
                          label="Detach skill"
                          variant="danger"
                          disabled={busy || !current.data}
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
                    Choose a literal version of {detail.name}. Attaching
                    external guidance changes how {titleOf(agent)} works.
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
                      No published versions are visible through your Ornn
                      access.
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
                        <Label mono>{item.version}</Label>
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
                      !current.data ||
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
                    <Notice message="Enable this skill from its details before updating its exact version." />
                  )}
                  {!pin && pins.length >= 16 && (
                    <Notice message="This agent already has 16 skills. Detach one before attaching another." />
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
                        {SKILL_HISTORY_LABELS[item.event_type] ??
                          "Skill activity"}
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
              {view === "draft" && (
                <>
                  {proposals.isFetching && (
                    <ActivityIndicator
                      accessibilityLabel="Loading skill draft"
                      color={colors.primary}
                    />
                  )}
                  {proposals.isError && (
                    <>
                      <Notice tone="error" message={failure(proposals.error)} />
                      <ActionButton
                        label="Retry skill draft"
                        variant="secondary"
                        onPress={() => void proposals.refetch()}
                      />
                    </>
                  )}
                  {proposal?.draft ? (
                    <SkillDraftEditor
                      key={`${proposal.id}:${proposal.revision}`}
                      agentId={agent.id}
                      proposal={proposal}
                      disabled={
                        !focused ||
                        current.isError ||
                        proposals.isError ||
                        proposals.isFetching ||
                        agent.can_maintain === false ||
                        agent.status === "destroyed"
                      }
                      onBusy={editorBusy}
                      onSaved={() =>
                        client.invalidateQueries({
                          queryKey: [...ROOT, "proposals", agent.id],
                        })
                      }
                    />
                  ) : (
                    !proposals.isFetching &&
                    !proposals.isError && (
                      <Notice message="This draft is no longer available." />
                    )
                  )}
                  {proposal?.card_conversation_id && onOpenChat && (
                    <ActionButton
                      label="Open the confirmation card"
                      variant="secondary"
                      loading={busy}
                      onPress={() =>
                        void run(async () => {
                          const conversationId = proposal.card_conversation_id!;
                          const cardHistory =
                            await assistantApi.history(conversationId);
                          const cardAgentId =
                            cardHistory.conversation.agent?.id;
                          if (!cardAgentId)
                            throw new Error(
                              "The confirmation's agent could not be resolved. Reload the draft and try again.",
                            );
                          if (authEpoch.current !== getAuthEpoch())
                            throw new Error(
                              "Your account session changed. Reopen Plugins to continue.",
                            );
                          setDetail(undefined);
                          onOpenChat({
                            agentId: cardAgentId,
                            conversationId,
                            acknowledgementId:
                              proposal.acknowledgement_id ?? undefined,
                          });
                        })
                      }
                    />
                  )}
                </>
              )}
            </ScrollView>
          </SafeAreaView>
        </SafeAreaProvider>
      </Modal>
    </View>
  );
}

export function SkillDraftEditor({
  agentId,
  proposal,
  disabled = false,
  onBusy,
  onSaved,
}: {
  agentId: string;
  proposal: SkillProposal;
  disabled?: boolean;
  onBusy: (busy: boolean) => void;
  onSaved: () => Promise<unknown>;
}) {
  const [draft, setDraft] = useState<SkillDraft>(() => ({
    ...proposal.draft!,
    files: proposal.draft!.files.map((file) => ({ ...file })),
  }));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [diagnostics, setDiagnostics] = useState<SkillDraftDiagnostic[]>([]);
  const [validBytes, setValidBytes] = useState<number>();
  const [saved, setSaved] = useState(false);
  const lock = useRef(false);
  const authEpoch = useRef(getAuthEpoch());
  const editable =
    proposal.status === "pending" || proposal.status === "publication_failed";
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [online, setOnline] = useState(true);
  const lifecycle = useRef({ mounted: true, revision: 0, active: false });
  const active = editable && !disabled && focused && foreground && online;
  if (lifecycle.current.active !== active) {
    lifecycle.current.active = active;
    lifecycle.current.revision++;
  }
  useEffect(() => {
    lifecycle.current.mounted = true;
    const subscription = AppState.addEventListener("change", (state) => {
      if (state !== "active") lifecycle.current.revision++;
      setForeground(state === "active");
    });
    const unsubscribe = NetInfo.addEventListener((state) => {
      const connected =
        state.isConnected !== false && state.isInternetReachable !== false;
      if (!connected) lifecycle.current.revision++;
      setOnline(connected);
    });
    return () => {
      lifecycle.current.mounted = false;
      lifecycle.current.revision++;
      subscription.remove();
      unsubscribe();
    };
  }, []);
  function update(next: SkillDraft) {
    setDraft(next);
    setValidBytes(undefined);
    setSaved(false);
    setDiagnostics([]);
  }
  async function run(save: boolean) {
    if (lock.current) return;
    const startedRevision = lifecycle.current.revision;
    lock.current = true;
    setBusy(true);
    onBusy(true);
    setError(undefined);
    setSaved(false);
    setDiagnostics([]);
    setValidBytes(undefined);
    try {
      if (authEpoch.current !== getAuthEpoch())
        throw new Error(
          "Your account session changed. Reopen Plugins to continue.",
        );
      if (!draft.name.trim() || !draft.skill_md.trim())
        throw new Error("Enter a skill name and SKILL.md contents.");
      if (draft.files.some((file) => !file.path.trim()))
        throw new Error("Enter a path for each extra file or remove the file.");
      const result = await submitSkillDraft({
        canContinue: () =>
          lifecycle.current.mounted &&
          lifecycle.current.active &&
          lifecycle.current.revision === startedRevision &&
          AppState.currentState === "active" &&
          authEpoch.current === getAuthEpoch(),
        validate: () => api.validateDraft(agentId, proposal.id, draft),
        save: save
          ? () => api.saveDraft(agentId, proposal.id, proposal.revision, draft)
          : undefined,
      });
      setValidBytes(result.bytes);
      if (save) {
        setSaved(true);
        await onSaved();
      }
    } catch (cause) {
      setError(failure(cause));
      setDiagnostics(skillDraftDiagnostics(cause));
    } finally {
      lock.current = false;
      setBusy(false);
      onBusy(false);
    }
  }
  return (
    <View style={styles.stack}>
      <Notice message="Saving changes only the draft. Publish it from its confirmation card in the conversation to update the attached skill." />
      {!editable && (
        <Notice
          message={`This draft is ${proposal.status} and cannot be edited.`}
        />
      )}
      {error && <Notice tone="error" message={error} />}
      {diagnostics.map((item, index) => (
        <Notice
          key={`${item.field}:${item.rule}:${index}`}
          tone="error"
          message={skillDraftDiagnosticText(item)}
        />
      ))}
      <Label muted>
        Kind:{" "}
        {draft.kind === "improve"
          ? "Improve attached skill"
          : draft.kind === "new"
            ? "New skill"
            : "No skill proposed"}
      </Label>
      {draft.base_skill && (
        <Label muted>
          Base: {draft.base_skill.name} · {draft.base_skill.version}
        </Label>
      )}
      {(
        [
          ["name", "Skill name"],
          ["description", "Skill description"],
          ["skill_md", "Skill draft"],
          ["rationale", "Rationale"],
          ["safety_notes", "Safety notes"],
        ] as const
      ).map(([name, label]) => (
        <Field
          key={name}
          label={label}
          value={draft[name]}
          multiline={name !== "name"}
          editable={
            active && !busy && !(name === "name" && draft.kind === "improve")
          }
          onChangeText={(value) => update({ ...draft, [name]: value })}
        />
      ))}
      {draft.files.map((file, index) => (
        <Card key={index}>
          <Field
            label={`File ${index + 1} path`}
            value={file.path}
            editable={active && !busy}
            onChangeText={(value) =>
              update({
                ...draft,
                files: draft.files.map((item, at) =>
                  at === index ? { ...item, path: value } : item,
                ),
              })
            }
          />
          <Field
            label={`File ${index + 1} content`}
            value={file.content}
            multiline
            editable={active && !busy}
            onChangeText={(value) =>
              update({
                ...draft,
                files: draft.files.map((item, at) =>
                  at === index ? { ...item, content: value } : item,
                ),
              })
            }
          />
          {editable && (
            <ActionButton
              label={`Remove file ${index + 1}`}
              variant="secondary"
              disabled={busy || !active}
              onPress={() =>
                update({
                  ...draft,
                  files: draft.files.filter((_, at) => at !== index),
                })
              }
            />
          )}
        </Card>
      ))}
      {editable && (
        <>
          <ActionButton
            label="Add file"
            variant="secondary"
            disabled={busy || !active}
            onPress={() =>
              update({
                ...draft,
                files: [...draft.files, { path: "", content: "" }],
              })
            }
          />
          <ActionButton
            label="Validate draft"
            disabled={!active}
            variant="secondary"
            loading={busy}
            onPress={() => void run(false)}
          />
          <ActionButton
            label="Save draft"
            disabled={!active}
            loading={busy}
            onPress={() => void run(true)}
          />
        </>
      )}
      {validBytes !== undefined && (
        <Notice
          tone="success"
          message={
            saved
              ? "Draft saved. The attached version stays in use until publication."
              : `The draft passes validation (${validBytes.toLocaleString()} bytes). Save it to keep changes.`
          }
        />
      )}
    </View>
  );
}

function SectionHeading({
  children,
  right,
}: {
  children: ReactNode;
  right?: string;
}) {
  const { colors } = useTheme();
  return (
    <View style={styles.row}>
      <Text
        style={[styles.section, styles.flex, { color: colors.textSecondary }]}
      >
        {children}
      </Text>
      {right && (
        <Label muted mono style={styles.caption}>
          {right}
        </Label>
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
  agentIcon: {
    width: 34,
    height: 34,
    borderRadius: 9,
    alignItems: "center",
    justifyContent: "center",
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
  modal: { flex: 1 },
  modalHeader: {
    minHeight: 56,
    borderBottomWidth: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    paddingHorizontal: 8,
  },
  iconButton: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  modalTitle: {
    fontFamily: fonts.bodySemi,
    fontSize: 17,
    lineHeight: 24,
    flex: 1,
    textAlign: "center",
  },
  modalContent: { padding: 20, gap: 16, paddingBottom: 40 },
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
