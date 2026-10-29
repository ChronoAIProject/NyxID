import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  ScrollView,
  StyleSheet,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { pluginSkillsApi as api } from "../../lib/api/pluginSkillsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { fonts } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type {
  AuthoredSkillPageParams,
  ContentScreenProps,
  WorkspaceNavigationProp,
} from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { useAssistant } from "../assistant/useAssistant";
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

/** The transcript holds only a route launcher; owner review is content in the slab. */
export function NativeAuthoredSkillReview({
  acknowledgement,
  conversationId,
  initiallyOpen = false,
}: {
  acknowledgement: {
    id: string;
    authored_skill?: AuthoredSkillReference | null;
  };
  conversationId: string;
  initiallyOpen?: boolean;
}) {
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const focused = useIsFocused();
  const opened = useRef<string | undefined>(undefined);
  const reference = acknowledgement.authored_skill;
  function open() {
    if (reference)
      navigation.push("AuthoredSkillReview", {
        conversationId,
        acknowledgementId: acknowledgement.id,
        agentId: reference.agent_id,
      });
  }
  useEffect(() => {
    if (!initiallyOpen || !focused || opened.current === acknowledgement.id)
      return;
    opened.current = acknowledgement.id;
    open();
  }, [initiallyOpen, focused, acknowledgement.id]);
  return reference ? (
    <ActionButton
      label="Review skill draft"
      variant="secondary"
      onPress={open}
    />
  ) : null;
}

export function AuthoredSkillPage({
  route,
}: ContentScreenProps<"AuthoredSkillReview">) {
  const params: AuthoredSkillPageParams = route.params;
  const assistant = useAssistant(
    params.conversationId,
    params.agentId,
    undefined,
    params.acknowledgementId,
  );
  const acknowledgement = assistant.history?.acknowledgements.find(
    (row) => row.id === params.acknowledgementId,
  );
  return (
    <WorkspacePage
      title="Review skill draft"
      showHeading={false}
      scroll={false}
      contentStyle={styles.page}
    >
      {assistant.error && <Notice tone="error" message={assistant.error} />}
      {assistant.loading && !acknowledgement && (
        <ActivityIndicator accessibilityLabel="Loading skill confirmation" />
      )}
      {acknowledgement?.authored_skill ? (
        <AuthoredSkillReviewContent
          acknowledgement={acknowledgement}
          disabled={Boolean(assistant.deciding) || assistant.loadingOlder}
          online={assistant.online}
          onDecision={(choice) => assistant.decide(acknowledgement.id, choice)}
          onRefresh={() => assistant.openConversation(params.conversationId)}
        />
      ) : (
        !assistant.loading && (
          <Notice message="This skill confirmation is no longer available. Return to the conversation and reload its latest history." />
        )
      )}
    </WorkspacePage>
  );
}

/** A real owner confirmation: server actions, complete files and exact card binding. */
export function AuthoredSkillReviewContent({
  acknowledgement,
  disabled = false,
  online = true,
  onDecision,
  onRefresh,
}: {
  acknowledgement: {
    id: string;
    authored_skill?: AuthoredSkillReference | null;
  };
  disabled?: boolean;
  online?: boolean;
  onDecision: (choice: "allow" | "deny") => Promise<unknown>;
  onRefresh?: () => Promise<unknown> | void;
}) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const client = useQueryClient();
  const reference = acknowledgement.authored_skill;
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const lock = useRef(false);
  const authEpoch = useRef(getAuthEpoch());
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
    enabled: Boolean(reference) && focused && foreground && online,
    retry: false,
    refetchInterval: (query) =>
      query.state.data?.status === "publishing" && query.state.data.lease_live
        ? Math.min(3000 * 2 ** Math.min(query.state.dataUpdateCount, 3), 24000)
        : false,
  });
  const data = preview.data;
  const matched = Boolean(
    reference && data && authoredReviewMatches(reference, data),
  );
  const blocked =
    disabled || busy || !online || !focused || !foreground || preview.isError;
  const actions = data ? supportedAuthoredSkillActions(data) : [];
  const active = data?.state === "active";
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
    if (lock.current || blocked || !reference || !data || preview.isFetching)
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
  useWorkspaceBack({
    active: false,
    onBack: navigation.goBack,
    defaultRole: "back",
    disabled: busy,
  });
  if (!reference) return null;
  return (
    <View style={styles.fill}>
      {data && data.state !== "active" && (
        <Notice
          tone={data.state === "pinned" ? "success" : "info"}
          message={authoredSkillStateLabel(data.state)}
        />
      )}
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
                  identity and attaches this exact version to the agent. Skill
                  instructions never grant permissions.
                </Label>
                {data.base_scripts_not_copied && (
                  <Notice message="Scripts and executable files from the source skill are not copied into this draft." />
                )}
                {data.files.map((file, index) => (
                  <Card key={`${file.path}:${index}`}>
                    <Label>{file.path}</Label>
                    <ScrollView style={styles.fileScroll} nestedScrollEnabled>
                      <Label selectable mono style={styles.fileContent}>
                        {file.content}
                      </Label>
                    </ScrollView>
                  </Card>
                ))}
                <Label muted>To change this skill, ask NyxBot.</Label>
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
                message={publicationFailureText(data.failure_code, data.status)}
              />
            )}
            {AUTHORED_SKILL_ACTIONS.filter(({ action }) =>
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
          disabled={busy || !online}
          onPress={() => {
            void refresh().catch(() =>
              setError("The confirmation could not be refreshed. Try again."),
            );
          }}
        />
      </ScrollView>
    </View>
  );
}
const styles = StyleSheet.create({
  page: { flex: 1, padding: 0 },
  fill: { flex: 1 },
  content: { padding: 20, paddingBottom: 40, gap: 16 },
  title: { fontFamily: fonts.bodySemi, fontSize: 16, lineHeight: 23 },
  caption: { fontSize: 11, lineHeight: 17 },
  fileScroll: { maxHeight: 320 },
  fileContent: { fontSize: 11, lineHeight: 18 },
});
