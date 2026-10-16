import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useQuery } from "@tanstack/react-query";
import Markdown, {
  MarkdownIt,
  type RenderRules,
} from "react-native-markdown-display";
import * as WebBrowser from "expo-web-browser";
import {
  Check,
  Paperclip,
  Plus,
  Send,
  Settings2,
  MessageSquare,
  ThumbsDown,
  ThumbsUp,
  Users,
  X,
} from "lucide-react-native";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { accountApi } from "../../lib/api/accountApi";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { agentsApi } from "../../lib/api/agentsApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius } from "../../theme/designTokens";
import { AssistantPendingUploads } from "../assistant/AssistantPendingUploads";
import { AssistantAttachmentView } from "../assistant/AssistantAttachmentView";
import { AssistantActivityDisclosure } from "../assistant/AssistantActivityDisclosure";
import { AssistantCopyButton } from "../assistant/AssistantCopyButton";
import { assistantSelectableMarkdown } from "../assistant/assistantSelectableMarkdown";
import { useAssistantUploads } from "../assistant/useAssistantUploads";
import {
  safeAssistantLinkUrl,
  assistantMarkdownOptions,
} from "../assistant/assistantMarkdown";
import { transcriptEndOffset } from "../assistant/assistantTranscriptScroll";
import {
  NO_RECIPIENT_EDITS,
  groupCanDecide,
  groupPendingOwned,
  groupPendingReview,
  resolveRecipients,
  withAdded,
  withDraft,
  withRemoved,
  type RecipientSelection,
} from "./groupState";
import type { Group, GroupMessage, GroupPendingAction } from "./groupContracts";
import { submitGroupDraft } from "./groupComposerDraft";
import {
  groupKeys,
  useGroupAvailability,
  useGroupChat,
} from "./useGroupRuntime";

export type GroupConversationTarget = {
  conversationId: string;
  agentId?: string;
  acknowledgementId?: string;
};
export type GroupChatScreenProps = {
  groupId: string;
  onBack: () => void;
  onSettings: () => void;
  onOpenConversation?: (target: GroupConversationTarget) => void;
  onOpenAgent?: (id: string) => void;
};
const groupMarkdown = MarkdownIt(assistantMarkdownOptions);
groupMarkdown.validateLink = (url: string) =>
  safeAssistantLinkUrl(url) !== null;

export function GroupChatScreen(props: GroupChatScreenProps) {
  const { epoch } = useGroupAvailability();
  return <GroupChat key={`${epoch}:${props.groupId}`} {...props} />;
}

function GroupChat({
  groupId,
  onBack,
  onSettings,
  onOpenConversation,
  onOpenAgent,
}: GroupChatScreenProps) {
  const { colors } = useTheme();
  const chat = useGroupChat(groupId);
  const [draft, setDraft] = useState("");
  const [composerGeneration, setComposerGeneration] = useState(0);
  const currentComposer = useRef(0);
  const revision = useRef(0);
  const [selection, setSelection] =
    useState<RecipientSelection>(NO_RECIPIENT_EDITS);
  const [pickingRecipients, setPickingRecipients] = useState(false);
  const [localError, setLocalError] = useState<string>();
  const [feedbackDraft, setFeedbackDraft] = useState<{
    messageId: string;
    vote?: "up" | "down";
    comment: string;
  }>();
  const group = chat.isError ? undefined : chat.data?.group;
  const names = useQuery({
    queryKey: ["workspace", "agents", "group-labels", chat.epoch],
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: chat.active,
    retry: false,
    staleTime: 60_000,
  });
  const identity = useQuery({
    queryKey: [...groupKeys(chat.epoch), "identity"],
    queryFn: ({ signal }) => accountApi.getProfile(signal),
    enabled: chat.active && !!chat.data?.pending_actions.length,
    retry: false,
    staleTime: 60_000,
  });
  function title(id: string, fallback: string) {
    const agent = names.data?.agents.find((row) => row.id === id);
    return agent?.display_name ?? fallback;
  }
  const liveMembers =
    group?.members.filter((member) => !member.destroyed) ?? [];
  const defaults =
    group?.default_recipient_ids ??
    [
      group?.lead_agent_id ??
        liveMembers.find((member) => member.kind === "nyxbot")?.id ??
        liveMembers[0]?.id,
    ].filter((id): id is string => !!id);
  const recipients = resolveRecipients(
    selection,
    defaults,
    group?.members ?? [],
  );
  const canParticipate =
    !!group &&
    (group.owner?.type !== "org" ||
      ["creator", "admin", "participant"].includes(group.your_role ?? ""));
  const uploads = useAssistantUploads({
    groupId,
    model: "",
    generation: 0,
    enabled:
      canParticipate &&
      !chat.isError &&
      !chat.recoveryRequired &&
      liveMembers.length > 0,
    online: chat.online,
  });
  const list = useRef<FlatList<GroupMessage>>(null);
  const following = useRef(true);
  const metrics = useRef({ height: 0, viewportHeight: 0 });
  const sendLock = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useWorkspaceBack({
    active: chat.sending || uploads.session.sending,
    disabled: chat.sending || uploads.session.sending,
    defaultRole: "back",
    onBack,
    label: "Back to groups",
  });
  useEffect(() => {
    if (!chat.active) {
      setPickingRecipients(false);
      setFeedbackDraft(undefined);
    }
  }, [chat.active]);
  const blocked =
    chat.sending ||
    uploads.session.sending ||
    chat.recoveryRequired ||
    chat.loadingOlder ||
    !chat.active ||
    !canParticipate ||
    chat.isError;
  const canSend =
    !blocked &&
    !uploads.blocked &&
    !uploads.session.sending &&
    !!recipients.ids.length &&
    liveMembers.length > 0 &&
    (!!draft.trim() || uploads.files.length > 0) &&
    [...draft].length <= 32768;

  function edit(value: string) {
    revision.current += 1;
    setDraft(value);
    setSelection((old) => withDraft(old, value, group?.members ?? []));
  }
  function editRecipients(
    update: (old: RecipientSelection) => RecipientSelection,
  ) {
    revision.current += 1;
    setSelection(update);
  }
  async function send() {
    if (!canSend || sendLock.current || !chat.available()) return;
    sendLock.current = true;
    setLocalError(undefined);
    const target = recipients;
    following.current = true;
    try {
      await submitGroupDraft({
        draft,
        selection,
        updateComposer: (text, nextSelection) => {
          revision.current += 1;
          // Discard native marked text and late IME events from the sent draft.
          currentComposer.current += 1;
          setComposerGeneration(currentComposer.current);
          setDraft(text);
          setSelection(nextSelection);
        },
        getRevision: () =>
          mounted.current && chat.epoch === getAuthEpoch()
            ? revision.current
            : Number.NaN,
        send: (text) =>
          uploads.session.submit((files) =>
            chat.post({
              text,
              ...(files?.attachmentIds.length
                ? { attachment_ids: files.attachmentIds }
                : {}),
              ...(target.edited ? { recipient_agent_ids: target.ids } : {}),
            }),
          ),
      });
    } catch (cause) {
      if (mounted.current && chat.available())
        setLocalError(
          cause instanceof Error
            ? cause.message
            : "The message was not delivered. Refresh the group before trying again.",
        );
    } finally {
      sendLock.current = false;
    }
  }
  async function decide(action: GroupPendingAction, choice: "allow" | "deny") {
    if (
      blocked ||
      !chat.available() ||
      !groupCanDecide(action, identity.data?.id, group?.owner?.type === "org")
    )
      return;
    try {
      await chat.perform(async () => {
        await groupsApi.decide(action, choice);
        if (chat.available()) await chat.refetch();
      });
    } catch {
      /* useGroupChat owns the visible action error */
    }
  }
  async function answer(action: GroupPendingAction, yes: boolean) {
    if (
      blocked ||
      !chat.available() ||
      group?.owner?.type === "org" ||
      !groupPendingOwned(action, identity.data?.id, false) ||
      !action.confirm_phrase ||
      Date.parse(action.expires_at) <= Date.now()
    )
      return;
    try {
      await chat.post({
        text: yes
          ? action.confirm_phrase
          : action.confirm_phrase.replace(/^yes /, "no "),
      });
    } catch {
      /* no implicit retry */
    }
  }
  const feedbackEnabled =
    chat.active &&
    !!group &&
    !chat.isError &&
    !chat.sending &&
    !uploads.session.sending;
  async function feedback(
    messageId: string,
    vote?: "up" | "down",
    comment?: string,
  ) {
    const message = chat.data?.messages.find((row) => row.id === messageId);
    if (
      !feedbackEnabled ||
      !chat.available() ||
      message?.role !== "agent" ||
      !message.text.trim()
    )
      return false;
    try {
      return await chat.feedback(
        messageId,
        vote ? { vote, comment } : undefined,
      );
    } catch {
      // The runtime retains the confirmed vote and exposes the action error.
      return false;
    }
  }
  const working =
    group?.members.filter((member) =>
      group.working_agent_ids.includes(member.id),
    ) ?? [];

  return (
    <WorkspacePage
      title={group?.name ?? "Group"}
      showHeading={false}
      scroll={false}
    >
      <View style={styles.header}>
        <View style={styles.fill}>
          <Label numberOfLines={1} style={styles.heading}>
            {group?.name ?? "Group"}
          </Label>
          <Label muted style={styles.small}>
            {group?.owner?.name ?? "Personal"} · Group
          </Label>
        </View>
        <Icon
          label="Group settings"
          onPress={onSettings}
          disabled={!group || chat.sending || uploads.session.sending}
        >
          <Settings2 size={20} color={colors.textSecondary} />
        </Icon>
      </View>
      {!chat.online && (
        <Notice message="You're offline. Reconnect to read new messages and send." />
      )}
      {(chat.recoveryRequired ||
        chat.error ||
        chat.actionError ||
        localError) && (
        <View style={styles.error}>
          <Notice
            tone="error"
            message={
              chat.recoveryRequired
                ? "This message may have been delivered. Refresh the group and check its history before sending again."
                : (localError ??
                  chat.actionError ??
                  (chat.error instanceof Error
                    ? chat.error.message
                    : "Could not load this group."))
            }
          />
          <ActionButton
            label="Refresh group"
            variant="secondary"
            disabled={
              !chat.active ||
              chat.isFetching ||
              chat.sending ||
              uploads.session.sending
            }
            onPress={() => {
              setLocalError(undefined);
              void chat.recoverDelivery();
            }}
          />
        </View>
      )}
      {group && (
        <ScrollView
          horizontal
          style={styles.horizontalStrip}
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={styles.members}
        >
          {group.members.map((member) => (
            <Pressable
              key={member.id}
              accessibilityRole="button"
              accessibilityLabel={`${title(member.id, member.name)}${member.destroyed ? ", destroyed" : ""}, agent details`}
              disabled={!onOpenAgent || chat.sending || uploads.session.sending}
              onPress={() => onOpenAgent?.(member.id)}
              style={[
                styles.member,
                {
                  borderColor: colors.border,
                  backgroundColor: colors.card,
                  opacity: member.destroyed ? 0.45 : 1,
                },
              ]}
            >
              <Users
                size={14}
                color={member.working ? colors.success : colors.primaryOnTint}
              />
              <Label style={styles.small}>
                {title(member.id, member.name)}
              </Label>
            </Pressable>
          ))}
        </ScrollView>
      )}
      <FlatList
        ref={list}
        style={styles.fill}
        data={group ? (chat.data?.messages ?? []) : []}
        keyExtractor={(item) => item.id}
        ItemSeparatorComponent={GroupSeparator}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="on-drag"
        maintainVisibleContentPosition={
          chat.data?.messages.length ? { minIndexForVisible: 0 } : undefined
        }
        contentContainerStyle={styles.transcript}
        onLayout={(event) => {
          metrics.current.viewportHeight = event.nativeEvent.layout.height;
          if (following.current)
            list.current?.scrollToOffset({
              offset: transcriptEndOffset(
                metrics.current.height,
                metrics.current.viewportHeight,
              ),
              animated: false,
            });
        }}
        onContentSizeChange={(_width, height) => {
          metrics.current.height = height;
          if (following.current)
            list.current?.scrollToOffset({
              offset: transcriptEndOffset(
                height,
                metrics.current.viewportHeight,
              ),
              animated: false,
            });
        }}
        onScrollBeginDrag={() => {
          following.current = false;
        }}
        onScroll={(event) => {
          const { contentSize, contentOffset, layoutMeasurement } =
            event.nativeEvent;
          following.current =
            contentSize.height - layoutMeasurement.height - contentOffset.y <
            80;
        }}
        scrollEventThrottle={100}
        ListHeaderComponent={
          chat.data?.before_seq ? (
            <ActionButton
              label="Load earlier messages"
              variant="secondary"
              loading={chat.loadingOlder}
              disabled={blocked}
              onPress={() => {
                following.current = false;
                void chat.loadOlder();
              }}
            />
          ) : null
        }
        ListHeaderComponentStyle={styles.historyHeader}
        ListEmptyComponent={
          chat.isPending ? (
            <ActivityIndicator
              accessibilityLabel="Loading group"
              color={colors.primary}
            />
          ) : group ? (
            <View style={styles.empty}>
              <Users size={32} color={colors.primaryOnTint} />
              <Label style={styles.heading}>Work together</Label>
              <Label muted>
                Say hello. @mention an agent to ask it directly.
              </Label>
            </View>
          ) : null
        }
        renderItem={({ item }) => (
          <GroupMessage
            group={group}
            message={item}
            title={title}
            onOpenAgent={
              chat.sending || uploads.session.sending ? undefined : onOpenAgent
            }
            feedbackEnabled={feedbackEnabled}
            onFeedback={(vote) => void feedback(item.id, vote)}
            onComment={() => {
              if (!feedbackEnabled || !chat.available()) return;
              following.current = false;
              setFeedbackDraft({
                messageId: item.id,
                vote: item.feedback?.vote,
                comment: item.feedback?.comment ?? "",
              });
            }}
            onDisclosure={() => {
              following.current = false;
            }}
          />
        )}
        ListFooterComponent={
          working.length ? (
            <View style={styles.working}>
              <ActivityIndicator size="small" color={colors.primary} />
              <Label muted>
                {working
                  .map((member) => title(member.id, member.name))
                  .join(", ")}{" "}
                working…
              </Label>
            </View>
          ) : null
        }
      />
      {!!group && !!chat.data?.pending_actions.length && (
        <ScrollView
          style={styles.pending}
          contentContainerStyle={{ gap: 8 }}
          keyboardShouldPersistTaps="handled"
        >
          {chat.data.pending_actions.map((action, index) => {
            const member = group?.members.find(
              (row) => row.id === action.agent_id,
            );
            const expired = Date.parse(action.expires_at) <= Date.now();
            const review = groupPendingReview(
              action,
              identity.data?.id,
              group?.owner?.type === "org",
            );
            const canDecide = groupCanDecide(
              action,
              identity.data?.id,
              group?.owner?.type === "org",
            );
            return (
              <Card
                key={
                  action.acknowledgement_id ??
                  action.approval_request_id ??
                  `${action.conversation_id}:${index}`
                }
                style={styles.pendingCard}
              >
                <Label numberOfLines={3}>
                  {member ? title(member.id, member.name) : "An agent"} wants
                  to: {action.summary}
                </Label>
                {expired ? (
                  <Label muted>
                    This request expired. Refresh the group for its latest
                    state.
                  </Label>
                ) : action.authored_skill ? (
                  review && onOpenConversation ? (
                    <ActionButton
                      label="Review draft"
                      variant="secondary"
                      disabled={blocked}
                      onPress={() => {
                        if (blocked || !chat.available()) return;
                        const currentReview = groupPendingReview(
                          action,
                          identity.data?.id,
                          group?.owner?.type === "org",
                        );
                        if (!currentReview) return;
                        onOpenConversation({
                          conversationId: currentReview.conversationId,
                          agentId: action.agent_id ?? undefined,
                          acknowledgementId: currentReview.acknowledgementId,
                        });
                      }}
                    />
                  ) : (
                    <Label muted>
                      Awaiting{" "}
                      {action.triggering_person?.display_name ?? "owner"} review
                    </Label>
                  )
                ) : action.triggering_person ? (
                  <>
                    <Label muted>
                      Awaiting {action.triggering_person.display_name}
                    </Label>
                    {canDecide && (
                      <View style={styles.actions}>
                        <View style={styles.fill}>
                          <ActionButton
                            label="Deny"
                            variant="secondary"
                            disabled={blocked}
                            onPress={() => void decide(action, "deny")}
                          />
                        </View>
                        <View style={styles.fill}>
                          <ActionButton
                            label="Allow"
                            disabled={blocked}
                            onPress={() => void decide(action, "allow")}
                          />
                        </View>
                      </View>
                    )}
                  </>
                ) : action.confirm_phrase && group?.owner?.type !== "org" ? (
                  <View style={styles.actions}>
                    <View style={styles.fill}>
                      <ActionButton
                        label="Cancel action"
                        variant="secondary"
                        disabled={blocked}
                        onPress={() => void answer(action, false)}
                      />
                    </View>
                    <View style={styles.fill}>
                      <ActionButton
                        label="Confirm action"
                        disabled={blocked}
                        onPress={() => void answer(action, true)}
                      />
                    </View>
                  </View>
                ) : (
                  <Label muted>
                    Awaiting the owner in the agent's conversation.
                  </Label>
                )}
              </Card>
            );
          })}
        </ScrollView>
      )}
      {group && (
        <View style={[styles.composerArea, { borderColor: colors.border }]}>
          <ScrollView
            horizontal
            style={styles.horizontalStrip}
            keyboardShouldPersistTaps="handled"
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={styles.recipients}
          >
            <Label muted style={styles.small}>
              To
            </Label>
            {recipients.ids.map((id) => {
              const member = liveMembers.find((row) => row.id === id);
              return member ? (
                <Pressable
                  key={id}
                  accessibilityRole="button"
                  accessibilityLabel={`Remove recipient ${title(id, member.name)}`}
                  disabled={blocked}
                  onPress={() => editRecipients((old) => withRemoved(old, id))}
                  style={[
                    styles.recipient,
                    {
                      backgroundColor: colors.cardSoft,
                      borderColor: colors.border,
                    },
                  ]}
                >
                  <Label style={styles.small}>{title(id, member.name)}</Label>
                  <X size={12} color={colors.textSecondary} />
                </Pressable>
              ) : null;
            })}
            {liveMembers.some(
              (member) => !recipients.ids.includes(member.id),
            ) && (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Add recipient"
                disabled={blocked}
                onPress={() => setPickingRecipients(true)}
                style={styles.addRecipient}
              >
                <Plus size={14} color={colors.primaryOnTint} />
                <Label style={[styles.small, { color: colors.primaryOnTint }]}>
                  Add
                </Label>
              </Pressable>
            )}
          </ScrollView>
          {!recipients.ids.length && (
            <Label style={[styles.small, { color: colors.danger }]}>
              Choose at least one recipient.
            </Label>
          )}
          {!liveMembers.length && (
            <Label muted>
              All agents have been removed or destroyed. This group is
              read-only.
            </Label>
          )}
          {!canParticipate && (
            <Label muted>
              This group is read-only for your current organization role.
            </Label>
          )}
          <AssistantPendingUploads
            files={uploads.files}
            remove={uploads.remove}
            disabled={chat.sending || uploads.session.sending}
          />
          {(uploads.error || uploads.notice) && (
            <Label muted>{uploads.error ?? uploads.notice}</Label>
          )}
          <View
            style={[
              styles.composer,
              { backgroundColor: colors.card, borderColor: colors.border },
            ]}
          >
            <Icon
              label="Attach a file to group"
              disabled={
                blocked ||
                !liveMembers.length ||
                uploads.picking ||
                uploads.session.sending
              }
              onPress={() => void uploads.pick()}
            >
              <Paperclip size={20} color={colors.textMuted} />
            </Icon>
            <TextInput
              key={composerGeneration}
              accessibilityLabel="Message the group"
              placeholder="Message · @mention an agent"
              placeholderTextColor={colors.textMuted}
              value={draft}
              onChangeText={(value) => {
                if (currentComposer.current === composerGeneration) edit(value);
              }}
              editable={!blocked && !!liveMembers.length}
              multiline
              maxLength={32768}
              style={[styles.input, { color: colors.textPrimary }]}
            />
            <Icon
              label="Send group message"
              disabled={!canSend}
              onPress={() => void send()}
            >
              {chat.sending ? (
                <ActivityIndicator color={colors.primary} />
              ) : (
                <Send
                  size={20}
                  color={canSend ? colors.primaryOnTint : colors.textMuted}
                />
              )}
            </Icon>
          </View>
        </View>
      )}
      <Modal
        visible={!!feedbackDraft}
        transparent
        animationType="slide"
        onRequestClose={() => {
          if (!chat.sending) setFeedbackDraft(undefined);
        }}
      >
        <View style={styles.modalBackdrop}>
          <View
            style={[
              styles.picker,
              { backgroundColor: colors.bg, borderColor: colors.border },
            ]}
          >
            <View style={styles.header}>
              <Label style={[styles.heading, styles.fill]}>
                Message feedback
              </Label>
              <Icon
                label="Close feedback"
                disabled={chat.sending}
                onPress={() => setFeedbackDraft(undefined)}
              >
                <X size={22} color={colors.textPrimary} />
              </Icon>
            </View>
            <View style={styles.actions}>
              {(["up", "down"] as const).map((vote) => (
                <View key={vote} style={styles.fill}>
                  <ActionButton
                    label={vote === "up" ? "Helpful" : "Not helpful"}
                    variant={
                      feedbackDraft?.vote === vote ? "primary" : "secondary"
                    }
                    disabled={chat.sending}
                    onPress={() =>
                      setFeedbackDraft((old) => old && { ...old, vote })
                    }
                  />
                </View>
              ))}
            </View>
            <TextInput
              accessibilityLabel="Optional feedback comment"
              placeholder="Optional comment"
              placeholderTextColor={colors.textMuted}
              value={feedbackDraft?.comment ?? ""}
              multiline
              editable={!chat.sending}
              onChangeText={(comment) =>
                setFeedbackDraft((old) => old && { ...old, comment })
              }
              style={[
                styles.feedbackInput,
                {
                  color: colors.textPrimary,
                  borderColor: colors.border,
                  backgroundColor: colors.card,
                },
              ]}
            />
            <Label muted style={styles.small}>
              {[...(feedbackDraft?.comment ?? "")].length} / 1000
            </Label>
            {chat.actionError && (
              <Notice tone="error" message={chat.actionError} />
            )}
            <ActionButton
              label="Save feedback"
              loading={chat.sending}
              disabled={
                !feedbackEnabled ||
                !feedbackDraft?.vote ||
                [...(feedbackDraft?.comment ?? "")].length > 1000
              }
              onPress={() => {
                const current = feedbackDraft;
                if (!current?.vote) return;
                void feedback(
                  current.messageId,
                  current.vote,
                  current.comment,
                ).then((saved) => {
                  if (saved && mounted.current && chat.available())
                    setFeedbackDraft(undefined);
                });
              }}
            />
          </View>
        </View>
      </Modal>
      <Modal
        visible={pickingRecipients}
        transparent
        animationType="slide"
        onRequestClose={() => setPickingRecipients(false)}
      >
        <View style={styles.modalBackdrop}>
          <View
            style={[
              styles.picker,
              { backgroundColor: colors.bg, borderColor: colors.border },
            ]}
          >
            <View style={styles.header}>
              <Label style={styles.heading}>Add recipient</Label>
              <Icon
                label="Close recipients"
                onPress={() => setPickingRecipients(false)}
              >
                <X size={22} color={colors.textPrimary} />
              </Icon>
            </View>
            <ScrollView>
              {liveMembers
                .filter((member) => !recipients.ids.includes(member.id))
                .map((member) => (
                  <Pressable
                    key={member.id}
                    accessibilityRole="button"
                    accessibilityLabel={`Add recipient ${title(member.id, member.name)}`}
                    style={styles.pickerRow}
                    onPress={() => {
                      if (blocked || !chat.available()) return;
                      editRecipients((old) => withAdded(old, member.id));
                      setPickingRecipients(false);
                    }}
                  >
                    <Label>{title(member.id, member.name)}</Label>
                    <Check size={18} color={colors.primaryOnTint} />
                  </Pressable>
                ))}
            </ScrollView>
          </View>
        </View>
      </Modal>
    </WorkspacePage>
  );
}

function GroupMessage({
  group,
  message,
  title,
  onOpenAgent,
  onDisclosure,
  feedbackEnabled,
  onFeedback,
  onComment,
}: {
  group?: Group;
  message: GroupMessage;
  title: (id: string, fallback: string) => string;
  onOpenAgent?: (id: string) => void;
  onDisclosure: () => void;
  feedbackEnabled: boolean;
  onFeedback: (vote?: "up" | "down") => void;
  onComment: () => void;
}) {
  const { colors } = useTheme();
  const user = message.role === "user";
  if (message.role === "notice")
    return (
      <Label muted selectable style={styles.notice}>
        {message.text}
      </Label>
    );
  const link = (url: string) => {
    const safe = safeAssistantLinkUrl(url);
    if (safe) void WebBrowser.openBrowserAsync(safe).catch(() => undefined);
    return false;
  };
  const rules: RenderRules = {
    ...assistantSelectableMarkdown,
    image: (node) => (
      <Text
        key={node.key}
        style={{ color: colors.primaryOnTint }}
        onPress={() => {
          if (typeof node.attributes.src === "string")
            link(node.attributes.src);
        }}
      >
        {typeof node.attributes.alt === "string" && node.attributes.alt.trim()
          ? node.attributes.alt
          : "Image"}
      </Text>
    ),
  };
  return (
    <View
      style={[
        styles.message,
        user && {
          backgroundColor: colors.card,
          borderColor: colors.border,
          borderWidth: 1,
          borderRadius: radius.lg,
          alignSelf: "flex-end",
        },
      ]}
    >
      <Pressable
        accessibilityRole={message.agent && onOpenAgent ? "button" : "text"}
        disabled={!message.agent || !onOpenAgent}
        onPress={() => {
          if (message.agent) onOpenAgent?.(message.agent.id);
        }}
      >
        <Label muted style={styles.small}>
          {user
            ? (message.author?.display_name ?? "You")
            : message.agent
              ? title(message.agent.id, message.agent.name)
              : "Agent"}{" "}
          · {clock(message.created_at)}
        </Label>
      </Pressable>
      {user && !!message.recipient_agent_ids?.length && (
        <Label muted style={styles.small}>
          to{" "}
          {message.recipient_agent_ids
            .map((id) => {
              const member = group?.members.find((row) => row.id === id);
              return member ? title(id, member.name) : "a removed agent";
            })
            .join(", ")}
        </Label>
      )}
      {user ? (
        <Label selectable>{message.text}</Label>
      ) : (
        <Markdown
          markdownit={groupMarkdown}
          mergeStyle
          style={{
            body: {
              color: colors.textPrimary,
              fontSize: 14,
              lineHeight: 23,
              fontFamily: fonts.bodyRegular,
            },
            paragraph: { marginTop: 0, marginBottom: 8 },
            link: { color: colors.primaryOnTint },
            fence: {
              backgroundColor: colors.cardSoft,
              color: colors.textPrimary,
              borderColor: colors.border,
              fontFamily: fonts.mono,
            },
            code_inline: {
              backgroundColor: colors.cardSoft,
              color: colors.textPrimary,
            },
            heading1: { fontSize: 22, color: colors.textPrimary },
            heading2: { fontSize: 18, color: colors.textPrimary },
          }}
          rules={rules}
          onLinkPress={link}
        >
          {message.text}
        </Markdown>
      )}
      {!!message.activities?.length && (
        <AssistantActivityDisclosure
          activities={message.activities}
          onToggle={onDisclosure}
        />
      )}
      {message.attachments?.map((attachment) => (
        <AssistantAttachmentView
          key={attachment.id}
          attachment={attachment}
          groupId={group?.id}
        />
      ))}
      {message.role === "agent" && !!message.text.trim() && (
        <View style={styles.feedbackActions}>
          <AssistantCopyButton text={message.text} />
          {(["up", "down"] as const).map((vote) => (
            <Pressable
              key={vote}
              accessibilityRole="button"
              accessibilityLabel={vote === "up" ? "Helpful" : "Not helpful"}
              accessibilityState={{
                selected: message.feedback?.vote === vote,
                disabled: !feedbackEnabled,
              }}
              disabled={!feedbackEnabled}
              style={[styles.icon, { opacity: feedbackEnabled ? 1 : 0.4 }]}
              onPress={() =>
                onFeedback(message.feedback?.vote === vote ? undefined : vote)
              }
            >
              {vote === "up" ? (
                <ThumbsUp
                  size={17}
                  color={
                    message.feedback?.vote === vote
                      ? colors.primaryOnTint
                      : colors.textMuted
                  }
                />
              ) : (
                <ThumbsDown
                  size={17}
                  color={
                    message.feedback?.vote === vote
                      ? colors.primaryOnTint
                      : colors.textMuted
                  }
                />
              )}
            </Pressable>
          ))}
          <Icon
            label="Add feedback comment"
            disabled={!feedbackEnabled}
            onPress={onComment}
          >
            <MessageSquare size={17} color={colors.textMuted} />
          </Icon>
        </View>
      )}
      {user && <AssistantCopyButton text={message.text} />}
    </View>
  );
}
function clock(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
      });
}
function GroupSeparator() {
  return <View style={{ height: 16 }} />;
}
function Icon({
  label,
  disabled,
  onPress,
  children,
}: {
  label: string;
  disabled?: boolean;
  onPress: () => void;
  children: React.ReactNode;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={[styles.icon, { opacity: disabled ? 0.4 : 1 }]}
    >
      {children}
    </Pressable>
  );
}
const styles = StyleSheet.create({
  horizontalStrip: { flexGrow: 0 },
  feedbackActions: { flexDirection: "row", alignItems: "center" },
  feedbackInput: {
    minHeight: 96,
    maxHeight: 180,
    borderWidth: 1,
    borderRadius: 12,
    padding: 12,
    fontFamily: fonts.bodyRegular,
    textAlignVertical: "top",
  },
  fill: { flex: 1, minWidth: 0 },
  header: { flexDirection: "row", alignItems: "center", gap: 8 },
  heading: { fontFamily: fonts.displaySemi, fontSize: 18 },
  small: { fontSize: 11, lineHeight: 17 },
  icon: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  members: { alignItems: "center", gap: 6 },
  member: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderWidth: 1,
    borderRadius: 20,
    paddingVertical: 8,
    paddingHorizontal: 10,
  },
  transcript: { paddingVertical: 8, flexGrow: 1 },
  historyHeader: { marginBottom: 16 },
  empty: { flex: 1, alignItems: "center", justifyContent: "center", gap: 12 },
  message: { gap: 6, padding: 12, maxWidth: "100%" },
  notice: { textAlign: "center", fontSize: 11 },
  working: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingVertical: 16,
  },
  error: { gap: 6 },
  pending: { maxHeight: 180 },
  pendingCard: { padding: 12, gap: 8 },
  actions: { flexDirection: "row", gap: 8 },
  composerArea: { gap: 6, borderTopWidth: 1, paddingTop: 8 },
  recipients: { gap: 6, alignItems: "center" },
  recipient: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 6,
    borderRadius: 20,
    borderWidth: 1,
    minHeight: 36,
  },
  addRecipient: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    minHeight: 36,
    paddingHorizontal: 8,
  },
  composer: {
    flexDirection: "row",
    alignItems: "flex-end",
    borderWidth: 1,
    borderRadius: 20,
    paddingVertical: 4,
  },
  input: {
    flex: 1,
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    maxHeight: 120,
    minHeight: 44,
    paddingTop: 12,
    paddingBottom: 10,
  },
  modalBackdrop: {
    flex: 1,
    backgroundColor: "#0008",
    justifyContent: "flex-end",
  },
  picker: {
    maxHeight: "60%",
    borderWidth: 1,
    borderTopLeftRadius: 24,
    borderTopRightRadius: 24,
    padding: 20,
    paddingBottom: 36,
    gap: 12,
  },
  pickerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    minHeight: 52,
  },
});
