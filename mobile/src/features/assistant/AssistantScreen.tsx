import {
  ChatComposerDock,
  ChatIconButton as IconButton,
  ChatMessageTurn,
  ChatTranscriptSeparator,
  chatSurfaceStyles,
} from "./ChatSurface";
import { TranscriptViewport } from "./TranscriptViewport";
import { ChatComposerInput, ChatComposerToolbar } from "./ChatComposerInput";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import {
  AssistantApprovalCard,
  AssistantWaitingNote,
} from "./AssistantPendingState";
import { AssistantSystemMessage } from "./AssistantSystemMessage";
import { AssistantLinkContext } from "./AssistantLinkContext";
import { AssistantComposerSettings } from "./AssistantComposerSettings";
import { useComposerConnection } from "./useComposerConnection";
import { useAssistantGuidance } from "./useAssistantGuidance";
import { ComposerConnectionPicker } from "./ComposerConnectionPicker";
import { transcriptColumnStyle } from "../display/chatLayout";
import { newAssistantChatParams } from "../../app/assistantChatParams";
import { assistantDraftKey, assistantDraftStore } from "./assistantDraftStore";
import { SkeletonRows } from "../../components/Skeleton";
import { WorkspaceChatList } from "../../components/workspace/WorkspaceChatList";
import {
  ChatHeaderFrame,
  ChatIdentity,
  chatThreadInfo,
} from "./ChatIdentityHeader";
import { useChatColumn } from "../display/ChatLayoutContext";
import { MachineToolCard } from "../machines/MachineToolCard";
import {
  ScrollEdgeFades,
  useScrollEdges,
} from "../../components/workspace/ScrollEdgeFades";
import {
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AccessibilityInfo,
  Alert,
  FlatList,
  findNodeHandle,
  Keyboard,
  LayoutAnimation,
  Modal,
  Platform,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useIsFocused, useNavigation } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import type { NativeStackNavigationOptions } from "@react-navigation/native-stack";
import * as WebBrowser from "expo-web-browser";
import { AssistantReplyBody } from "./AssistantReplyBody";
import { MoreHorizontal, RotateCcw, SquarePen, X } from "lucide-react-native";
import type {
  AssistantParams,
  WorkspaceNavigationProp,
} from "../../app/navigationTypes";
import {
  useWorkspaceBack,
  WorkspaceHeaderVisibility,
} from "../../app/WorkspaceHeader";
import {
  openWorkspaceSheetAction,
  openWorkspacePageAction,
  workspaceNavigateAction,
} from "../../app/navigationHierarchy";
import {
  WorkspacePage,
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { chatBubbleColors } from "../../theme/chatAppearance";
import { fonts } from "../../theme/designTokens";
import { useAssistant } from "./useAssistant";
import { NyxBotHome } from "./NyxBotHome";
import { showNyxBotHome } from "./nyxBotHomeModel";
import { chatAgentLabel } from "./assistantAgentContext";
import { liveReplyText } from "./assistantStream";
import {
  ASSISTANT_SEND_PRESS_GRACE_MS,
  createAssistantComposerPressGuard,
} from "./assistantComposerPress";
import { submitAssistantDraft } from "./assistantComposerDraft";
import { AssistantActivityDisclosure } from "./AssistantActivityDisclosure";
import { AssistantAcknowledgements } from "./AssistantAcknowledgements";
import type { RevealReviewTarget } from "./AssistantReviewTarget";
import { revealAssistantReviewTarget } from "./reviewTargetGeometry";
import {
  AssistantBanner,
  AssistantConnectionBanner,
  AssistantErrorNotice,
  JumpToLatest,
} from "./AssistantBanner";
import { sanitizeAssistantMessageContent } from "./assistantChatContent";
import { isReported } from "../../lib/api/replyReportTransport";
import { AssistantMessageFooter } from "./AssistantMessageFooter";
import { AssistantAttachmentView } from "./AssistantAttachmentView";
import { AssistantPendingUploads } from "./AssistantPendingUploads";
import { ComposerSubmitButton } from "./ChatComposer";
import { ComposerDictationFrame } from "./ComposerDictation";
import { AssistantMentionInput } from "./AssistantMentionInput";
import { useAssistantUploads } from "./useAssistantUploads";
import { appendDictation, useComposerDictation } from "./useComposerDictation";
import { AssistantCmaOperation } from "./AssistantCmaOperation";
import { AssistantAcknowledgementCard } from "./AssistantAcknowledgementCard";
import { AssistantTurnStatus } from "./AssistantTurnStatus";
import {
  AssistantMessageNotices,
  QuietButton,
} from "./AssistantMessageNotices";
import { contextResetCopy, steeringNotice } from "./assistantTurnPresentation";
import { NativeAuthoredSkillReview } from "../plugins/NativeAuthoredSkillReview";
import { parseAutomationLink } from "../automations/automationLinks";
import { getFrontendUrl, firstPartyFrontendUrls } from "../../lib/env";
import { useTranscriptScrollFollow } from "./useTranscriptScrollFollow";
import { AssistantVoiceConversation } from "./voice/AssistantVoiceConversation";
import {
  mergeVoiceTranscript,
  retainVoiceSnapshot,
  voiceTranscriptLabel,
  withoutLiveUserSpeech,
  type VoiceTranscriptMessage,
} from "./voice/voiceTranscript";
import type { VoiceSnapshot } from "./voice/voiceContracts";
import {
  NATIVE_VOICE_ENTRY_ENABLED,
  composerTrailingControl,
} from "./voice/voiceSelection";
import { assistantVoiceFeatures } from "../../lib/api/assistantVoiceApi";
import { assistantUploadsApi } from "../../lib/api/assistantUploadsApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { safeAssistantLinkUrl } from "./assistantMarkdown";
import { retryInput } from "./assistantState";
import type { AssistantAcknowledgement } from "./assistantTypes";

type AssistantScreenProps = {
  contextual?: boolean;
  route?: {
    params?: AssistantParams;
  };
};

export function AssistantScreen({
  route,
  contextual = false,
}: AssistantScreenProps) {
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const focused = useIsFocused();
  const linkHost = useContext(AssistantLinkContext);
  const { colors, mode, bubblePalette } = useTheme();
  const bubble = chatBubbleColors(mode, bubblePalette);
  const chatColumn = useChatColumn();
  const transcriptEdges = useScrollEdges();
  const adoptedId = useRef<string | undefined>(undefined);
  const assistant = useAssistant(
    route?.params?.conversationId,
    route?.params?.agentId,
    (id) => {
      if (route?.params?.conversationId === id) return;
      adoptedId.current = id;
      navigation.setParams({
        conversationId: id,
        acknowledgementId: undefined,
        home: false,
        draft: false,
        draftId: undefined,
        openLatest: false,
      });
    },
    route?.params?.acknowledgementId,
  );
  const draftKey = assistantDraftKey(
    route?.params?.agentId,
    route?.params?.conversationId,
    route?.params?.draftId,
    (route?.params?.home === true && !route.params.draft) || !route?.params,
  );
  const draftOwner = useRef(draftKey);
  const [draft, setDraft] = useState(
    () => assistantDraftStore.get<string>(`text:${draftKey}`) ?? "",
  );
  const [composerGeneration, setComposerGeneration] = useState(0);
  const draftRevision = useRef(0);
  function updateDraft(value: string) {
    draftRevision.current += 1;
    assistantDraftStore.set(`text:${draftOwner.current}`, value);
    setDraft(value);
  }
  const [sheet, setSheet] = useState<"agents" | "permissions">();
  const [localError, setLocalError] = useState<string>();
  const [voiceThread, setVoiceThread] = useState<string>();
  const [voiceMinimized, setVoiceMinimized] = useState(false);
  function changeVoiceSize(minimized: boolean) {
    if (minimized === voiceMinimized) return;
    if (!reducedMotion)
      LayoutAnimation.configureNext(LayoutAnimation.Presets.easeInEaseOut);
    setVoiceMinimized(minimized);
  }
  const [voicePreferencesOpen, setVoicePreferencesOpen] = useState(false);
  const [voiceTranscript, setVoiceTranscript] = useState<{
    thread: string;
    epoch: number;
    snapshot: VoiceSnapshot;
  }>();
  const [openingVoice, setOpeningVoice] = useState(false);
  const voiceOpening = useRef<AbortController | undefined>(undefined);
  const composerPress = useRef(createAssistantComposerPressGuard());
  const [stopProtected, setStopProtected] = useState(false);
  const transcript = useRef<FlatList<VoiceTranscriptMessage>>(null);
  const transcriptViewport = useRef<View>(null);
  const transcriptOffset = useRef(0);
  useEffect(() => {
    transcriptOffset.current = 0;
  }, [assistant.conversationId]);
  const reducedMotion = useReducedMotion();
  const transcriptScroll = useTranscriptScrollFollow(
    transcript,
    assistant.conversationId,
    {
      reducedMotion,
      focused,
      loading: assistant.loading,
      historyError: !!assistant.error,
    },
  );
  const conversation = assistant.history?.conversation;
  const currentAgent = assistant.agent ?? conversation?.agent;
  const agentName = currentAgent
    ? chatAgentLabel(currentAgent)
    : assistant.agentId
      ? "Agent"
      : "NyxBot";
  // Specialists share this screen, so only NyxBot introduces itself as NyxBot.
  const isNyxBotChat = currentAgent
    ? currentAgent.kind === "nyxbot"
    : !assistant.agentId;
  const homeVisible = showNyxBotHome({
    home:
      (route?.params?.home === true && !route.params.draft) || !route?.params,
    contextual,
    nyxbot: isNyxBotChat,
    conversationId: assistant.conversationId,
    routeConversationId: route?.params?.conversationId,
    openLatest: route?.params?.openLatest,
    sending: assistant.sending,
    loading: assistant.loading,
    inputText: assistant.inputText,
    inputAttachments: assistant.inputAttachments,
  });
  const chatTitle = assistant.conversationId
    ? (conversation?.title ?? "Loading chat…")
    : homeVisible
      ? "Home"
      : "New chat";

  const pendingCount =
    (assistant.history?.acknowledgements.filter((a) => a.status === "pending")
      .length ?? 0) + (assistant.history?.approvals.length ?? 0);
  const headerStatus = pendingCount
    ? ("Needs you" as const)
    : conversation?.active_turn || assistant.sending
      ? ("Running" as const)
      : undefined;
  const headerDetail = chatThreadInfo(
    assistant.conversationId
      ? conversation?.title
      : homeVisible
        ? "Home"
        : "New chat",
    currentAgent?.name,
    assistant.conversationId ? conversation?.last_message_at : undefined,
  );
  const running = assistant.sending || !!conversation?.active_turn;
  const liveText = sanitizeAssistantMessageContent(
    assistant.live
      ? liveReplyText(assistant.live) ||
          conversation?.active_turn?.reply?.text ||
          ""
      : (conversation?.active_turn?.reply?.text ?? ""),
  );
  const liveAttachments = conversation?.active_turn?.attachments ?? [];
  const liveActivities = conversation?.active_turn?.activities ?? [];
  const destroyed = conversation?.agent?.destroyed;
  const streamPersisted =
    assistant.live?.turnId &&
    assistant.history?.messages.some(
      (message) =>
        message.turn_id === assistant.live?.turnId &&
        message.role === "assistant",
    );
  const optimisticUserPersisted =
    assistant.live?.turnId &&
    assistant.history?.messages.some(
      (message) =>
        message.turn_id === assistant.live?.turnId && message.role === "user",
    );
  const historyMessages = assistant.history?.messages;
  const voiceEpoch = getAuthEpoch();
  const voiceSurfaceToken = useMemo(() => ({}), [voiceThread, voiceEpoch]);
  const currentVoiceSurface = useRef(voiceSurfaceToken);
  currentVoiceSurface.current = voiceSurfaceToken;
  const voiceSnapshot =
    voiceTranscript &&
    voiceTranscript.thread === assistant.conversationId &&
    voiceTranscript.epoch === voiceEpoch
      ? voiceTranscript.snapshot
      : undefined;
  // While the voice composer is open it alone shows the user's unsent words;
  // once it closes, the chat keeps them as a partial transcript.
  const voiceComposerOpen =
    !!voiceThread && voiceThread === assistant.conversationId;
  const messages = useMemo(
    () =>
      mergeVoiceTranscript(
        historyMessages ?? [],
        voiceComposerOpen
          ? withoutLiveUserSpeech(voiceSnapshot)
          : voiceSnapshot,
      ),
    [historyMessages, voiceSnapshot, voiceComposerOpen],
  );
  const voiceHistory = useRef(historyMessages);
  voiceHistory.current = historyMessages;
  const receiveVoiceSnapshot = useCallback(
    (snapshot: VoiceSnapshot | undefined) => {
      // Unmounting the controls must not erase captions before history catches up.
      if (
        !snapshot ||
        !voiceThread ||
        getAuthEpoch() !== voiceEpoch ||
        currentVoiceSurface.current !== voiceSurfaceToken
      )
        return;
      if (voiceContext.current.conversationId !== voiceThread) return;
      setVoiceTranscript((previous) => ({
        thread: voiceThread,
        epoch: voiceEpoch,
        snapshot: retainVoiceSnapshot(
          previous &&
            previous.thread === voiceThread &&
            previous.epoch === voiceEpoch
            ? previous.snapshot
            : undefined,
          snapshot,
          voiceHistory.current ?? [],
        ),
      }));
    },
    [voiceThread, voiceEpoch, voiceSurfaceToken],
  );
  const stableTranscript =
    messages.length > 0 &&
    !running &&
    !assistant.inputText &&
    !assistant.inputAttachments.length;
  const hasHistoryGap = (historyMessages ?? []).some(
    (item, index) =>
      index > 0 &&
      item.seq > (historyMessages?.[index - 1]?.seq ?? item.seq) + 1,
  );
  const canCompose =
    !running &&
    !destroyed &&
    assistant.online &&
    !assistant.loading &&
    !assistant.loadingOlder &&
    !assistant.recoveryRequired &&
    !route?.params?.openLatest &&
    assistant.agent?.can_use !== false &&
    (!!assistant.conversationId ||
      (assistant.agent?.can_use === true &&
        assistant.agent.status !== "destroyed"));
  const uploads = useAssistantUploads({
    retainKey: draftKey,
    conversationId: assistant.conversationId,
    agentId: assistant.agentId,
    generation: composerGeneration,
    enabled: canCompose,
    online: assistant.online,
  });
  const guidance = useAssistantGuidance({
    scope: draftKey,
    conversationId: assistant.conversationId,
    turn: conversation?.active_turn,
    activeTurn: running,
    disabled:
      !assistant.online ||
      !!destroyed ||
      assistant.loading ||
      assistant.recoveryRequired ||
      !!route?.params?.openLatest ||
      !!sheet ||
      openingVoice ||
      !!voiceThread,
    onRefresh: assistant.recover,
  });
  const connection = useComposerConnection(
    draftKey,
    !canCompose ||
      uploads.session.sending ||
      uploads.picking ||
      openingVoice ||
      !!voiceThread,
  );
  const newChatDisabled =
    assistant.loading ||
    !assistant.agentId ||
    ((assistant.sending || uploads.session.sending) &&
      !assistant.conversationId);
  const permissionsLocked = running || uploads.session.sending;
  const headerDisabled =
    assistant.loading ||
    ((assistant.sending || uploads.session.sending) &&
      !assistant.conversationId);
  const voiceFeatures = useQuery({
    queryKey: ["workspace", "voice-features", getAuthEpoch()],
    queryFn: ({ signal }) => assistantVoiceFeatures(signal),
    enabled: assistant.online,
    retry: false,
    staleTime: 30000,
  });
  const dictation = useComposerDictation({
    scope: `${assistant.agentId}:${assistant.conversationId ?? "new"}:${composerGeneration}`,
    enabled:
      canCompose &&
      !uploads.picking &&
      !uploads.session.sending &&
      !openingVoice &&
      !voiceThread &&
      // Apple's recognizer only fills the composer: no NyxID voice service,
      // feature flag or thread eligibility is involved.
      !destroyed,
    onInsert: (text) => updateDraft(appendDictation(draft, text)),
  });
  const canSend =
    canCompose &&
    !connection.unavailable &&
    !dictation.active &&
    !uploads.session.sending &&
    !uploads.blocked &&
    !uploads.picking &&
    (!!draft.trim() || uploads.files.length > 0) &&
    [...draft].length <= 32768;

  const canGuideSend =
    guidance.canSteer &&
    !guidance.submitting &&
    !dictation.active &&
    !uploads.files.length &&
    !uploads.picking &&
    !!draft.trim() &&
    [...draft].length <= guidance.maxChars;
  const trailingControl = composerTrailingControl({
    enabled:
      NATIVE_VOICE_ENTRY_ENABLED &&
      voiceFeatures.data?.includes("assistant:voice") === true,
    running,
    conversationId: assistant.conversationId,
    text: draft,
    attachmentCount: uploads.files.length,
    uploadBlocked:
      !!connection.connection ||
      uploads.blocked ||
      uploads.picking ||
      uploads.session.sending,
    recording: dictation.active,
    channel: conversation?.channel,
    destroyed: !!destroyed,
    canCompose,
    canSend,
    voiceOpen: !!voiceThread && voiceThread === assistant.conversationId,
    openingVoice,
    stopDisabled:
      stopProtected ||
      assistant.mutating ||
      conversation?.active_turn?.stop_requested,
  });
  const voiceAvailable = trailingControl.kind === "voice";
  const voiceContext = useRef({
    agentId: assistant.agentId,
    conversationId: assistant.conversationId,
    epoch: getAuthEpoch(),
    revision: draftRevision.current,
    voiceAllowed: canCompose && voiceAvailable,
  });
  voiceContext.current = {
    agentId: assistant.agentId,
    conversationId: assistant.conversationId,
    epoch: getAuthEpoch(),
    revision: draftRevision.current,
    voiceAllowed: canCompose && voiceAvailable,
  };
  useEffect(() => {
    setOpeningVoice(false);
    return () => {
      voiceOpening.current?.abort();
    };
  }, [route?.params?.agentId, route?.params?.conversationId]);
  useEffect(() => {
    if (voiceThread && voiceThread !== assistant.conversationId)
      setVoiceThread(undefined);
  }, [voiceThread, assistant.conversationId]);
  useEffect(() => {
    setVoicePreferencesOpen(false);
    setVoiceTranscript((previous) =>
      previous &&
      previous.thread === assistant.conversationId &&
      previous.epoch === voiceEpoch
        ? previous
        : undefined,
    );
  }, [assistant.conversationId, voiceEpoch]);

  async function openVoice() {
    if (!canCompose || !voiceAvailable || voiceOpening.current) return;
    if (voiceThread === assistant.conversationId && voiceThread) return;
    Keyboard.dismiss();
    const scope = { ...voiceContext.current, revision: draftRevision.current };
    const controller = new AbortController();
    voiceOpening.current = controller;
    setOpeningVoice(true);
    const current = () =>
      !controller.signal.aborted &&
      voiceContext.current.agentId === scope.agentId &&
      voiceContext.current.conversationId === scope.conversationId &&
      voiceContext.current.voiceAllowed &&
      getAuthEpoch() === scope.epoch &&
      draftRevision.current === scope.revision;
    try {
      const id =
        scope.conversationId ??
        (await assistantUploadsApi.createDraft(
          scope.agentId,
          controller.signal,
        ));
      if (!current()) return;
      if (!scope.conversationId) {
        adoptedId.current = id;
        assistant.openConversation(id);
        navigation.setParams({
          conversationId: id,
          acknowledgementId: undefined,
          home: false,
          draft: false,
          draftId: undefined,
          openLatest: false,
        });
      }
      setVoiceMinimized(false);
      setVoiceThread(id);
    } catch (cause) {
      if (current())
        setLocalError(
          cause instanceof Error
            ? cause.message
            : "Voice could not open. Try again.",
        );
    } finally {
      if (voiceOpening.current === controller) {
        voiceOpening.current = undefined;
        if (!controller.signal.aborted) setOpeningVoice(false);
      }
    }
  }

  const openSafeLink = useCallback(
    (url: string): boolean => {
      const href = safeAssistantLinkUrl(url);
      if (!href) return false;
      if (linkHost?.(href)) return false;
      const automation = parseAutomationLink(href, getFrontendUrl());
      if (automation?.kind === "invalid") {
        Alert.alert("Could not open automation", automation.message);
      } else if (automation?.kind === "automation") {
        navigation.dispatch(
          workspaceNavigateAction("Automations", {
            agentId: undefined,
            ...automation.params,
          }),
        );
      } else {
        void WebBrowser.openBrowserAsync(href).catch(() => undefined);
      }
      return false;
    },
    [navigation, linkHost],
  );

  useEffect(() => {
    if (
      !route?.params?.openLatest ||
      route.params.conversationId ||
      !route.params.agentId ||
      assistant.indexedAgentId !== route.params.agentId
    )
      return;
    const id = assistant.conversations[0]?.id;
    navigation.setParams({
      conversationId: id,
      acknowledgementId: undefined,
      openLatest: false,
    });
    if (id) assistant.openConversation(id);
  }, [
    route?.params?.openLatest,
    route?.params?.conversationId,
    route?.params?.agentId,
    assistant.indexedAgentId,
    assistant.conversations,
    assistant.openConversation,
    navigation,
  ]);

  useLayoutEffect(() => {
    setLocalError(undefined);
  }, [assistant.conversationId]);
  useLayoutEffect(() => {
    // Persisting the first turn's identity is not a user switching chats.
    if (
      adoptedId.current &&
      route?.params?.conversationId === adoptedId.current
    ) {
      adoptedId.current = undefined;
      const value =
        assistantDraftStore.get<string>(`text:${draftOwner.current}`) ?? "";
      assistantDraftStore.delete(`text:${draftOwner.current}`);
      assistantDraftStore.set(`text:${draftKey}`, value);
      draftOwner.current = draftKey;
      return;
    }
    draftRevision.current += 1;
    draftOwner.current = draftKey;
    setDraft(assistantDraftStore.get<string>(`text:${draftKey}`) ?? "");
  }, [draftKey]);
  useEffect(() => {
    if (!stopProtected) return;
    const timeout = setTimeout(
      () => setStopProtected(false),
      ASSISTANT_SEND_PRESS_GRACE_MS,
    );
    return () => clearTimeout(timeout);
  }, [stopProtected]);

  async function perform(operation: () => Promise<unknown>) {
    setLocalError(undefined);
    try {
      await operation();
    } catch (cause) {
      setLocalError(
        cause instanceof Error
          ? cause.message
          : "Could not complete this action.",
      );
    }
  }

  function newChat() {
    setComposerGeneration((value) => value + 1);
    const agentId = assistant.agentId;
    assistant.openConversation();
    navigation.setParams(newAssistantChatParams(agentId));
    setSheet(undefined);
  }

  const pickerDestination = useRef<(() => void) | undefined>(undefined);
  const pickerScope = `${getAuthEpoch()}:${draftKey}`;
  useEffect(
    () => () => {
      pickerDestination.current = undefined;
    },
    [pickerScope],
  );
  function finishPickerDismissal() {
    const action = pickerDestination.current;
    pickerDestination.current = undefined;
    action?.();
  }
  useEffect(() => {
    // iOS reports native dismissal completion; Android Modal has no onDismiss.
    if (sheet || Platform.OS === "ios" || !pickerDestination.current) return;
    const timer = setTimeout(finishPickerDismissal, reducedMotion ? 0 : 350);
    return () => clearTimeout(timer);
  }, [sheet, reducedMotion]);
  function afterPickerDismissal(action: () => void) {
    if (pickerDestination.current) return;
    if (sheet !== "agents") {
      action();
      return;
    }
    const epoch = getAuthEpoch();
    pickerDestination.current = () => {
      if (getAuthEpoch() === epoch) action();
    };
    setSheet(undefined);
  }
  function openDetails() {
    afterPickerDismissal(() =>
      navigation.push("AssistantDetails", {
        agentId: currentAgent?.id ?? assistant.agentId,
        conversationId: assistant.conversationId,
      }),
    );
  }

  function openChatOptions() {
    if (!voiceThread || voiceThread !== assistant.conversationId)
      return openDetails();
    const current = () =>
      getAuthEpoch() === voiceEpoch &&
      currentVoiceSurface.current === voiceSurfaceToken &&
      voiceContext.current.conversationId === voiceThread;
    Alert.alert("Chat options", undefined, [
      {
        text: "Voice preferences",
        onPress: () => {
          if (current()) setVoicePreferencesOpen(true);
        },
      },
      {
        text: "Chat details",
        onPress: () => {
          if (current()) openDetails();
        },
      },
      { text: "Cancel", style: "cancel" },
    ]);
  }
  const headerHandlers = useRef({ newChat, openDetails: openChatOptions });
  headerHandlers.current = { newChat, openDetails: openChatOptions };
  const headerOptions = useMemo<NativeStackNavigationOptions>(() => {
    const actions = () => (
      <WorkspaceHeaderVisibility>
        <View style={styles.headerActions}>
          <IconButton
            label="New chat"
            disabled={newChatDisabled}
            onPress={() => headerHandlers.current.newChat()}
          >
            <SquarePen size={21} color={colors.textPrimary} />
          </IconButton>
          <IconButton
            label="Chat options"
            onPress={() => {
              headerHandlers.current.openDetails();
            }}
          >
            <MoreHorizontal size={23} color={colors.textPrimary} />
          </IconButton>
        </View>
      </WorkspaceHeaderVisibility>
    );
    return {
      header: ChatHeaderFrame,
      headerTitle: () => (
        <WorkspaceHeaderVisibility>
          <ChatIdentity
            name={agentName}
            detail={headerDetail}
            status={headerStatus}
            agent={
              currentAgent ?? {
                id: assistant.agentId ?? "nyxbot",
                name: agentName,
                kind: isNyxBotChat ? "nyxbot" : "specialist",
              }
            }
            disabled={headerDisabled}
            onPress={() => setSheet("agents")}
          />
        </WorkspaceHeaderVisibility>
      ),
      headerRight: actions,
      unstable_headerRightItems: () => [
        { type: "custom", element: actions(), hidesSharedBackground: true },
      ],
    };
  }, [
    agentName,
    headerDetail,
    headerStatus,
    assistant.agentId,
    isNyxBotChat,
    currentAgent,
    chatTitle,
    conversation?.title,
    colors.textPrimary,
    colors.textSecondary,
    headerDisabled,
    newChatDisabled,
  ]);

  useWorkspaceBack({
    // The primary Chats destination always keeps the drawer available. Only
    // contextual chats pushed from another screen have a native Back parent.
    active: false,
    defaultRole: contextual ? "back" : "menu",
    disabled: headerDisabled,
    options: headerOptions,
    onBack: () => navigation.goBack(),
  });

  async function sendDraft() {
    if (!(canSend || canGuideSend) || !composerPress.current("send")) return;
    setStopProtected(true);
    transcriptScroll.follow();
    await perform(() =>
      submitAssistantDraft({
        draft,
        updateDraft,
        getRevision: () => draftRevision.current,
        send: async (text) => {
          if (running) return guidance.submit(text, uploads.files.length);
          const accepted = await uploads.session.submit((payload) =>
            assistant.send(connection.prefix(text), undefined, payload),
          );
          if (accepted) connection.sent();
          return accepted;
        },
      }),
    );
  }

  // Activity ids are unique, so one set serves every turn's disclosure.
  const confirmationActivityIds = new Set(
    (assistant.history?.acknowledgements ?? []).flatMap((row) =>
      row.activity_id ? [row.activity_id] : [],
    ),
  );

  function renderMessage({
    item,
    index,
  }: {
    item: VoiceTranscriptMessage;
    index: number;
  }) {
    const isUser = item.role === "user";
    const voiceLabel = voiceTranscriptLabel(
      item,
      voiceThread ? voiceSnapshot : undefined,
    );
    const resetAt = Date.parse(conversation?.context_reset_at ?? "");
    const resetHere =
      Number.isFinite(resetAt) &&
      Date.parse(item.created_at) > resetAt &&
      (index === 0 || Date.parse(messages[index - 1]!.created_at) <= resetAt);
    if (item.continuation_of)
      return (
        <Label muted accessibilityLiveRegion="polite" style={styles.message}>
          Continuing after your decision
        </Label>
      );
    if (item.role === "event" || item.role === "orchestrator")
      return (
        <AssistantSystemMessage
          message={item}
          conversationId={assistant.conversationId}
          onOpenAutomations={() =>
            navigation.dispatch(workspaceNavigateAction("Automations"))
          }
        />
      );
    const input = retryInput(assistant.history, item);
    const attribution = [
      isUser
        ? ""
        : item.role === "assistant"
          ? (conversation?.agent?.display_name ??
            conversation?.agent?.name ??
            "NyxBot")
          : "Update",
      item.via,
    ]
      .filter(Boolean)
      .join(" · ");
    // Notes about the transcript itself, never inside anyone's bubble.
    const notes = (
      <>
        {index > 0 &&
        !item.voiceTransient &&
        !messages[index - 1]?.voiceTransient &&
        item.seq > (messages[index - 1]?.seq ?? item.seq) + 1 ? (
          <Notice message="Messages between these pages haven't loaded yet. Load missing messages to restore them." />
        ) : null}
        {resetHere ? (
          <Label muted accessibilityLabel="Conversation context reset">
            {contextResetCopy(conversation?.context_reset_reason)}
          </Label>
        ) : null}
      </>
    );
    const body = (
      <>
        {attribution ? (
          <Label
            muted
            style={[
              styles.messageLabel,
              isUser && { color: bubble.foreground },
            ]}
          >
            {attribution}
          </Label>
        ) : null}
        {isUser && item.steering ? (
          <Label style={{ color: bubble.foreground }}>
            Steering ·{" "}
            {steeringNotice(item.steering.code, item.steering.outcome)}
          </Label>
        ) : null}
        {isUser ? (
          item.text || voiceLabel ? (
            <Text
              selectable
              style={[styles.messageText, { color: bubble.foreground }]}
            >
              {item.text || "Transcribing…"}
            </Text>
          ) : null
        ) : (
          <AssistantReplyBody
            text={sanitizeAssistantMessageContent(item.text)}
            onLinkPress={openSafeLink}
            activity={
              item.activities.length ||
              (item.turn_metadata?.capacity_wait_ms ?? 0) > 0 ||
              (item.turn_metadata?.attempts ?? 0) > 1 ||
              item.turn_metadata?.channel_delivery ||
              assistant.history?.acknowledgements.some(
                (row) =>
                  row.status !== "pending" &&
                  !row.authored_skill &&
                  row.creation_turn_id === item.turn_id,
              ) ||
              assistant.history?.cma_operations?.some(
                (row) => row.turn_id === item.turn_id,
              ) ? (
                <>
                  <AssistantActivityDisclosure
                    key={item.id}
                    activities={item.activities}
                    activityTotal={item.activity_total}
                    turnLabel={
                      item.error_code === "cancelled"
                        ? "Stopped"
                        : item.status === "failed"
                          ? "Turn failed"
                          : "Completed"
                    }
                    turnFailed={
                      item.status === "failed" &&
                      item.error_code !== "cancelled"
                    }
                    metadata={item.turn_metadata}
                    confirmationActivityIds={confirmationActivityIds}
                    decisions={(
                      assistant.history?.acknowledgements ?? []
                    ).filter(
                      (row) =>
                        row.status !== "pending" &&
                        !row.authored_skill &&
                        row.creation_turn_id === item.turn_id,
                    )}
                    onToggle={() => {
                      // Opening details is an explicit reading action. Keep the header
                      // in view instead of following the newly expanded list to its end.
                      transcriptScroll.pause();
                    }}
                  />
                  {conversation?.id &&
                    item.activities
                      .filter((activity) => activity.machine)
                      .map((activity) => (
                        <MachineToolCard
                          key={`machine:${activity.id}`}
                          receipt={activity.machine!}
                          decision={activity.decision}
                          attachments={item.attachments}
                          conversationId={conversation.id}
                          onOpenDesktop={(params) =>
                            navigation.navigate("MachineDesktop", params)
                          }
                        />
                      ))}
                  {!isUser &&
                    assistant.history?.cma_operations
                      ?.filter(
                        (operation) => operation.turn_id === item.turn_id,
                      )
                      .map((operation) => (
                        <AssistantCmaOperation
                          key={operation.id}
                          operation={operation}
                        />
                      ))}
                </>
              ) : undefined
            }
          />
        )}
        {voiceLabel && item.text ? (
          <Label
            muted
            style={[
              styles.messageLabel,
              isUser && { color: bubble.foreground, opacity: 0.7 },
            ]}
          >
            {voiceLabel}
          </Label>
        ) : null}
        {item.attachments
          .filter(
            (attachment) =>
              !item.activities.some(
                (a) => a.machine?.screenshot_id === attachment.id,
              ),
          )
          .map((attachment) => (
            <AssistantAttachmentView
              key={attachment.id}
              attachment={attachment}
              conversationId={assistant.conversationId}
              captionColor={isUser ? bubble.foreground : undefined}
            />
          ))}
        {!isUser ? (
          <AssistantMessageNotices
            message={item}
            onOpenBilling={() =>
              navigation.dispatch(openWorkspaceSheetAction("UsageCredits"))
            }
          />
        ) : null}
        {input !== undefined ||
        (item.status === "failed" && item.error_code !== "cancelled") ? (
          <View style={styles.turnActions}>
            {input !== undefined ? (
              <QuietButton
                label="Retry"
                icon={<RotateCcw size={14} color={colors.textPrimary} />}
                disabled={
                  running ||
                  !assistant.online ||
                  assistant.recoveryRequired ||
                  assistant.loadingOlder
                }
                onPress={() => void perform(() => assistant.retry(item.id))}
              />
            ) : null}
            {item.status === "failed" && item.error_code !== "cancelled" ? (
              <QuietButton
                label="Check retry availability"
                disabled={
                  running || !assistant.online || assistant.loadingOlder
                }
                onPress={() =>
                  void perform(() => assistant.checkRecovery(item.id))
                }
              />
            ) : null}
          </View>
        ) : null}
      </>
    );
    // Your own messages: the bubble holds only what you sent; the time and
    // copy control sit under it in the page's quiet colour.
    const footer =
      !item.voiceTransient && !voiceLabel ? (
        <AssistantMessageFooter
          key={`${assistant.conversationId}:${item.id}`}
          text={isUser ? item.text : sanitizeAssistantMessageContent(item.text)}
          timestamp={item.created_at}
          user={isUser}
          report={
            item.role === "assistant" &&
            item.status === "completed" &&
            assistant.conversationId
              ? {
                  kind: "conversation",
                  id: assistant.conversationId,
                  messageId: item.id,
                  reported: isReported(item.feedback),
                }
              : undefined
          }
        />
      ) : null;
    return (
      <ChatMessageTurn user={isUser} notes={notes} footer={footer}>
        {body}
      </ChatMessageTurn>
    );
  }

  function renderAuthoredReview(row: AssistantAcknowledgement) {
    return (
      <NativeAuthoredSkillReview
        acknowledgement={row}
        initiallyOpen={row.id === route?.params?.acknowledgementId}
        conversationId={assistant.conversationId!}
      />
    );
  }

  function renderAcknowledgement(row: AssistantAcknowledgement) {
    if (row.status !== "pending") return null;
    return (
      <AssistantAcknowledgementCard
        key={row.id}
        row={row}
        selected={row.id === route?.params?.acknowledgementId}
        disabled={
          !!assistant.deciding || !assistant.online || assistant.loadingOlder
        }
        deciding={assistant.deciding === row.id}
        onDecision={(choice) =>
          void perform(() => assistant.decide(row.id, choice))
        }
        authoredReview={
          row.authored_skill ? renderAuthoredReview(row) : undefined
        }
      />
    );
  }

  const revealConfirmation: RevealReviewTarget = (target, heading, current) => {
    const viewport = transcriptViewport.current;
    if (!viewport) return;
    revealAssistantReviewTarget({
      viewport,
      target,
      current,
      offset: () => transcriptOffset.current,
      pause: transcriptScroll.pause,
      scroll: (offset) =>
        transcript.current?.scrollToOffset({ offset, animated: false }),
      focus: () => {
        const handle = findNodeHandle(heading);
        if (handle) AccessibilityInfo.setAccessibilityFocus(handle);
      },
    });
  };

  return (
    <WorkspacePage
      title={chatTitle}
      showHeading={false}
      scroll={false}
      contentStyle={styles.chatPage}
    >
      <AssistantConnectionBanner
        inset
        onRetry={() => void assistant.recover()}
      />
      {assistant.agentsError ? (
        <View style={styles.errorRow}>
          <Notice message={assistant.agentsError} tone="error" />
          <ActionButton
            label="Reload agents"
            variant="secondary"
            onPress={() => void assistant.reloadAgents()}
          />
        </View>
      ) : null}
      {assistant.error || localError ? (
        <View style={styles.errorRow}>
          <AssistantErrorNotice message={localError ?? assistant.error} />
          <ActionButton
            label="Refresh history"
            variant="secondary"
            onPress={() => void assistant.recover()}
          />
        </View>
      ) : null}
      {destroyed ? (
        <View style={styles.errorRow}>
          <Notice message="This agent was destroyed. This chat is read-only." />
        </View>
      ) : null}
      {route?.params?.openLatest ? (
        <View style={styles.errorRow}>
          <Notice message={`Opening ${agentName}'s latest chat…`} />
        </View>
      ) : null}
      <TranscriptViewport
        key="transcript-column"
        ready={homeVisible || transcriptScroll.ready}
        onTouchStart={() => {
          if (voiceThread) changeVoiceSize(true);
        }}
        viewportRef={transcriptViewport}
      >
        {homeVisible ? (
          <NyxBotHome
            key={getAuthEpoch()}
            scrollProps={{
              style: styles.fill,
              contentContainerStyle: transcriptColumnStyle(chatColumn.maxWidth),
              scrollEventThrottle: 32,
              onLayout: (event) =>
                transcriptEdges.layout(event.nativeEvent.layout.height),
              onContentSizeChange: (_width, height) =>
                transcriptEdges.content(height),
              onScroll: (event) =>
                transcriptEdges.scroll(event.nativeEvent.contentOffset.y),
            }}
            onChat={(agentId, conversationId, acknowledgementId) => {
              if (newChatDisabled || running || uploads.session.sending) return;
              const agent = assistant.agents.find((row) => row.id === agentId);
              if (
                !agent ||
                agent.can_use === false ||
                agent.status === "destroyed"
              )
                return;
              setComposerGeneration((value) => value + 1);
              assistant.openConversation(conversationId, acknowledgementId);
              navigation.setParams({
                ...(conversationId
                  ? { home: false, draftId: undefined }
                  : newAssistantChatParams(agentId)),
                agentId,
                conversationId,
                acknowledgementId,
                draft: false,
                openLatest: !conversationId,
              });
            }}
            onStartNextStep={(step) => {
              if (newChatDisabled || running || uploads.session.sending) {
                setLocalError(
                  "NyxBot is busy. Start this step once it finishes.",
                );
                return;
              }
              void perform(() => assistant.send(step.prompt));
            }}
            onOpenCrew={(groupId) =>
              navigation.navigate("GroupChat", { groupId })
            }
            onNewAgent={() =>
              navigation.dispatch(openWorkspacePageAction("AgentCreate"))
            }
            onNewCrew={() =>
              navigation.dispatch(openWorkspacePageAction("GroupEditor"))
            }
          />
        ) : (
          <FlatList
            key={assistant.conversationId ?? "new-chat"}
            ref={transcript}
            style={styles.fill}
            data={messages}
            keyExtractor={(item) => item.id}
            renderItem={renderMessage}
            ItemSeparatorComponent={ChatTranscriptSeparator}
            keyboardShouldPersistTaps="handled"
            keyboardDismissMode="on-drag"
            contentContainerStyle={[
              styles.transcript,
              transcriptColumnStyle(chatColumn.maxWidth),
            ]}
            // Anchor stable history only while reading it. Native row anchoring
            // competes with keeping the newest live or newly saved reply visible.
            maintainVisibleContentPosition={
              stableTranscript && !transcriptScroll.following
                ? { minIndexForVisible: 0 }
                : undefined
            }
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
              transcriptOffset.current = event.nativeEvent.contentOffset.y;
              transcriptScroll.listProps.onScroll(event);
              transcriptEdges.scroll(event.nativeEvent.contentOffset.y);
            }}
            ListHeaderComponent={
              assistant.history?.before_seq ? (
                <ActionButton
                  label={
                    hasHistoryGap
                      ? "Load missing messages"
                      : "Load earlier messages"
                  }
                  variant="secondary"
                  loading={assistant.loadingOlder}
                  // Page a stable transcript so dynamic footer changes cannot
                  // interfere with the native reading anchor.
                  disabled={
                    running ||
                    assistant.loading ||
                    !assistant.online ||
                    !!assistant.deciding ||
                    assistant.mutating
                  }
                  onPress={() => {
                    transcriptScroll.pause();
                    void assistant.loadOlder();
                  }}
                />
              ) : null
            }
            ListHeaderComponentStyle={styles.transcriptHeader}
            ListEmptyComponent={
              assistant.loading ? (
                <SkeletonRows
                  rows={4}
                  icon={false}
                  accessibilityLabel="Loading messages"
                />
              ) : !assistant.sending && !voiceThread ? (
                <View style={styles.welcome}>
                  <Text
                    style={[styles.welcomeTitle, { color: colors.textPrimary }]}
                  >
                    {isNyxBotChat ? "Hi, I'm NyxBot." : "How can I help?"}
                  </Text>
                  {isNyxBotChat ? (
                    <Text
                      style={[
                        styles.welcomeBody,
                        { color: colors.textSecondary },
                      ]}
                    >
                      I'm here to help you use NyxID, manage your NyxID services
                      and automate them.
                    </Text>
                  ) : null}
                </View>
              ) : null
            }
            ListFooterComponent={
              <View style={styles.footer}>
                {conversation?.context_reset_at &&
                (!messages.length ||
                  Date.parse(messages.at(-1)!.created_at) <=
                    Date.parse(conversation.context_reset_at)) ? (
                  <Label muted accessibilityLabel="Conversation context reset">
                    {contextResetCopy(conversation.context_reset_reason)}
                  </Label>
                ) : null}
                {(assistant.inputText ||
                  assistant.inputAttachments.length > 0) &&
                !optimisticUserPersisted ? (
                  <View
                    style={[
                      styles.message,
                      styles.userRow,
                      styles.userMessage,
                      {
                        backgroundColor: bubble.background,
                      },
                    ]}
                  >
                    <Label
                      style={[
                        styles.messageLabel,
                        { color: bubble.foreground },
                      ]}
                    >
                      Sending…
                    </Label>
                    {assistant.inputText ? (
                      <Text
                        selectable
                        style={[
                          styles.messageText,
                          { color: bubble.foreground },
                        ]}
                      >
                        {assistant.inputText}
                      </Text>
                    ) : null}
                    {assistant.inputAttachments.map((attachment) => (
                      <AssistantAttachmentView
                        key={attachment.id}
                        attachment={attachment}
                        captionColor={bubble.foreground}
                        conversationId={
                          assistant.conversationId ??
                          assistant.live?.conversationId
                        }
                      />
                    ))}
                  </View>
                ) : null}
                {running && !streamPersisted ? (
                  <View style={styles.message}>
                    <Label muted style={styles.messageLabel}>
                      {agentName}
                    </Label>
                    <AssistantReplyBody
                      text={liveText}
                      onLinkPress={openSafeLink}
                      activity={
                        liveActivities.length ? (
                          <>
                            <AssistantActivityDisclosure
                              key={
                                conversation?.active_turn?.turn_id ??
                                assistant.live?.turnId ??
                                "pending-turn"
                              }
                              activities={liveActivities}
                              activityTotal={
                                conversation?.active_turn?.activity_total
                              }
                              turnLabel="Working…"
                              confirmationActivityIds={confirmationActivityIds}
                              onToggle={() => {
                                transcriptScroll.pause();
                              }}
                            />
                            {conversation?.id &&
                              liveActivities
                                .filter((activity) => activity.machine)
                                .map((activity) => (
                                  <MachineToolCard
                                    key={`machine:${activity.id}`}
                                    receipt={activity.machine!}
                          decision={activity.decision}
                                    attachments={liveAttachments}
                                    conversationId={conversation.id}
                                    onOpenDesktop={(params) =>
                                      navigation.navigate(
                                        "MachineDesktop",
                                        params,
                                      )
                                    }
                                  />
                                ))}
                          </>
                        ) : undefined
                      }
                    />
                    {liveAttachments
                      .filter(
                        (a) =>
                          !liveActivities.some(
                            (tool) => tool.machine?.screenshot_id === a.id,
                          ),
                      )
                      .map((a) => (
                        <AssistantAttachmentView
                          key={a.id}
                          attachment={a}
                          conversationId={
                            assistant.conversationId ??
                            assistant.live?.conversationId
                          }
                        />
                      ))}
                    <AssistantTurnStatus
                      turn={conversation?.active_turn}
                      live={assistant.live}
                      pendingApproval={
                        !!assistant.history?.approvals.length ||
                        !!assistant.history?.acknowledgements.some(
                          (row) => row.status === "pending",
                        )
                      }
                      interrupted={assistant.interrupted}
                    />
                    {assistant.history?.cma_operations
                      ?.filter(
                        (operation) =>
                          operation.turn_id ===
                          (conversation?.active_turn?.turn_id ??
                            assistant.live?.turnId),
                      )
                      .map((operation) => (
                        <AssistantCmaOperation
                          key={operation.id}
                          operation={operation}
                        />
                      ))}
                  </View>
                ) : null}
                <AssistantWaitingNote
                  items={assistant.history?.waiting ?? []}
                  agentName={agentName}
                  pendingEvents={conversation?.pending_events}
                />
                <AssistantAcknowledgements
                  key={conversation?.id ?? "draft"}
                  acknowledgements={(
                    assistant.history?.acknowledgements ?? []
                  ).filter(
                    (row) =>
                      row.status === "pending" ||
                      row.authored_skill ||
                      row.id === route?.params?.acknowledgementId ||
                      !messages.some(
                        (message) =>
                          message.role === "assistant" &&
                          message.turn_id === row.creation_turn_id,
                      ),
                  )}
                  selectedId={route?.params?.acknowledgementId}
                  scope={`${getAuthEpoch()}:${conversation?.id ?? "draft"}`}
                  active={assistant.active && !sheet}
                  onReveal={revealConfirmation}
                  onOpenLink={openSafeLink}
                  renderPending={renderAcknowledgement}
                  renderAuthoredReview={renderAuthoredReview}
                  onToggle={() => transcriptScroll.pause()}
                />
                {assistant.history?.approvals.map((approval) => (
                  <AssistantApprovalCard
                    key={approval.id}
                    approval={approval}
                    disabled={
                      !assistant.active || !!assistant.deciding || !!destroyed
                    }
                    onDecide={(approved) =>
                      assistant.decideApproval(approval.id, approved)
                    }
                  />
                ))}
              </View>
            }
          />
        )}
        <JumpToLatest
          visible={!homeVisible && transcriptScroll.showJump}
          onPress={transcriptScroll.follow}
        />
        <ScrollEdgeFades edges={transcriptEdges.edges} color={colors.bg} />
      </TranscriptViewport>
      <ChatComposerDock>
        {/* Voice mode slides up in place of the composer; the chat above stays
            on screen, so its cards and links work during the call. */}
        {voiceThread && voiceThread === assistant.conversationId ? (
          <AssistantVoiceConversation
            key={`voice:${voiceThread}`}
            thread={voiceThread}
            agentName={agentName}
            minimized={voiceMinimized}
            onExpand={() => changeVoiceSize(false)}
            onClose={() => {
              if (
                getAuthEpoch() !== voiceEpoch ||
                currentVoiceSurface.current !== voiceSurfaceToken
              )
                return;
              setVoiceThread(undefined);
              setVoicePreferencesOpen(false);
            }}
            onSnapshot={receiveVoiceSnapshot}
            preferencesOpen={voicePreferencesOpen}
            onPreferencesClose={() => setVoicePreferencesOpen(false)}
            onUpdate={() => void assistant.recover()}
            onReview={(acknowledgementId) => {
              assistant.openConversation(voiceThread, acknowledgementId);
              navigation.setParams({
                conversationId: voiceThread,
                acknowledgementId,
                openLatest: false,
              });
            }}
          />
        ) : (
          <ComposerDictationFrame dictation={dictation}>
            {guidance.canSteer ? (
              <Label muted style={styles.composerHint}>
                Still replying. What you send now steers this reply; Stop ends
                it.
              </Label>
            ) : null}
            {guidance.notice ? (
              <View style={{ gap: 4, paddingHorizontal: 8 }}>
                <Label
                  muted
                  accessibilityLiveRegion="polite"
                  style={{ fontSize: 12 }}
                >
                  {steeringNotice(
                    guidance.notice.code,
                    guidance.notice.outcome,
                  )}
                </Label>
                {guidance.notice.code === "no_active_turn" &&
                guidance.notice.text ? (
                  <ActionButton
                    label="Send as new message"
                    compact
                    variant="secondary"
                    disabled={running || guidance.submitting || !canCompose}
                    onPress={() =>
                      void guidance.sendAsMessage((text) =>
                        assistant.send(text),
                      )
                    }
                  />
                ) : null}
              </View>
            ) : null}
            <ComposerConnectionPicker state={connection} />
            <AssistantPendingUploads
              files={uploads.files}
              remove={uploads.remove}
              disabled={running || uploads.session.sending}
              showMentionHandles={false}
            />
            {uploads.error || uploads.notice ? (
              <Label
                style={{
                  color: uploads.error ? colors.danger : colors.textSecondary,
                }}
              >
                {uploads.error ?? uploads.notice}
              </Label>
            ) : null}
            <ChatComposerInput
              dictation={dictation}
              value={draft}
              onChangeText={updateDraft}
              generation={`${draftKey}:${composerGeneration}`}
              editable={
                !destroyed &&
                !assistant.recoveryRequired &&
                !route?.params?.openLatest &&
                (!running || guidance.canSteer)
              }
              accessibilityLabel="Message to assistant"
              placeholder={
                guidance.canSteer
                  ? "Guide this running reply…"
                  : `Message ${agentName}`
              }
              renderActions={(mentionAction) => (
                <ChatComposerToolbar
                  dictation={dictation}
                  dictationDisabled={
                    !canCompose ||
                    uploads.picking ||
                    uploads.session.sending ||
                    openingVoice ||
                    !!voiceThread
                  }
                  attach={{
                    label: "Attach photos or files, or take a photo",
                    busy: uploads.picking,
                    disabled:
                      !canCompose ||
                      dictation.active ||
                      uploads.picking ||
                      uploads.session.sending ||
                      uploads.files.length >= 10,
                    onPress: () =>
                      uploads.choose([
                        {
                          label: "Use a connection",
                          onPress: connection.choose,
                        },
                      ]),
                  }}
                  mention={mentionAction}
                  settings={
                    <AssistantComposerSettings
                      scope={`${getAuthEpoch()}:${draftKey}`}
                      agentOverride={assistant.agent?.access_mode_override}
                      activeTurn={permissionsLocked}
                      disabled={
                        !assistant.online ||
                        !assistant.agent ||
                        !!destroyed ||
                        assistant.loading ||
                        assistant.recoveryRequired ||
                        uploads.picking ||
                        openingVoice ||
                        !!voiceThread
                      }
                      visible={sheet === "permissions"}
                      onOpenChange={(open) =>
                        setSheet(open ? "permissions" : undefined)
                      }
                    />
                  }
                >
                  {guidance.canSteer && draft.trim() ? (
                    <ActionButton
                      label="Steer"
                      accessibilityLabel="Send guidance"
                      compact
                      disabled={!canGuideSend}
                      loading={guidance.submitting}
                      onPress={() => void sendDraft()}
                    />
                  ) : null}
                  <ComposerSubmitButton
                    {...trailingControl}
                    label={
                      trailingControl.kind === "stop"
                        ? "Stop assistant"
                        : trailingControl.kind === "voice"
                          ? "Open voice call"
                          : "Send message"
                    }
                    onPress={() => {
                      if (trailingControl.kind === "stop") {
                        if (composerPress.current("stop")) void assistant.stop();
                      } else if (trailingControl.kind === "voice") {
                        void openVoice();
                      } else {
                        void sendDraft();
                      }
                    }}
                  />
                </ChatComposerToolbar>
              )}
            />
            {assistant.recoveryRequired || uploads.blocked ? (
              <Label muted style={styles.composerHint}>
                {assistant.recoveryRequired
                  ? "Open the saved chat before continuing."
                  : "Wait for uploads or remove failed files."}
              </Label>
            ) : null}
            {[...draft].length >
            (guidance.canSteer ? guidance.maxChars : 32768) ? (
              <Label style={{ color: colors.danger }}>
                {guidance.canSteer
                  ? `Guidance must be text only and at most ${guidance.maxChars.toLocaleString()} characters.`
                  : "Keep the message under 32,768 characters."}
              </Label>
            ) : null}
          </ComposerDictationFrame>
        )}
      </ChatComposerDock>

      <Sheet
        visible={!!sheet && sheet !== "permissions"}
        scroll={sheet !== "agents"}
        title="Choose agent"
        onDismiss={finishPickerDismissal}
        onClose={() => setSheet(undefined)}
      >
        {sheet === "agents" ? (
          <WorkspaceChatList
            key={getAuthEpoch()}
            mode="picker"
            visible
            current={{
              kind: "agent",
              agentId: currentAgent?.id ?? assistant.agentId,
              conversationId: assistant.conversationId,
            }}
            onCreateAgent={() => {}}
            onCreateGroup={() => {}}
            scrollProps={{
              contentContainerStyle: styles.sheetContent,
              keyboardShouldPersistTaps: "handled",
            }}
            scrollHeader={
              <Card>
                <Label style={styles.sheetSection}>This thread</Label>
                <Label>{conversation?.title || chatTitle}</Label>
                <Label muted>{agentName}</Label>
                {conversation ? (
                  <Label muted>
                    {formatDate(conversation.last_message_at)} ·{" "}
                    {conversation.message_count} messages
                  </Label>
                ) : null}
                <View style={styles.row}>
                  <ActionButton
                    compact
                    label="Details"
                    variant="secondary"
                    onPress={() => {
                      openDetails();
                    }}
                  />
                  <ActionButton
                    compact
                    label="History"
                    variant="secondary"
                    onPress={() => {
                      afterPickerDismissal(() =>
                        navigation.push("AssistantHistory", {
                          agentId: currentAgent?.id ?? assistant.agentId,
                        }),
                      );
                    }}
                  />
                </View>
              </Card>
            }
            scrollFooter={
              <ActionButton
                label="Manage agents"
                variant="secondary"
                onPress={() => {
                  afterPickerDismissal(() =>
                    navigation.dispatch(workspaceNavigateAction("Agents")),
                  );
                }}
              />
            }
            onSelect={(target) => {
              if (target.kind !== "agent" || newChatDisabled) return;
              const agent = assistant.agents.find(
                (row) => row.id === target.agentId,
              );
              if (
                !agent ||
                agent.can_use !== true ||
                agent.status === "destroyed"
              )
                return;
              const sameThread =
                !!target.conversationId &&
                target.conversationId === assistant.conversationId &&
                (target.agentId === currentAgent?.id ||
                  target.agentId === assistant.agentId);
              if (!sameThread) {
                setComposerGeneration((value) => value + 1);
                assistant.openConversation(target.conversationId);
                navigation.setParams({
                  ...(target.conversationId
                    ? { home: false, draftId: undefined }
                    : newAssistantChatParams(agent.id)),
                  agentId: agent.id,
                  conversationId: target.conversationId,
                  acknowledgementId: undefined,
                  draft: !target.conversationId,
                  openLatest: false,
                });
              }
              setSheet(undefined);
            }}
          />
        ) : null}
      </Sheet>
    </WorkspacePage>
  );
}

function Sheet({
  visible,
  title,
  onClose,
  children,
  scroll = true,
  onDismiss,
}: {
  scroll?: boolean;
  onDismiss?: () => void;
  visible: boolean;
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  return (
    <Modal
      visible={visible}
      animationType={reducedMotion ? "none" : "slide"}
      presentationStyle="pageSheet"
      onRequestClose={onClose}
      onDismiss={onDismiss}
    >
      <View style={[styles.sheet, { backgroundColor: colors.bg }]}>
        <View style={styles.sheetHeader}>
          <Text
            accessibilityRole="header"
            style={[styles.sheetTitle, { color: colors.textPrimary }]}
          >
            {title}
          </Text>
          <IconButton label={`Close ${title.toLowerCase()}`} onPress={onClose}>
            <X size={22} color={colors.textSecondary} />
          </IconButton>
        </View>
        {scroll ? (
          <ScrollView
            keyboardShouldPersistTaps="handled"
            contentContainerStyle={styles.sheetContent}
          >
            {children}
          </ScrollView>
        ) : (
          <View style={styles.fill}>{children}</View>
        )}
      </View>
    </Modal>
  );
}

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}
const styles = {
  ...chatSurfaceStyles,
  ...StyleSheet.create({
    headerActions: { flexDirection: "row", alignItems: "center" },
    turnActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
    metadata: { fontSize: 11 },
    runningActivity: { flexShrink: 1, fontSize: 11 },
    welcome: { alignItems: "center", paddingVertical: 100 },
    welcomeTitle: {
      fontFamily: fonts.displaySemi,
      fontSize: 24,
      lineHeight: 32,
      textAlign: "center",
    },
    welcomeBody: {
      fontFamily: fonts.bodyRegular,
      fontSize: 15,
      lineHeight: 22,
      textAlign: "center",
      maxWidth: 300,
      marginTop: 10,
    },
    thinking: { alignSelf: "flex-start", marginVertical: 10 },
    sheet: { flex: 1, paddingTop: 20 },
    sheetHeader: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "space-between",
      paddingHorizontal: 20,
      gap: 16,
    },
    sheetSection: { fontFamily: fonts.bodySemi, fontSize: 13 },
    sheetTitle: { fontFamily: fonts.displaySemi, fontSize: 22, lineHeight: 30 },
    sheetContent: { padding: 20, paddingBottom: 40, gap: 14 },
  }),
};
