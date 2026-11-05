import {
  ChatComposerDock,
  ChatIconButton as Icon,
  ChatMessageTurn,
  ChatTranscriptSeparator,
  chatSurfaceStyles,
} from "../assistant/ChatSurface";
import { CrewVoiceComposer } from "./CrewVoiceComposer";
import {
  ChatComposerInput,
  ChatComposerToolbar,
} from "../assistant/ChatComposerInput";
import {
  appendDictation,
  useComposerDictation,
} from "../assistant/useComposerDictation";
import { TranscriptViewport } from "../assistant/TranscriptViewport";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { transcriptColumnStyle } from "../display/chatLayout";
import { assistantDraftStore } from "../assistant/assistantDraftStore";
import { ChatHeaderFrame, ChatIdentity } from "../assistant/ChatIdentityHeader";
import { AgentAvatar, AgentAvatarStack } from "../../components/AgentAvatar";
import { SkeletonRows } from "../../components/Skeleton";
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
  LayoutAnimation,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
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
import { chatBubbleColors } from "../../theme/chatAppearance";
import { fonts } from "../../theme/designTokens";
import { AssistantPendingUploads } from "../assistant/AssistantPendingUploads";
import { type AssistantMentionOption } from "../assistant/AssistantMentionInput";
import {
  ChatComposerFrame,
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
import { isReported } from "../../lib/api/replyReportTransport";
import { GroupPendingCard } from "./GroupPendingCard";
import {
  CollaborationCards,
  GroupAgentAccessRequest,
} from "./CollaborationGroupControls";
import { GroupMachineReceipts } from "./GroupMachineReceipts";
import { AssistantMessageFooter } from "../assistant/AssistantMessageFooter";
import { useAssistantUploads } from "../assistant/useAssistantUploads";
import { useTranscriptScrollFollow } from "../assistant/useTranscriptScrollFollow";
import {
  NO_RECIPIENT_EDITS,
  ambiguousGroupMentions,
  groupAgentUnavailableReason,
  groupMentionBindings,
  unavailableGroupRecipients,
  groupCanDecide,
  groupPendingOwned,
  groupPendingReview,
  groupWorkingLabel,
  resolveRecipients,
  withAdded,
  withDraft,
  withRemoved,
  type RecipientSelection,
  type RecipientMember,
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
  const [voiceOpen, setVoiceOpen] = useState(false);
  const [voiceMinimized, setVoiceMinimized] = useState(false);
  function resizeVoice(minimized: boolean) {
    if (voiceMinimized === minimized) return;
    if (!reducedMotion)
      LayoutAnimation.configureNext(LayoutAnimation.Presets.easeInEaseOut);
    setVoiceMinimized(minimized);
  }
  const [localError, setLocalError] = useState<string>();
  const group = chat.isError ? undefined : chat.data?.group;
  const recipientRoster = useRef<Group["members"]>(
    assistantDraftStore.get<Group["members"]>(`recipient-roster:${draftKey}`) ??
      [],
  );
  const recipientMentions = useRef<RecipientMember[]>(
    assistantDraftStore.get<RecipientMember[]>(
      `recipient-mentions:${draftKey}`,
    ) ??
      groupMentionBindings(
        draft,
        [...recipientRoster.current, ...(group?.members ?? [])],
        recipientRoster.current.filter((member) =>
          selection.mentioned.includes(member.id),
        ),
      ),
  );
  // Missing identities stay unavailable; resolved handles are kept separately
  // until that mention is edited away, including when the same ID is renamed.
  const routingMembers = [
    ...(group?.members ?? []),
    ...recipientRoster.current
      .filter(
        (member) =>
          !group?.members.some((row) => row.id === member.id) &&
          [...selection.mentioned, ...selection.added].includes(member.id),
      )
      .map((member) => ({ ...member, unavailable: true })),
  ];
  const mentionBindings = recipientMentions.current.filter((member) =>
    selection.mentioned.includes(member.id),
  );
  recipientRoster.current = routingMembers;
  useEffect(() => {
    const selected = [...selection.mentioned, ...selection.added];
    if (selected.length)
      assistantDraftStore.set(
        `recipient-roster:${draftKey}`,
        routingMembers.filter((member) => selected.includes(member.id)),
      );
    else assistantDraftStore.delete(`recipient-roster:${draftKey}`);
    if (mentionBindings.length)
      assistantDraftStore.set(
        `recipient-mentions:${draftKey}`,
        mentionBindings,
      );
    else assistantDraftStore.delete(`recipient-mentions:${draftKey}`);
  }, [draftKey, routingMembers, selection]);
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
    group?.members.filter(
      (member) => !member.destroyed && !member.unavailable,
    ) ?? [];
  const defaults =
    group?.default_recipient_ids ??
    [
      group?.lead_agent_id ??
        liveMembers.find((member) => member.kind === "nyxbot")?.id ??
        liveMembers[0]?.id,
    ].filter((id): id is string => !!id);
  // Collaboration participants may always talk to the people in the group;
  // an empty recipient list is an explicit people-only message.
  const collaboration = !!group?.collaboration;
  const unavailableMembers = collaboration
    ? (group?.members.filter(
        (member) => member.destroyed || member.unavailable,
      ) ?? [])
    : [];
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
        handle:
          routingMembers.filter(
            (row) => row.name.toLowerCase() === member.name.toLowerCase(),
          ).length === 1
            ? member.name
            : member.id,
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
  function routeDraft(text: string) {
    const resolvedDraft = resolveAttachmentMentions(text, attachmentMentions);
    const ambiguous =
      !selection.added.length &&
      ambiguousGroupMentions(resolvedDraft, routingMembers, mentionBindings)
        .length > 0;
    const routingSelection = withDraft(
      selection,
      resolvedDraft,
      routingMembers,
      mentionBindings,
    );
    const unavailableRecipients = unavailableGroupRecipients(
      routingSelection,
      routingMembers,
    );
    const recipients = resolveRecipients(
      // Route from the text that will reach the server. Recompute on roster
      // changes so an existing @file-1 cannot become a new agent's mention.
      routingSelection,
      defaults,
      group?.members ?? [],
    );
    return {
      resolvedDraft,
      ambiguous,
      routingSelection,
      unavailableRecipients,
      recipients,
    };
  }
  const {
    resolvedDraft,
    ambiguous,
    routingSelection,
    unavailableRecipients,
    recipients,
  } = routeDraft(draft);
  const list = useRef<FlatList<GroupMessage>>(null);
  const transcriptScroll = useTranscriptScrollFollow(list, groupId, {
    loading: chat.isPending,
    historyError: chat.isError,
    reducedMotion,
  });
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
              chat.data?.pending_actions.length ||
              chat.data?.collaboration_decisions?.some(
                (card) => card.can_decide,
              )
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
    chat.data?.collaboration_decisions,
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
  const dictation = useComposerDictation({
    scope: `${chat.epoch}:${groupId}:${composerGeneration}`,
    enabled: !voiceOpen && !blocked && (collaboration || !!liveMembers.length),
    onInsert: (text) => edit(appendDictation(draft, text)),
  });
  function canSubmit(text: string, route = routeDraft(text)) {
    return (
      !dictation.active &&
      !blocked &&
      !route.ambiguous &&
      !route.unavailableRecipients.length &&
      !uploads.blocked &&
      !uploads.session.sending &&
      (collaboration
        ? route.recipients.ids.length > 0 || !uploads.files.length
        : !!route.recipients.ids.length && liveMembers.length > 0) &&
      (!!text.trim() || uploads.files.length > 0) &&
      [...route.resolvedDraft].length <= 32768
    );
  }
  const canSend = canSubmit(draft);

  function edit(value: string) {
    revision.current += 1;
    assistantDraftStore.set(`text:${draftKey}`, value);
    setDraft(value);
    setSelection((old) => {
      const previousBindings = recipientMentions.current.filter((member) =>
        old.mentioned.includes(member.id),
      );
      const text = resolveAttachmentMentions(value, attachmentMentions);
      const next = withDraft(old, text, routingMembers, previousBindings);
      recipientMentions.current = groupMentionBindings(
        text,
        routingMembers,
        previousBindings,
      );
      if (recipientMentions.current.length)
        assistantDraftStore.set(
          `recipient-mentions:${draftKey}`,
          recipientMentions.current,
        );
      else assistantDraftStore.delete(`recipient-mentions:${draftKey}`);
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
  async function send(message = draft) {
    const route = routeDraft(message);
    if (!canSubmit(message, route) || sendLock.current || !chat.available())
      return false;
    sendLock.current = true;
    setLocalError(undefined);
    const target = route.recipients;
    let accepted = false;
    const submittedRoster = [...recipientRoster.current];
    const submittedMentions = groupMentionBindings(
      route.resolvedDraft,
      routingMembers,
      mentionBindings,
    );
    transcriptScroll.follow();
    try {
      await submitGroupDraft({
        draft: message,
        selection: route.routingSelection,
        updateComposer: (text, nextSelection) => {
          // A definite refusal restores the submitted handle bindings together
          // with the text/selection, even if the optimistic clear lost its roster.
          recipientRoster.current = submittedRoster;
          recipientMentions.current = submittedMentions;
          const restoredIds = [
            ...nextSelection.mentioned,
            ...nextSelection.added,
          ];
          if (restoredIds.length)
            assistantDraftStore.set(
              `recipient-roster:${draftKey}`,
              submittedRoster.filter((member) =>
                restoredIds.includes(member.id),
              ),
            );
          else assistantDraftStore.delete(`recipient-roster:${draftKey}`);
          const restoredMentions = submittedMentions.filter((member) =>
            nextSelection.mentioned.includes(member.id),
          );
          if (restoredMentions.length)
            assistantDraftStore.set(
              `recipient-mentions:${draftKey}`,
              restoredMentions,
            );
          else assistantDraftStore.delete(`recipient-mentions:${draftKey}`);
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
        send: async (text) => {
          accepted = await uploads.session.submit((files) =>
            chat.post({
              text: resolveAttachmentMentions(text, attachmentMentions),
              ...(files?.attachmentIds.length
                ? { attachment_ids: files.attachmentIds }
                : {}),
              // Freeze the visible recipients while resolving local file
              // aliases, including a member joining between edit and POST.
              recipient_agent_ids: target.ids,
            }),
          );
          return accepted;
        },
      });
      return accepted;
    } catch (cause) {
      if (mounted.current && chat.available())
        setLocalError(
          cause instanceof Error
            ? cause.message
            : "The message was not delivered. Refresh the crew before trying again.",
        );
      return false;
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
        <View style={styles.errorRow}>
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
      <TranscriptViewport
        ready={transcriptScroll.ready}
        onTouchStart={() => {
          if (voiceOpen) resizeVoice(true);
        }}
      >
        <FlatList
          ref={list}
          style={styles.fill}
          data={group ? (chat.data?.messages ?? []) : []}
          keyExtractor={(item) => item.id}
          ItemSeparatorComponent={ChatTranscriptSeparator}
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
          ListHeaderComponentStyle={styles.transcriptHeader}
          ListEmptyComponent={
            chat.isPending ? (
              <SkeletonRows rows={4} accessibilityLabel="Loading crew" />
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
            <View style={styles.footer}>
              {working.length ? (
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
              ) : null}
              {!!group?.collaboration &&
                !!chat.data?.collaboration_decisions?.length && (
                  <View
                    style={{ gap: 8 }}
                    accessibilityLabel="Collaboration decisions"
                  >
                    <CollaborationCards
                      key={`${chat.epoch}:${group.id}`}
                      group={group}
                      decisions={chat.data?.collaboration_decisions ?? []}
                    />
                  </View>
                )}
              {!!group &&
                !group.collaboration &&
                !!chat.data?.pending_actions.length && (
                  <View
                    accessibilityLabel="Actions waiting for you"
                    style={{ gap: 8 }}
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
                          name={
                            member ? title(member.id, member.name) : "An agent"
                          }
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
                                      acknowledgementId:
                                        review.acknowledgementId,
                                    });
                                }
                              : undefined
                          }
                          onDecision={(choice) => void decide(action, choice)}
                          onAnswer={(yes) => void answer(action, yes)}
                        />
                      );
                    })}
                  </View>
                )}
            </View>
          }
        />
        <JumpToLatest
          visible={transcriptScroll.showJump}
          onPress={transcriptScroll.follow}
        />
        <ScrollEdgeFades edges={transcriptEdges.edges} color={colors.bg} />
      </TranscriptViewport>
      {group && (
        <ChatComposerDock>
          {ambiguous && (
            <Label style={[styles.small, { color: colors.danger }]}>
              This handle names multiple agents. Choose an exact recipient or
              use its UUID mention before sending.
            </Label>
          )}
          {!!unavailableRecipients.length && (
            <Label style={[styles.small, { color: colors.danger }]}>
              A selected agent is no longer available. Remove it and choose
              another recipient before sending.
            </Label>
          )}
          {!recipients.ids.length &&
            (collaboration ? (
              <Label muted style={styles.small}>
                {uploads.files.length
                  ? "Attachments need an agent recipient. Add an agent or remove the attachments."
                  : "Everyone in the group · no agent reply"}
              </Label>
            ) : (
              <Label style={[styles.small, { color: colors.danger }]}>
                Choose at least one recipient.
              </Label>
            ))}
          {!liveMembers.length &&
            (collaboration ? (
              <Label muted>
                No agent here is available to you. You can still message the
                people in this group.
              </Label>
            ) : (
              <Label muted>
                All agents have been removed or destroyed. This crew is
                read-only.
              </Label>
            ))}
          {!canParticipate && (
            <Label muted>
              This crew is read-only for your current organization role.
            </Label>
          )}
          {(uploads.error || uploads.notice) && (
            <Label muted>{uploads.error ?? uploads.notice}</Label>
          )}
          {voiceOpen ? (
            <CrewVoiceComposer
              crewName={group.name}
              scope={`${chat.epoch}:${groupId}`}
              enabled={
                !chat.recoveryRequired &&
                !chat.loadingOlder &&
                chat.active &&
                canParticipate &&
                !chat.isError &&
                chat.online &&
                !draft.length &&
                !uploads.files.length
              }
              busy={chat.sending || uploads.session.sending}
              messages={chat.data?.messages ?? []}
              onSend={send}
              onClose={() => setVoiceOpen(false)}
              minimized={voiceMinimized}
              onExpand={() => resizeVoice(false)}
              draft={draft}
              onChangeText={edit}
              canSendText={canSend}
              onSendText={() => void send()}
              recipientLabel={
                recipients.ids
                  .map((id) =>
                    title(
                      id,
                      liveMembers.find((member) => member.id === id)?.name ??
                        id,
                    ),
                  )
                  .join(", ") || "everyone in the group"
              }
            />
          ) : (
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
                {unavailableRecipients.map((id) => (
                  <Pressable
                    key={`unavailable:${id}`}
                    accessibilityRole="button"
                    accessibilityLabel={`Remove unavailable recipient ${title(id, routingMembers.find((member) => member.id === id)?.name ?? id)}`}
                    disabled={blocked}
                    onPress={() =>
                      editRecipients((old) => withRemoved(old, id))
                    }
                    style={[styles.recipient, { borderColor: colors.danger }]}
                  >
                    <Label style={[styles.small, { color: colors.danger }]}>
                      {title(
                        id,
                        routingMembers.find((member) => member.id === id)
                          ?.name ?? id,
                      )}{" "}
                      · Unavailable
                    </Label>
                    <X size={12} color={colors.danger} />
                  </Pressable>
                ))}
                {(liveMembers.some(
                  (member) => !recipients.ids.includes(member.id),
                ) ||
                  unavailableMembers.length > 0) && (
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
              <ChatComposerInput
                dictation={dictation}
                generation={composerGeneration}
                accessibilityLabel="Message the crew"
                placeholder="Message the crew"
                value={draft}
                mentions={mentions}
                onChangeText={(value) => {
                  if (currentComposer.current === composerGeneration)
                    edit(value);
                }}
                editable={!blocked && (collaboration || !!liveMembers.length)}
                renderActions={(mentionAction) => (
                  <ChatComposerToolbar
                    dictation={dictation}
                    dictationDisabled={
                      blocked || uploads.picking || uploads.session.sending
                    }
                    attach={{
                      label: "Attach photos or files, or take a photo to crew",
                      busy: uploads.picking,
                      disabled:
                        blocked ||
                        dictation.active ||
                        !liveMembers.length ||
                        uploads.picking ||
                        uploads.session.sending ||
                        uploads.files.length >= 10,
                      onPress: uploads.choose,
                    }}
                    mention={mentionAction}
                    settings={
                      <Icon
                        label="Crew composer settings"
                        onPress={onSettings}
                        disabled={blocked || dictation.active}
                      >
                        <Settings2 size={20} color={colors.textSecondary} />
                      </Icon>
                    }
                  >
                    <ComposerSubmitButton
                      kind={
                        !draft.trim() && !uploads.files.length
                          ? "voice"
                          : "send"
                      }
                      label={
                        !draft.trim() && !uploads.files.length
                          ? "Start crew voice"
                          : "Send crew message"
                      }
                      busy={chat.sending}
                      disabled={
                        !draft.trim() && !uploads.files.length
                          ? blocked ||
                            !chat.online ||
                            dictation.active ||
                            (!collaboration && !liveMembers.length)
                          : !canSend
                      }
                      onPress={() => {
                        if (!draft.trim() && !uploads.files.length) {
                          setVoiceMinimized(false);
                          setVoiceOpen(true);
                        } else void send();
                      }}
                    />
                  </ChatComposerToolbar>
                )}
              />
            </ChatComposerFrame>
          )}
          {[...resolvedDraft].length > 32768 && (
            <Label style={[styles.small, { color: colors.danger }]}>
              Keep the message within 32,768 characters, including attachment
              references.
            </Label>
          )}
        </ChatComposerDock>
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
              {group &&
                pickingRecipients !== "header" &&
                unavailableMembers.map((member) => (
                  <View
                    key={`unavailable:${member.id}`}
                    style={styles.unavailableMember}
                  >
                    <Label numberOfLines={1}>
                      {title(member.id, member.display_name ?? member.name)}
                      {member.owner_name ? ` · ${member.owner_name}` : ""}
                    </Label>
                    <Label muted style={styles.small}>
                      {groupAgentUnavailableReason(member)}
                    </Label>
                    <GroupAgentAccessRequest group={group} member={member} />
                  </View>
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
  const header = (
    <>
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
          <Label muted style={styles.messageLabel}>
            {user
              ? (message.author?.display_name ?? "You")
              : message.agent
                ? title(message.agent.id, message.agent.name)
                : "Agent"}{" "}
          </Label>
        </View>
      </Pressable>
      {message.requester_user_id && (
        <Label muted>
          For{" "}
          {group?.participants?.find(
            (person) => person.id === message.requester_user_id,
          )?.display_name ?? "participant"}
          {message.execution_mode === "guest"
            ? " · Guest reply (agent owner pays)"
            : " · Requester access and billing"}
        </Label>
      )}
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
    </>
  );
  const footer = (
    <AssistantMessageFooter
      text={user ? message.text : sanitizeAssistantMessageContent(message.text)}
      timestamp={message.created_at}
      user={user}
      report={
        message.role === "agent" && group
          ? {
              kind: "group",
              id: group.id,
              messageId: message.id,
              reported: isReported(message.feedback),
            }
          : undefined
      }
    />
  );
  return (
    <ChatMessageTurn user={user} author={header} footer={footer}>
      {user ? (
        <>
          <Text
            selectable
            style={[styles.messageText, { color: bubble.foreground }]}
          >
            {message.text}
          </Text>
          <GroupMachineReceipts
            groupId={group?.id}
            message={message}
            foreground={bubble.foreground}
          />
        </>
      ) : (
        <AssistantReplyBody
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
    </ChatMessageTurn>
  );
}

const styles = {
  ...chatSurfaceStyles,
  ...StyleSheet.create({
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
    header: { flexDirection: "row", alignItems: "center", gap: 8 },
    heading: { fontFamily: fonts.displaySemi, fontSize: 18 },
    small: { fontSize: 11, lineHeight: 17 },
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
    empty: { flex: 1, alignItems: "center", justifyContent: "center", gap: 12 },
    notice: { textAlign: "center", fontSize: 11 },
    working: {
      flexDirection: "row",
      alignItems: "center",
      gap: 8,
      paddingVertical: 16,
    },
    pendingCard: { padding: 12, gap: 8 },
    actions: { flexDirection: "row", gap: 8 },
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
    unavailableMember: { gap: 4, paddingVertical: 10 },
  }),
};
