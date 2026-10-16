import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronLeft, X } from "lucide-react-native";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { pluginSkillsApi as api } from "../../lib/api/pluginSkillsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { SkillDraftEditor } from "./PluginSkills";
import { skillModalBackAction } from "./skillModalNavigation";
import {
  AUTHORED_SKILL_ACTIONS,
  authoredReviewMatches,
  authoredSkillActionRequest,
  authoredSkillStateLabel,
  interruptedPublicationText,
  publicationFailureText,
  supportedAuthoredSkillActions,
  type AuthoredSkillAction,
  type AuthoredSkillReference,
} from "./authoredSkillReviewState";

/** A real owner confirmation: server actions, complete files and exact card binding. */
export function NativeAuthoredSkillReview({
  acknowledgement,
  disabled = false,
  online = true,
  initiallyOpen = false,
  onDecision,
  onRefresh,
}: {
  acknowledgement: {
    id: string;
    authored_skill?: AuthoredSkillReference | null;
  };
  disabled?: boolean;
  online?: boolean;
  initiallyOpen?: boolean;
  onDecision: (choice: "allow" | "deny") => Promise<unknown>;
  onRefresh?: () => Promise<unknown> | void;
}) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const client = useQueryClient();
  const reference = acknowledgement.authored_skill;
  const [open, setOpen] = useState(initiallyOpen);
  const [editing, setEditing] = useState(false);
  const [editorBusy, setEditorBusy] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const lock = useRef(false);
  const editorLock = useRef(false);
  const authEpoch = useRef(getAuthEpoch());
  useEffect(() => {
    if (initiallyOpen) setOpen(true);
  }, [initiallyOpen, acknowledgement.id]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);
  const preview = useQuery({
    queryKey: [
      "workspace",
      "plugins",
      "authored-review",
      reference?.agent_id,
      reference?.proposal_id,
      acknowledgement.id,
    ],
    queryFn: ({ signal }) =>
      api.authoredReview(
        reference!.agent_id,
        reference!.proposal_id,
        acknowledgement.id,
        signal,
      ),
    enabled: Boolean(reference) && focused && foreground && open && online,
    retry: false,
    refetchInterval: (query) =>
      query.state.data?.status === "publishing" && query.state.data.lease_live
        ? Math.min(3000 * 2 ** Math.min(query.state.dataUpdateCount, 3), 24000)
        : false,
  });
  const proposals = useQuery({
    queryKey: [
      "workspace",
      "plugins",
      "skills",
      "proposals",
      reference?.agent_id,
    ],
    queryFn: ({ signal }) => api.proposals(reference!.agent_id, signal),
    enabled:
      Boolean(reference) && focused && foreground && open && editing && online,
    retry: false,
  });
  const data = preview.data;
  const matched = Boolean(
    reference && data && authoredReviewMatches(reference, data),
  );
  const proposal = proposals.data?.proposals.find(
    (item) => item.id === reference?.proposal_id,
  );
  const blocked =
    disabled ||
    busy ||
    editorBusy ||
    !online ||
    !focused ||
    !foreground ||
    preview.isError;
  const actions = data ? supportedAuthoredSkillActions(data) : [];
  const active = data?.state === "active";
  const editable =
    data?.status === "pending" || data?.status === "publication_failed";
  async function refresh() {
    await Promise.all([
      preview.refetch(),
      client.invalidateQueries({
        queryKey: ["workspace", "plugins", "skills"],
      }),
      Promise.resolve(onRefresh?.()),
    ]);
  }
  async function act(action: AuthoredSkillAction) {
    if (
      lock.current ||
      editorLock.current ||
      blocked ||
      editing ||
      !reference ||
      !data ||
      preview.isFetching
    )
      return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      if (authEpoch.current !== getAuthEpoch())
        throw new Error(
          "Your account session changed. Reopen this confirmation.",
        );
      const request = authoredSkillActionRequest(
        reference,
        data,
        acknowledgement.id,
        action,
      );
      if (request.kind === "decision") {
        await onDecision(request.choice);
      } else {
        const result = await api.authoredAction(
          reference.agent_id,
          reference.proposal_id,
          request.action,
          request.body,
        );
        if (request.action === "approve")
          setNotice("A new confirmation was posted in this conversation.");
        if (request.action === "reprepare")
          setNotice(
            result?.card_pending
              ? "The package was rebuilt, but its review card was not posted. Use Show updated draft to post it."
              : "The package was rebuilt from the source skill. Review the new confirmation posted in this conversation.",
          );
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Publication did not finish. Reload the current state before trying again.",
      );
    } finally {
      try {
        await refresh();
      } catch {
        setError(
          "The current publication state could not be refreshed. Reload before trying again.",
        );
      }
      lock.current = false;
      setBusy(false);
    }
  }
  function close() {
    if (!lock.current && !busy && !editorLock.current && !editorBusy) {
      setOpen(false);
      setEditing(false);
    }
  }
  function back() {
    const action = skillModalBackAction(
      editing,
      lock.current || busy || editorLock.current || editorBusy,
    );
    if (action === "parent") setEditing(false);
    if (action === "close") close();
  }
  function onEditorBusy(value: boolean) {
    editorLock.current = value;
    setEditorBusy(value);
  }
  if (!reference) return null;
  return (
    <View style={styles.stack}>
      {data && data.state !== "active" && (
        <Notice
          tone={data.state === "pinned" ? "success" : "info"}
          message={authoredSkillStateLabel(data.state)}
        />
      )}
      <ActionButton
        label={
          data?.state === "pinned"
            ? "View published skill"
            : "Review skill draft"
        }
        variant="secondary"
        onPress={() => setOpen(true)}
      />
      <Modal
        visible={open}
        presentationStyle="pageSheet"
        animationType="slide"
        onRequestClose={back}
      >
        <SafeAreaProvider>
          <SafeAreaView style={[styles.modal, { backgroundColor: colors.bg }]}>
            <View style={[styles.header, { borderColor: colors.border }]}>
              {editing ? (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Back to skill confirmation"
                  disabled={busy || editorBusy}
                  style={styles.iconButton}
                  onPress={back}
                >
                  <ChevronLeft color={colors.textPrimary} size={22} />
                </Pressable>
              ) : (
                <View style={styles.iconButton} accessible={false} />
              )}
              <Text
                accessibilityRole="header"
                numberOfLines={1}
                style={[styles.heading, { color: colors.textPrimary }]}
              >
                {editing ? "Edit skill draft" : "Review skill draft"}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Close skill confirmation"
                disabled={busy || editorBusy}
                style={styles.iconButton}
                onPress={close}
              >
                <X color={colors.textSecondary} size={21} />
              </Pressable>
            </View>
            <ScrollView
              automaticallyAdjustKeyboardInsets
              keyboardShouldPersistTaps="handled"
              contentContainerStyle={styles.content}
            >
              {!online && (
                <Notice message="Connect to the internet to load and confirm the current draft." />
              )}
              {preview.isFetching && (
                <ActivityIndicator
                  accessibilityLabel="Loading complete skill draft"
                  color={colors.primary}
                />
              )}
              {preview.isError && (
                <Notice
                  tone="error"
                  message="Draft unavailable or changed. Reload and review it again."
                />
              )}
              {error && <Notice tone="error" message={error} />}
              {notice && <Notice tone="success" message={notice} />}
              {busy && (
                <ActivityIndicator
                  accessibilityLabel="Updating skill publication state"
                  color={colors.primary}
                />
              )}
              {data && (
                <>
                  <Card>
                    <Label style={styles.title}>
                      {data.name} · {data.version}
                    </Label>
                    <Label muted>Attach to {data.agent_name}</Label>
                    <Label muted style={styles.caption}>
                      Draft revision {data.revision} · Skills revision{" "}
                      {data.skills_revision}
                    </Label>
                    {data.state !== "active" && (
                      <Notice
                        tone={data.state === "pinned" ? "success" : "info"}
                        message={authoredSkillStateLabel(data.state)}
                      />
                    )}
                  </Card>
                  {!matched && (
                    <Notice message="This confirmation does not match the current reviewed draft, or it predates supported review metadata. Reload the confirmation from the conversation." />
                  )}
                  {active && (
                    <>
                      <Label muted>
                        Allow publishes these files privately to Ornn under your
                        identity and attaches this exact version to the agent.
                        Skill instructions never grant permissions.
                      </Label>
                      {data.base_scripts_not_copied && (
                        <Notice message="Scripts and executable files from the source skill are not copied into this draft." />
                      )}
                      {!editing &&
                        data.files.map((file, index) => (
                          <Card key={`${file.path}:${index}`}>
                            <Label>{file.path}</Label>
                            <ScrollView
                              style={styles.fileScroll}
                              nestedScrollEnabled
                            >
                              <Label selectable mono style={styles.fileContent}>
                                {file.content}
                              </Label>
                            </ScrollView>
                          </Card>
                        ))}
                      {editable && matched && (
                        <ActionButton
                          label={
                            editing
                              ? "Close the editor"
                              : "Edit before publishing"
                          }
                          variant="secondary"
                          disabled={blocked || preview.isFetching}
                          onPress={() => setEditing((value) => !value)}
                        />
                      )}
                      {editing && (
                        <>
                          {proposals.isFetching && (
                            <ActivityIndicator
                              accessibilityLabel="Loading editable skill draft"
                              color={colors.primary}
                            />
                          )}
                          {proposals.isError && (
                            <Notice
                              tone="error"
                              message="The editable draft could not be loaded. Reload and try again."
                            />
                          )}
                          {proposal?.draft ? (
                            <SkillDraftEditor
                              key={`${proposal.id}:${proposal.revision}`}
                              agentId={reference.agent_id}
                              proposal={proposal}
                              disabled={
                                !focused ||
                                !foreground ||
                                !online ||
                                preview.isError ||
                                proposals.isError ||
                                preview.isFetching ||
                                proposals.isFetching ||
                                !matched ||
                                !editable
                              }
                              onBusy={onEditorBusy}
                              onSaved={async () => {
                                setEditing(false);
                                setNotice(
                                  "Draft saved. A new review card for this version was posted in the conversation. Confirm it there.",
                                );
                                await refresh();
                              }}
                            />
                          ) : (
                            !proposals.isFetching &&
                            !proposals.isError && (
                              <Notice message="This editable draft is no longer available." />
                            )
                          )}
                        </>
                      )}
                    </>
                  )}
                  {data.status === "publishing" && (
                    <Notice
                      message={
                        data.lease_live
                          ? "Publication is in progress."
                          : interruptedPublicationText(actions)
                      }
                    />
                  )}
                  {data.failure_code && (
                    <Notice
                      tone="error"
                      message={publicationFailureText(
                        data.failure_code,
                        data.status,
                      )}
                    />
                  )}
                  {!editing &&
                    AUTHORED_SKILL_ACTIONS.filter(({ action }) =>
                      actions.includes(action),
                    ).map(({ action, label }) => (
                      <ActionButton
                        key={action}
                        label={label}
                        variant={
                          action === "deny" ||
                          action === "dismiss_card" ||
                          action === "discard"
                            ? "secondary"
                            : "primary"
                        }
                        disabled={blocked || !matched || preview.isFetching}
                        onPress={() => void act(action)}
                      />
                    ))}
                </>
              )}
              <ActionButton
                label="Reload confirmation"
                variant="secondary"
                disabled={busy || editorBusy || !online}
                onPress={() => {
                  void refresh().catch(() =>
                    setError(
                      "The confirmation could not be refreshed. Try again.",
                    ),
                  );
                  if (editing) void proposals.refetch();
                }}
              />
            </ScrollView>
          </SafeAreaView>
        </SafeAreaProvider>
      </Modal>
    </View>
  );
}
const styles = StyleSheet.create({
  stack: { gap: 12 },
  modal: { flex: 1 },
  header: {
    minHeight: 56,
    paddingHorizontal: 8,
    borderBottomWidth: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  iconButton: {
    minHeight: 44,
    minWidth: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  heading: {
    flex: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 17,
    lineHeight: 24,
    textAlign: "center",
  },
  content: { padding: 20, paddingBottom: 40, gap: 16 },
  title: { fontFamily: fonts.bodySemi, fontSize: 16, lineHeight: 23 },
  caption: { fontSize: 11, lineHeight: 17 },
  fileScroll: { maxHeight: 320 },
  fileContent: { fontSize: 11, lineHeight: 18 },
});
