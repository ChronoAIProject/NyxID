import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { transcriptColumnStyle } from "../display/chatLayout";
import { assistantDraftStore } from "../assistant/assistantDraftStore";
import { ChatHeaderFrame, ChatIdentity } from "../assistant/ChatIdentityHeader";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import type { NativeStackNavigationOptions } from "@react-navigation/native-stack";
import { useChatColumn } from "../display/ChatLayoutContext";
import {
  ScrollEdgeFades,
  useScrollEdges,
  OverflowScrollView,
} from "../../components/workspace/ScrollEdgeFades";
import { useEffect, useMemo, useRef, useState } from "react";
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
import { AssistantReplyBody } from "../assistant/AssistantReplyBody";
import { Check, Plus, Settings2, X } from "lucide-react-native";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { accountApi } from "../../lib/api/accountApi";
import {
  useWorkspaceBack,
  WorkspaceHeaderVisibility,
} from "../../app/WorkspaceHeader";
import { agentsApi } from "../../lib/api/agentsApi";
import { groupsApi } from "../../lib/api/groupsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { chatBubbleColors, userBubbleCorners } from "../../theme/chatAppearance";
import { fonts, radius } from "../../theme/designTokens";
import { AssistantPendingUploads } from "../assistant/AssistantPendingUploads";
import {
  AssistantMentionInput,
  type AssistantMentionOption,
} from "../assistant/AssistantMentionInput";
import {
  ChatComposerFrame,
  ChatComposerRow,
  ComposerAttachButton,
  ComposerSubmitButton,
} from "../assistant/ChatComposer";
import {
  resolveAttachmentMentions,
  stripAttachmentMention,
  type AttachmentMention,
} from "../assistant/assistantMentions";
import { AssistantActivityDisclosure } from "../assistant/AssistantActivityDisclosure";
import {
  AssistantConnectionBanner,
  AssistantErrorNotice,
  JumpToLatest,
} from "../assistant/AssistantBanner";
import { sanitizeAssistantMessageContent } from "../assistant/assistantChatContent";
import { GroupPendingCard } from "./GroupPendingCard";
import { GroupMachineReceipts } from "./GroupMachineReceipts";
import { AssistantMessageFooter } from "../assistant/AssistantMessageFooter";
import { useAssistantUploads } from "../assistant/useAssistantUploads";
import { useTranscriptScrollFollow } from "../assistant/useTranscriptScrollFollow";
import {
  NO_RECIPIENT_EDITS,
  groupCanDecide,
  groupPendingOwned,
  groupPendingReview,
  groupWorkingLabel,
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
  const reducedMotion = useReducedMotion();
  const chatColumn = useChatColumn();
  const transcriptEdges = useScrollEdges();
  const chat = useGroupChat(groupId);
  const draftKey = `crew:${groupId}`;
  const [draft, setDraft] = useState(
    () => assistantDraftStore.get<string>(`text:${draftKey}`) ?? "",
  );
  const [composerGeneration, setComposerGeneration] = useState(0);
  const currentComposer = useRef(0);
  const revision = useRef(0);
  const [selection, setSelection] = useState<RecipientSelection>(
    () =>
      assistantDraftStore.get<RecipientSelection>(`recipients:${draftKey}`) ??
      NO_RECIPIENT_EDITS,
  );
  const [pickingRecipients, setPickingRecipients] = useState<
    false | "add" | "header"
  >(false);
  const [localError, setLocalError] = useState<string>();
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
  const canParticipate =
    !!group &&
    (group.owner?.type !== "org" ||
      ["creator", "admin", "participant"].includes(group.your_role ?? ""));
  const uploads = useAssistantUploads({
    retainKey: draftKey,
    groupId,
    generation: 0,
    enabled:
      canParticipate &&
      !chat.isError &&
      !chat.recoveryRequired &&
      liveMembers.length > 0,
    online: chat.online,
    reservedMentionHandles: liveMembers.map((member) => member.name),
  });
  const attachmentMentions: AttachmentMention[] = uploads.files.map((row) => ({
    handle: row.handle,
    label: row.item?.label ?? row.file.name,
    kind: row.kind,
  }));
  const mentions: AssistantMentionOption[] = [
    ...liveMembers
      // A roster update cannot change an existing attachment's stable handle.
      // The same-name agent remains selectable through the recipient pills.
      .filter(
        (member) =>
          !attachmentMentions.some(
            (attachment) =>
              attachment.handle.toLowerCase() === member.name.toLowerCase(),
          ),
      )
      .map((member) => ({
        id: `agent:${member.id}`,
        handle: member.name,
        label: title(member.id, member.name),
        kind: "agent" as const,
        agent: member,
      })),
    ...uploads.files.map((row) => ({
      id: `attachment:${row.key}`,
      handle: row.handle,
      label: row.item?.label ?? row.file.name,
      kind: row.kind,
      detail: row.error ?? (row.item ? "Ready" : "Uploading…"),
    })),
  ];
  const resolvedDraft = resolveAttachmentMentions(draft, attachmentMentions);
  const recipients = resolveRecipients(
    // Route from the text that will reach the server. Recompute on roster
    // changes so an existing @file-1 cannot become a new agent's mention.
    withDraft(selection, resolvedDraft, group?.members ?? []),
    defaults,
    group?.members ?? [],
  );
  const list = useRef<FlatList<GroupMessage>>(null);
  const transcriptScroll = useTranscriptScrollFollow(list, groupId);
  const sendLock = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const groupHeaderOptions = useMemo<NativeStackNavigationOptions>(() => {
    const settings = () => (
      <WorkspaceHeaderVisibility>
        <Icon
          label="Crew settings"
          onPress={onSettings}
          disabled={!group || chat.sending || uploads.session.sending}
        >
          <Settings2 size={20} color={colors.textSecondary} />
        </Icon>
      </WorkspaceHeaderVisibility>
    );
    return {
      header: ChatHeaderFrame,
      headerTitle: () => (
        <WorkspaceHeaderVisibility>
          <ChatIdentity
            name={group?.name ?? "Crew"}
            detail={
              group?.members
                .map((member) => title(member.id, member.name))
                .join(", ") || "Crew members"
            }
            members={group?.members ?? []}
            status={
              chat.data?.pending_actions.length
                ? "Needs you"
                : group?.working_agent_ids.length
                  ? "Running"
                  : undefined
            }
            disabled={!group || chat.sending || uploads.session.sending}
            onPress={() => setPickingRecipients("header")}
          />
        </WorkspaceHeaderVisibility>
      ),
      headerRight: settings,
      unstable_headerRightItems: () => [
        { type: "custom", element: settings(), hidesSharedBackground: true },
      ],
    };
  }, [
    group,
    chat.data?.pending_actions.length,
    names.data,
    Boolean(group),
    chat.sending,
    uploads.session.sending,
    onSettings,
    colors,
  ]);
  useWorkspaceBack({
    options: groupHeaderOptions,
    active: chat.sending || uploads.session.sending,
    disabled: chat.sending || uploads.session.sending,
    defaultRole: "back",
    onBack,
    label: "Back to chat",
  });
  useEffect(() => {
    if (!chat.active) {
      setPickingRecipients(false);
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
    [...resolvedDraft].length <= 32768;

  function edit(value: string) {
    revision.current += 1;
    assistantDraftStore.set(`text:${draftKey}`, value);
    setDraft(value);
    setSelection((old) => {
      const next = withDraft(
        old,
        resolveAttachmentMentions(value, attachmentMentions),
        group?.members ?? [],
      );
      assistantDraftStore.set(`recipients:${draftKey}`, next);
      return next;
    });
  }
  function editRecipients(
    update: (old: RecipientSelection) => RecipientSelection,
  ) {
    revision.current += 1;
    setSelection((old) => {
      const next = update(old);
      assistantDraftStore.set(`recipients:${draftKey}`, next);
      return next;
    });
  }
  function removeAttachment(key: string) {
    if (chat.sending || uploads.session.sending) return;
    const row = uploads.files.find((file) => file.key === key);
    if (!row) return;
    const next = stripAttachmentMention(draft, row.handle);
    if (next !== draft) edit(next);
    uploads.remove(key);
  }
  async function send() {
    if (!canSend || sendLock.current || !chat.available()) return;
    sendLock.current = true;
    setLocalError(undefined);
    const target = recipients;
    transcriptScroll.follow();
    try {
      await submitGroupDraft({
        draft,
        selection,
        updateComposer: (text, nextSelection) => {
          revision.current += 1;
          // Discard native marked text and late IME events from the sent draft.
          currentComposer.current += 1;
          setComposerGeneration(currentComposer.current);
          assistantDraftStore.set(`text:${draftKey}`, text);
          assistantDraftStore.set(`recipients:${draftKey}`, nextSelection);
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
              text: resolveAttachmentMentions(text, attachmentMentions),
              ...(files?.attachmentIds.length
                ? { attachment_ids: files.attachmentIds }
                : {}),
              // Freeze the visible recipients while resolving local file
              // aliases, including a member joining between edit and POST.
              ...(target.edited || files?.attachmentIds.length
                ? { recipient_agent_ids: target.ids }
                : {}),
            }),
          ),
      });
    } catch (cause) {
      if (mounted.current && chat.available())
        setLocalError(
          cause instanceof Error
            ? cause.message
            : "The message was not delivered. Refresh the crew before trying again.",
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
  const working =
    group?.members.filter((member) =>
      group.working_agent_ids.includes(member.id),
    ) ?? [];
  const stableTranscript =
    !!chat.data?.messages.length && !chat.sending && !working.length;

  return (
    <WorkspacePage
      title={group?.name ?? "Crew"}
      showHeading={false}
      scroll={false}
      contentStyle={styles.chatPage}
    >
      <AssistantConnectionBanner
        inset
        onRetry={() => void chat.recoverDelivery()}
      />
      {(chat.recoveryRequired ||
        chat.error ||
        chat.actionError ||
        localError) && (
        <View style={styles.error}>
          <AssistantErrorNotice
            message={
              chat.recoveryRequired
                ? "This message may have been delivered. Refresh the crew and check its history before sending again."
                : (localError ??
                  chat.actionError ??
                  (chat.error instanceof Error
                    ? chat.error.message
                    : "Could not load this crew."))
            }
          />
          <ActionButton
            label="Refresh crew"
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
        <OverflowScrollView
          fadeColor={colors.bg}
          horizontal
          style={styles.horizontalStrip}
          showsHorizontalScrollIndicator={false}
          contentContainerStyle={[styles.members, { paddingHorizontal: 12 }]}
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
              <AgentAvatar agent={member} size="sm" />
              <Label style={styles.small}>
                {title(member.id, member.name)}
              </Label>
            </Pressable>
          ))}
        </OverflowScrollView>
      )}
      <View style={styles.fill}>
        <FlatList
          ref={list}
          style={styles.fill}
          data={group ? (chat.data?.messages ?? []) : []}
          keyExtractor={(item) => item.id}
          ItemSeparatorComponent={GroupSeparator}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="on-drag"
          maintainVisibleContentPosition={
            stableTranscript && !transcriptScroll.following
              ? { minIndexForVisible: 0 }
              : undefined
          }
          contentContainerStyle={[
            styles.transcript,
            transcriptColumnStyle(chatColumn.maxWidth),
          ]}
          {...transcriptScroll.listProps}
          onLayout={(event) => {
            transcriptScroll.listProps.onLayout(event);
            transcriptEdges.layout(event.nativeEvent.layout.height);
          }}
          onContentSizeChange={(width, height) => {
            transcriptScroll.listProps.onContentSizeChange(width, height);
            transcriptEdges.content(height);
          }}
          onScroll={(event) => {
            transcriptScroll.listProps.onScroll(event);
            transcriptEdges.scroll(event.nativeEvent.contentOffset.y);
          }}
          ListHeaderComponent={
            chat.data?.before_seq ? (
              <ActionButton
                label="Load earlier messages"
                variant="secondary"
                loading={chat.loadingOlder}
                disabled={blocked}
                onPress={() => {
                  transcriptScroll.pause();
                  void chat.loadOlder();
                }}
              />
            ) : null
          }
          ListHeaderComponentStyle={styles.historyHeader}
          ListEmptyComponent={
            chat.isPending ? (
              <ActivityIndicator
                accessibilityLabel="Loading crew"
                color={colors.primary}
              />
            ) : group ? (
              <View style={styles.empty}>
                <AgentAvatarStack agents={group.members} size="lg" />
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
                chat.sending || uploads.session.sending
                  ? undefined
                  : onOpenAgent
              }
              onDisclosure={() => {
                transcriptScroll.pause();
              }}
            />
          )}
          ListFooterComponent={
            working.length ? (
              <View
                style={styles.working}
                accessibilityRole="text"
                accessibilityLabel={`Agents working. ${groupWorkingLabel(working.map((member) => title(member.id, member.name)))}`}
                accessibilityLiveRegion="polite"
              >
                <AgentAvatar agent={working[0]!} size="md" />
                <ActivityIndicator size="small" color={colors.primary} />
                <Label muted>
                  {groupWorkingLabel(
                    working.map((member) => title(member.id, member.name)),
                  )}
                </Label>
              </View>
            ) : null
          }
        />
        <JumpToLatest
          visible={transcriptScroll.showJump}
          onPress={transcriptScroll.follow}
        />
        <ScrollEdgeFades edges={transcriptEdges.edges} color={colors.bg} />
      </View>
      {!!group && !!chat.data?.pending_actions.length && (
        <ScrollView
          accessibilityLabel="Actions waiting for you"
          style={styles.pending}
          contentContainerStyle={{ gap: 8 }}
          keyboardShouldPersistTaps="handled"
        >
          {chat.data.pending_actions.map((action, index) => {
            const member = group?.members.find(
              (row) => row.id === action.agent_id,
            );
            return (
              <GroupPendingCard
                key={
                  action.acknowledgement_id ??
                  action.approval_request_id ??
                  `${action.conversation_id}:${index}`
                }
                action={action}
                name={member ? title(member.id, member.name) : "An agent"}
                actorId={identity.data?.id}
                organization={group?.owner?.type === "org"}
                blocked={blocked}
                onReview={
                  onOpenConversation
                    ? () => {
                        if (blocked || !chat.available()) return;
                        const review = groupPendingReview(
                          action,
                          identity.data?.id,
                          group?.owner?.type === "org",
                        );
                        if (review)
                          onOpenConversation({
                            conversationId: review.conversationId,
                            agentId: action.agent_id ?? undefined,
                            acknowledgementId: review.acknowledgementId,
                          });
                      }
                    : undefined
                }
                onDecision={(choice) => void decide(action, choice)}
                onAnswer={(yes) => void answer(action, yes)}
              />
            );
          })}
        </ScrollView>
      )}
      {group && (
        <View style={styles.composerInset}>
          <View style={[styles.composerArea, chatColumn]}>
            {!recipients.ids.length && (
              <Label style={[styles.small, { color: colors.danger }]}>
                Choose at least one recipient.
              </Label>
            )}
            {!liveMembers.length && (
              <Label muted>
                All agents have been removed or destroyed. This crew is
                read-only.
              </Label>
            )}
            {!canParticipate && (
              <Label muted>
                This crew is read-only for your current organization role.
              </Label>
            )}
            {(uploads.error || uploads.notice) && (
              <Label muted>{uploads.error ?? uploads.notice}</Label>
            )}
            <ChatComposerFrame>
              <OverflowScrollView
                fadeColor={colors.composerBg}
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
                      onPress={() =>
                        editRecipients((old) => withRemoved(old, id))
                      }
                      style={[
                        styles.recipient,
                        {
                          backgroundColor: colors.navActive,
                          borderColor: colors.composerBorder,
                        },
                      ]}
                    >
                      <Label style={styles.small}>
                        {title(id, member.name)}
                      </Label>
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
                    onPress={() => setPickingRecipients("add")}
                    style={styles.addRecipient}
                  >
                    <Plus size={14} color={colors.primaryOnTint} />
                    <Label
                      style={[styles.small, { color: colors.primaryOnTint }]}
                    >
                      Add
                    </Label>
                  </Pressable>
                )}
              </OverflowScrollView>
              <AssistantPendingUploads
                files={uploads.files}
                remove={removeAttachment}
                disabled={chat.sending || uploads.session.sending}
              />
              <AssistantMentionInput
                generation={composerGeneration}
                accessibilityLabel="Message the crew"
                placeholder="Message the crew"
                value={draft}
                mentions={mentions}
                onChangeText={(value) => {
                  if (currentComposer.current === composerGeneration)
                    edit(value);
                }}
                editable={!blocked && !!liveMembers.length}
                style={[styles.input, { color: colors.textPrimary }]}
                renderActions={(mentionAction) => (
                  <ChatComposerRow>
                    <ComposerAttachButton
                      label="Attach photos or files, or take a photo to crew"
                      busy={uploads.picking}
                      disabled={
                        blocked ||
                        !liveMembers.length ||
                        uploads.picking ||
                        uploads.session.sending
                      }
                      onPress={uploads.choose}
                    />
                    {mentionAction}
                    <View style={{ flex: 1 }} />

                    <ComposerSubmitButton
                      kind="send"
                      label="Send crew message"
                      busy={chat.sending}
                      disabled={!canSend}
                      onPress={() => void send()}
                    />
                  </ChatComposerRow>
                )}
              />
            </ChatComposerFrame>
            {[...resolvedDraft].length > 32768 && (
              <Label style={[styles.small, { color: colors.danger }]}>
                Keep the message within 32,768 characters, including attachment
                references.
              </Label>
            )}
          </View>
        </View>
      )}
      <Modal
        visible={!!pickingRecipients}
        transparent
        animationType={reducedMotion ? "none" : "slide"}
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
              <Label style={styles.heading}>
                {pickingRecipients === "header"
                  ? "Choose agent"
                  : "Add recipient"}
              </Label>
              <Icon
                label="Close recipients"
                onPress={() => setPickingRecipients(false)}
              >
                <X size={22} color={colors.textPrimary} />
              </Icon>
            </View>
            <ScrollView>
              {liveMembers
                .filter(
                  (member) =>
                    pickingRecipients === "header" ||
                    !recipients.ids.includes(member.id),
                )
                .map((member) => (
                  <Pressable
                    key={member.id}
                    accessibilityRole="button"
                    accessibilityLabel={`${pickingRecipients === "header" ? "Message" : "Add recipient"} ${title(member.id, member.name)}`}
                    accessibilityState={{
                      selected: recipients.ids.includes(member.id),
                      disabled: blocked,
                    }}
                    disabled={blocked}
                    style={styles.pickerRow}
                    onPress={() => {
                      if (blocked || !chat.available()) return;
                      editRecipients((old) =>
                        pickingRecipients === "header"
                          ? liveMembers.reduce(
                              (selection, candidate) =>
                                candidate.id === member.id
                                  ? withAdded(selection, candidate.id)
                                  : withRemoved(selection, candidate.id),
                              old,
                            )
                          : withAdded(old, member.id),
                      );
                      setPickingRecipients(false);
                    }}
                  >
                    <AgentAvatar agent={member} size="lg" />
                    <Label numberOfLines={1}>
                      {title(member.id, member.name)}
                    </Label>
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
}: {
  group?: Group;
  message: GroupMessage;
  title: (id: string, fallback: string) => string;
  onOpenAgent?: (id: string) => void;
  onDisclosure: () => void;
}) {
  const { mode, bubblePalette } = useTheme();
  const bubble = chatBubbleColors(mode, bubblePalette);
  const user = message.role === "user";
  if (message.role === "notice")
    return (
      <View>
        <Label
          muted
          selectable
          accessibilityLabel="Crew notice"
          style={styles.notice}
        >
          {message.text}
        </Label>
        <GroupMachineReceipts groupId={group?.id} message={message} />
      </View>
    );
  return (
    <View
      style={[
        styles.message,
        user && {
          backgroundColor: bubble.background,
          ...userBubbleCorners,
          marginLeft: 38,
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
        <View style={styles.row}>
          {message.agent ? (
            <AgentAvatar agent={message.agent} size="sm" />
          ) : null}
          <Label muted style={[styles.small, user && { color: bubble.foreground }]}>
            {user
              ? (message.author?.display_name ?? "You")
              : message.agent
                ? title(message.agent.id, message.agent.name)
                : "Agent"}{" "}
          </Label>
        </View>
      </Pressable>
      {user && !!message.recipient_agent_ids?.length && (
        <Label style={[styles.small, { color: bubble.foreground }]}>
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
        <>
          <Label selectable style={{ color: bubble.foreground }}>{message.text}</Label>
          <GroupMachineReceipts groupId={group?.id} message={message} foreground={bubble.foreground} />
        </>
      ) : (
        <AssistantReplyBody
          variant="crew"
          text={sanitizeAssistantMessageContent(message.text)}
          activity={
            message.activities.length || message.attachments.length ? (
              <>
                <AssistantActivityDisclosure
                  activities={message.activities}
                  onToggle={onDisclosure}
                />
                <GroupMachineReceipts groupId={group?.id} message={message} />
              </>
            ) : undefined
          }
        />
      )}
      <AssistantMessageFooter
        text={
          user ? message.text : sanitizeAssistantMessageContent(message.text)
        }
        timestamp={message.created_at}
        user={user}
        foreground={user ? bubble.foreground : undefined}
      />
    </View>
  );
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
  row: { flexDirection: "row", alignItems: "center", gap: 6 },
  horizontalStrip: { flexGrow: 0 },
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
  error: { gap: 6, marginHorizontal: 12 },
  pending: { maxHeight: 180, marginHorizontal: 12 },
  pendingCard: { padding: 12, gap: 8 },
  actions: { flexDirection: "row", gap: 8 },
  composerInset: { paddingHorizontal: 12 },
  chatPage: { paddingHorizontal: 0, paddingTop: 0, paddingBottom: 4, gap: 8 },
  composerArea: { gap: 6, paddingTop: 4 },
  recipients: { gap: 6, alignItems: "center", paddingHorizontal: 10 },
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
  input: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
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
