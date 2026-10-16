import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  FlatList,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { useNavigation } from "@react-navigation/native";
import * as WebBrowser from "expo-web-browser";
import Markdown, {
  MarkdownIt,
  type RenderRules,
} from "react-native-markdown-display";
import {
  History,
  Bot,
  ChevronDown,
  MessageSquare,
  Check,
  Info,
  Paperclip,
  SquarePen,
  Send,
  Square,
  X,
} from "lucide-react-native";
import type {
  AssistantParams,
  WorkspaceNavigationProp,
} from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import {
  WorkspacePage,
  ActionButton,
  Card,
  Field,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius } from "../../theme/designTokens";
import { useAssistant } from "./useAssistant";
import { chatAgentLabel } from "./assistantAgentContext";
import { liveReplyText } from "./assistantStream";
import {
  ASSISTANT_SEND_PRESS_GRACE_MS,
  createAssistantComposerPressGuard,
} from "./assistantComposerPress";
import { submitAssistantDraft } from "./assistantComposerDraft";
import { AssistantActivityDisclosure } from "./AssistantActivityDisclosure";
import { AssistantCopyButton } from "./AssistantCopyButton";
import { assistantSelectableMarkdown } from "./assistantSelectableMarkdown";
import { AssistantAttachmentView } from "./AssistantAttachmentView";
import { AssistantPendingUploads } from "./AssistantPendingUploads";
import { useAssistantUploads } from "./useAssistantUploads";
import { describeToolActivity } from "./assistantActivityText";
import { NativeAuthoredSkillReview } from "../plugins/NativeAuthoredSkillReview";
import { parseAutomationLink } from "../automations/automationLinks";
import { NYXID_FRONTEND_URL } from "../../lib/env";
import { transcriptEndOffset } from "./assistantTranscriptScroll";
import {
  assistantMarkdownOptions,
  safeAssistantLinkUrl,
} from "./assistantMarkdown";
import {
  acknowledgementTitle,
  retryInput,
  storedAssistantError,
} from "./assistantState";
import type {
  AssistantAcknowledgement,
  AssistantMessage,
} from "./assistantTypes";

type AssistantScreenProps = {
  contextual?: boolean;
  route?: {
    params?: AssistantParams;
  };
};

// Both persisted and streamed replies share Web's automatic URL recognition.
const assistantMarkdown = MarkdownIt(assistantMarkdownOptions);
assistantMarkdown.validateLink = (url) => safeAssistantLinkUrl(url) !== null;

export function AssistantScreen({
  route,
  contextual = false,
}: AssistantScreenProps) {
  const navigation = useNavigation<WorkspaceNavigationProp>();
  const { colors } = useTheme();
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
        openLatest: false,
      });
    },
    route?.params?.acknowledgementId,
  );
  const [draft, setDraft] = useState("");
  const [composerGeneration, setComposerGeneration] = useState(0);
  const draftRevision = useRef(0);
  function updateDraft(value: string) {
    draftRevision.current += 1;
    setDraft(value);
  }
  const [historyOpen, setHistoryOpen] = useState(false);
  const [agentsOpen, setAgentsOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [localError, setLocalError] = useState<string>();
  const composerPress = useRef(createAssistantComposerPressGuard());
  const [stopProtected, setStopProtected] = useState(false);
  const transcript = useRef<FlatList<AssistantMessage>>(null);
  const following = useRef(true);
  const transcriptMetrics = useRef({ height: 0, viewportHeight: 0 });
  const conversation = assistant.history?.conversation;
  const currentAgent = assistant.agent ?? conversation?.agent;
  const agentName = currentAgent
    ? chatAgentLabel(currentAgent)
    : assistant.agentId
      ? "Agent"
      : "NyxBot";
  const chatTitle = assistant.conversationId
    ? (conversation?.title ?? "Loading chat…")
    : "New chat";
  const currentModelId = conversation?.model ?? assistant.model;
  const currentModelLabel =
    assistant.models.find((item) => item.id === currentModelId)?.label ??
    currentModelId.replace(/^nyxagent\//, "");
  const running = assistant.sending || !!conversation?.active_turn;
  const liveText = assistant.live ? liveReplyText(assistant.live) : "";
  const liveActivities = conversation?.active_turn?.activities ?? [];
  const runningActivity = liveActivities
    .filter((activity) => activity.status === "running")
    .at(-1);
  const waitingActivity = runningActivity ?? liveActivities.at(-1);
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
  const messages = assistant.history?.messages ?? [];
  const stableTranscript =
    messages.length > 0 &&
    !running &&
    !assistant.inputText &&
    !assistant.inputAttachments.length;
  const hasHistoryGap = messages.some(
    (item, index) =>
      index > 0 && item.seq > (messages[index - 1]?.seq ?? item.seq) + 1,
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
    conversationId: assistant.conversationId,
    agentId: assistant.agentId,
    model: assistant.model,
    generation: composerGeneration,
    enabled: canCompose,
    online: assistant.online,
  });
  const newChatDisabled =
    assistant.loading ||
    !assistant.agentId ||
    ((assistant.sending || uploads.session.sending) &&
      !assistant.conversationId);
  const modelLocked =
    !!assistant.conversationId || assistant.sending || uploads.session.sending;
  const canSend =
    canCompose &&
    !uploads.session.sending &&
    !uploads.blocked &&
    !uploads.picking &&
    (!!draft.trim() || uploads.files.length > 0) &&
    [...draft].length <= 32768;

  function openSafeLink(url: string): boolean {
    const href = safeAssistantLinkUrl(url);
    if (!href) return false;
    const automation = parseAutomationLink(href, NYXID_FRONTEND_URL);
    if (automation?.kind === "invalid") {
      Alert.alert("Could not open automation", automation.message);
    } else if (automation?.kind === "automation") {
      if (automation.params.setup) {
        navigation.push("AutomationEditor", automation.params);
      } else if (contextual) {
        navigation.popTo(
          "Workspace",
          {
            screen: "Automations",
            params: {
              agentId: undefined,
              setup: undefined,
              ...automation.params,
            },
          },
          { merge: true },
        );
      } else {
        navigation.navigate({
          name: "Automations",
          params: {
            agentId: undefined,
            setup: undefined,
            ...automation.params,
          },
          merge: true,
        });
      }
    } else {
      void WebBrowser.openBrowserAsync(href).catch(() => undefined);
    }
    return false;
  }

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
    following.current = true;
    transcriptMetrics.current = { height: 0, viewportHeight: 0 };
    setLocalError(undefined);
  }, [assistant.conversationId]);
  useEffect(() => {
    // Persisting the first turn's identity is not a user switching chats.
    if (
      adoptedId.current &&
      route?.params?.conversationId === adoptedId.current
    ) {
      adoptedId.current = undefined;
      return;
    }
    draftRevision.current += 1;
    setDraft("");
  }, [route?.params?.agentId, route?.params?.conversationId]);
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
    assistant.setModel(assistant.agent?.model ?? "nyxagent/chat");
    navigation.setParams({
      conversationId: undefined,
      acknowledgementId: undefined,
      agentId,
      draft: true,
      openLatest: false,
    });
    updateDraft("");
    setHistoryOpen(false);
  }

  useWorkspaceBack({
    active:
      !contextual &&
      Boolean(assistant.conversationId || currentAgent?.kind === "specialist"),
    defaultRole: contextual ? "back" : "menu",
    disabled:
      assistant.loading ||
      ((assistant.sending || uploads.session.sending) &&
        !assistant.conversationId),
    label: contextual
      ? "Back"
      : assistant.conversationId
        ? `Back to ${agentName}`
        : "Back to NyxBot",
    onBack: () => {
      if (contextual) {
        navigation.goBack();
        return;
      }
      if (assistant.conversationId) newChat();
      else {
        setComposerGeneration((value) => value + 1);
        assistant.openConversation();
        navigation.setParams({
          conversationId: undefined,
          acknowledgementId: undefined,
          agentId: undefined,
          draft: false,
          openLatest: false,
        });
        updateDraft("");
      }
    },
  });

  async function sendDraft() {
    if (!canSend || !composerPress.current("send")) return;
    setStopProtected(true);
    following.current = true;
    await perform(() =>
      submitAssistantDraft({
        draft,
        updateDraft,
        getRevision: () => draftRevision.current,
        send: (text) =>
          uploads.session.submit((payload) =>
            assistant.send(text, undefined, payload),
          ),
      }),
    );
  }

  const markdownStyle = {
    body: {
      color: colors.textPrimary,
      fontFamily: fonts.bodyRegular,
      fontSize: 14,
      lineHeight: 23,
    },
    paragraph: { marginTop: 0, marginBottom: 10 },
    heading1: {
      fontFamily: fonts.displaySemi,
      fontSize: 22,
      lineHeight: 29,
      color: colors.textPrimary,
    },
    heading2: {
      fontFamily: fonts.displaySemi,
      fontSize: 18,
      lineHeight: 25,
      color: colors.textPrimary,
    },
    link: {
      color: colors.primaryOnTint,
      textDecorationLine: "underline" as const,
    },
    code_inline: {
      fontFamily: fonts.mono,
      backgroundColor: colors.cardSoft,
      color: colors.textPrimary,
    },
    fence: {
      fontFamily: fonts.mono,
      fontSize: 12,
      backgroundColor: colors.cardSoft,
      color: colors.textPrimary,
      borderColor: colors.border,
      borderRadius: radius.md,
    },
    blockquote: {
      backgroundColor: colors.cardSoft,
      borderColor: colors.primary,
      color: colors.textSecondary,
    },
    table: { borderColor: colors.border },
    tr: { borderColor: colors.border },
  };
  const markdownRules: RenderRules = {
    ...assistantSelectableMarkdown,
    link: (node, children) => (
      <Text
        key={node.key}
        accessibilityRole="link"
        style={markdownStyle.link}
        onPress={() => openSafeLink(node.attributes.href)}
      >
        {children}
      </Text>
    ),
    // Match Web: model-provided remote images require an explicit link tap.
    image: (node) => (
      <Text
        key={node.key}
        style={{ color: colors.primaryOnTint }}
        onPress={() => {
          if (typeof node.attributes.src === "string")
            openSafeLink(node.attributes.src);
        }}
      >
        {typeof node.attributes.alt === "string" && node.attributes.alt.trim()
          ? node.attributes.alt
          : "Image"}
      </Text>
    ),
  };

  function renderMessage({
    item,
    index,
  }: {
    item: AssistantMessage;
    index: number;
  }) {
    const isUser = item.role === "user";
    const input = retryInput(assistant.history, item);
    return (
      <View
        style={[
          styles.message,
          isUser && styles.userMessage,
          isUser && {
            backgroundColor: colors.card,
            borderColor: colors.border,
          },
        ]}
      >
        {index > 0 && item.seq > (messages[index - 1]?.seq ?? item.seq) + 1 ? (
          <Notice message="Messages between these pages haven't loaded yet. Load missing messages to restore them." />
        ) : null}
        <Label muted style={styles.messageLabel}>
          {isUser
            ? "You"
            : item.role === "assistant"
              ? (conversation?.agent?.display_name ??
                conversation?.agent?.name ??
                "NyxBot")
              : item.role === "orchestrator"
                ? "NyxBot · instruction"
                : "Update"}
          {item.via ? ` · ${item.via}` : ""}
          {item.steering ? " · guidance" : ""}
        </Label>
        <AssistantActivityDisclosure
          key={item.id}
          activities={item.activities}
          onToggle={() => {
            // Opening details is an explicit reading action. Keep the header
            // in view instead of following the newly expanded list to its end.
            following.current = false;
          }}
        />
        {isUser ? (
          item.text ? (
            <Text
              selectable
              style={[styles.messageText, { color: colors.textPrimary }]}
            >
              {item.text}
            </Text>
          ) : null
        ) : (
          <Markdown
            markdownit={assistantMarkdown}
            style={markdownStyle}
            rules={markdownRules}
            onLinkPress={openSafeLink}
          >
            {item.text || (item.status === "failed" ? "" : "No text response.")}
          </Markdown>
        )}
        {item.attachments.map((attachment) => (
          <AssistantAttachmentView
            key={attachment.id}
            attachment={attachment}
            conversationId={assistant.conversationId}
          />
        ))}
        {item.status === "failed" ? (
          <Notice
            message={storedAssistantError(item.error_code)}
            tone={item.error_code === "cancelled" ? "info" : "error"}
          />
        ) : null}
        {input !== undefined ? (
          <ActionButton
            label="Retry unfinished work"
            variant="secondary"
            disabled={
              running ||
              !assistant.online ||
              assistant.recoveryRequired ||
              assistant.loadingOlder
            }
            onPress={() =>
              void perform(() => assistant.send(input, item.turn_id))
            }
          />
        ) : null}
        <Label muted mono style={styles.timestamp}>
          {formatDate(item.created_at)}
        </Label>
        <AssistantCopyButton text={item.text} />
      </View>
    );
  }

  function renderAcknowledgement(row: AssistantAcknowledgement) {
    const pending = row.status === "pending";
    const expired = Date.parse(row.expires_at) <= Date.now();
    return (
      <Card
        key={row.id}
        style={
          row.id === route?.params?.acknowledgementId
            ? { borderColor: colors.primary }
            : undefined
        }
      >
        {row.id === route?.params?.acknowledgementId && (
          <Label style={{ color: colors.primary }}>Selected confirmation</Label>
        )}
        <Label style={styles.cardTitle}>{acknowledgementTitle(row)}</Label>
        <Label>{row.summary}</Label>
        {row.service_name || row.service_slug ? (
          <Label muted>
            Service: {row.service_name ?? row.service_slug}
            {row.service_name && row.service_slug
              ? ` (${row.service_slug})`
              : ""}
          </Label>
        ) : null}
        {row.tool_name ? (
          <Label muted mono>
            Tool: {row.tool_name}
          </Label>
        ) : null}
        <Label muted>
          {row.confirmation ? `${row.confirmation.risk} · ` : ""}
          {pending
            ? row.decider === "orchestrator"
              ? "Requested from NyxBot"
              : "Your confirmation is required"
            : row.status}
          {row.decided_by
            ? ` · ${row.decided_by === "user" ? "you" : "NyxBot"}`
            : ""}
        </Label>
        {row.reason ? <Label muted>{row.reason}</Label> : null}
        {row.confirmation ? (
          <Label muted>
            {row.confirmation.reason === "webhook"
              ? "A webhook requested this action."
              : row.confirmation.reason === "policy"
                ? "The resource's confirmation policy requires your approval."
                : `${row.confirmation.access_mode === "ask_every_time" ? "Ask Every Time" : row.confirmation.access_mode === "nyxmode" ? "NyxMode" : row.confirmation.access_mode === "autopilot" ? "Autopilot" : "Your access mode"} requires this confirmation.`}
          </Label>
        ) : null}
        {row.authored_skill ? (
          <NativeAuthoredSkillReview
            acknowledgement={row}
            initiallyOpen={row.id === route?.params?.acknowledgementId}
            disabled={Boolean(assistant.deciding) || assistant.loadingOlder}
            online={assistant.online}
            onDecision={(choice) => assistant.decide(row.id, choice)}
            onRefresh={() => {
              if (conversation?.id)
                return assistant.openConversation(conversation.id);
            }}
          />
        ) : pending ? (
          <>
            <Label muted>
              {row.kind === "action"
                ? "This applies once, to this action only."
                : row.kind === "operations"
                  ? "This updates the specialist's operations across its threads."
                  : row.kind === "skills"
                    ? "NyxBot will resolve the exact version and ask separately before attaching it."
                    : "This grants the requested permission."}
            </Label>
            <Label muted mono>
              Expires {formatDate(row.expires_at)}
            </Label>
            <View style={styles.row}>
              <View style={styles.fill}>
                <ActionButton
                  label="Deny"
                  variant="secondary"
                  disabled={
                    !!assistant.deciding ||
                    expired ||
                    !assistant.online ||
                    assistant.loadingOlder
                  }
                  onPress={() =>
                    void perform(() => assistant.decide(row.id, "deny"))
                  }
                />
              </View>
              <View style={styles.fill}>
                <ActionButton
                  label="Allow"
                  loading={assistant.deciding === row.id}
                  disabled={
                    !!assistant.deciding ||
                    expired ||
                    !assistant.online ||
                    assistant.loadingOlder
                  }
                  onPress={() =>
                    void perform(() => assistant.decide(row.id, "allow"))
                  }
                />
              </View>
            </View>
          </>
        ) : null}
      </Card>
    );
  }

  return (
    <WorkspacePage title={chatTitle} showHeading={false} scroll={false}>
      <View style={styles.chatHeading}>
        <View style={styles.fill}>
          <Label muted style={styles.currentChatLabel}>
            Current chat
          </Label>
          <Text
            accessibilityRole="header"
            numberOfLines={1}
            ellipsizeMode="tail"
            style={[styles.chatTitle, { color: colors.textPrimary }]}
          >
            {chatTitle}
          </Text>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Chat details"
          onPress={() => {
            setTitle(conversation?.title ?? "");
            setDetailsOpen(true);
          }}
          style={({ pressed }) => [
            styles.labeledAction,
            { opacity: pressed ? 0.65 : 1 },
          ]}
        >
          <Info size={17} color={colors.textSecondary} />
          <Label muted style={styles.actionLabel}>
            Details
          </Label>
        </Pressable>
      </View>
      <View style={styles.chatNavigation}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Chatting with ${agentName}. Choose agent`}
          onPress={() => setAgentsOpen(true)}
          style={[styles.agentSelector, { backgroundColor: colors.cardSoft }]}
        >
          <Bot size={17} color={colors.primaryOnTint} />
          <View style={styles.fill}>
            <Label numberOfLines={1} style={styles.agentName}>
              {agentName}
            </Label>
            {assistant.agent?.owner_kind === "org" ? (
              <Label muted numberOfLines={1} style={styles.ownerLabel}>
                {assistant.agent.owner_name ?? "Organization"}
              </Label>
            ) : null}
          </View>
          <ChevronDown size={15} color={colors.textMuted} />
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Chat history"
          onPress={() => setHistoryOpen(true)}
          style={({ pressed }) => [
            styles.labeledAction,
            { opacity: pressed ? 0.65 : 1 },
          ]}
        >
          <History size={17} color={colors.textSecondary} />
          <Label muted style={styles.actionLabel}>
            History
          </Label>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="New chat"
          accessibilityState={{ disabled: newChatDisabled }}
          disabled={newChatDisabled}
          onPress={newChat}
          style={({ pressed }) => [
            styles.labeledAction,
            styles.newChatAction,
            {
              backgroundColor: colors.primaryGlow,
              opacity: newChatDisabled ? 0.4 : pressed ? 0.65 : 1,
            },
          ]}
        >
          <SquarePen size={17} color={colors.primaryOnTint} />
          <Label style={[styles.actionLabel, { color: colors.primaryOnTint }]}>
            New chat
          </Label>
        </Pressable>
      </View>
      {!assistant.online ? (
        <Notice message="You're offline. Reconnect to refresh this chat and send messages." />
      ) : null}
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
          <Notice message={localError ?? assistant.error ?? ""} tone="error" />
          <ActionButton
            label="Refresh history"
            variant="secondary"
            onPress={() => void assistant.recover()}
          />
        </View>
      ) : null}
      {destroyed ? (
        <Notice message="This agent was destroyed. This chat is read-only." />
      ) : null}
      {route?.params?.openLatest ? (
        <Notice message={`Opening ${agentName}'s latest chat…`} />
      ) : null}
      <FlatList
        key={assistant.conversationId ?? "new-chat"}
        ref={transcript}
        style={styles.fill}
        data={messages}
        keyExtractor={(item) => item.id}
        renderItem={renderMessage}
        ItemSeparatorComponent={TranscriptSeparator}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="on-drag"
        contentContainerStyle={styles.transcript}
        // Anchor persisted rows when paging history. A live reply footer
        // disappears into saved rows, so disable anchoring while it changes.
        maintainVisibleContentPosition={
          stableTranscript ? { minIndexForVisible: 0 } : undefined
        }
        onLayout={(event) => {
          const metrics = transcriptMetrics.current;
          metrics.viewportHeight = event.nativeEvent.layout.height;
          if (following.current)
            transcript.current?.scrollToOffset({
              offset: transcriptEndOffset(
                metrics.height,
                metrics.viewportHeight,
              ),
              animated: false,
            });
        }}
        onScroll={(event) => {
          const { contentOffset, contentSize, layoutMeasurement } =
            event.nativeEvent;
          following.current =
            contentSize.height - layoutMeasurement.height - contentOffset.y <
            100;
        }}
        onScrollBeginDrag={() => {
          following.current = false;
        }}
        scrollEventThrottle={100}
        onContentSizeChange={(_width, height) => {
          const metrics = transcriptMetrics.current;
          metrics.height = height;
          if (following.current) {
            transcript.current?.scrollToOffset({
              offset: transcriptEndOffset(height, metrics.viewportHeight),
              animated: false,
            });
          }
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
                following.current = false;
                void assistant.loadOlder();
              }}
            />
          ) : null
        }
        ListHeaderComponentStyle={styles.transcriptHeader}
        ListEmptyComponent={
          assistant.loading ? (
            <ActivityIndicator color={colors.primary} />
          ) : !assistant.sending ? (
            <View style={styles.welcome}>
              <View
                style={[
                  styles.welcomeIcon,
                  { backgroundColor: colors.primaryGlow },
                ]}
              >
                <MessageSquare color={colors.primary} size={30} />
              </View>
              <Text
                style={[styles.welcomeTitle, { color: colors.textPrimary }]}
              >
                What would you like to do?
              </Text>
              <Label muted>
                {currentAgent?.kind === "specialist"
                  ? `Start a conversation with ${agentName}.`
                  : `Ask ${agentName} to help with your connected services.`}
              </Label>
              {["Show my connected services", "Check my pending approvals"].map(
                (prompt) => (
                  <Pressable
                    key={prompt}
                    accessibilityRole="button"
                    onPress={() => updateDraft(prompt)}
                    style={[
                      styles.suggestion,
                      {
                        borderColor: colors.border,
                        backgroundColor: colors.card,
                      },
                    ]}
                  >
                    <Label>{prompt}</Label>
                  </Pressable>
                ),
              )}
            </View>
          ) : null
        }
        ListFooterComponent={
          <View style={styles.footer}>
            {(assistant.inputText || assistant.inputAttachments.length > 0) &&
            !optimisticUserPersisted ? (
              <View
                style={[
                  styles.message,
                  styles.userMessage,
                  { backgroundColor: colors.card, borderColor: colors.border },
                ]}
              >
                <Label muted style={styles.messageLabel}>
                  You · sending
                </Label>
                {assistant.inputText ? (
                  <Text
                    selectable
                    style={[styles.messageText, { color: colors.textPrimary }]}
                  >
                    {assistant.inputText}
                  </Text>
                ) : null}
                {assistant.inputAttachments.map((attachment) => (
                  <AssistantAttachmentView
                    key={attachment.id}
                    attachment={attachment}
                    conversationId={
                      assistant.conversationId ?? assistant.live?.conversationId
                    }
                  />
                ))}
              </View>
            ) : null}
            {running && !streamPersisted ? (
              <View style={styles.message}>
                <Label muted style={styles.messageLabel}>
                  {conversation?.agent?.display_name ??
                    conversation?.agent?.name ??
                    "NyxBot"}{" "}
                  ·{" "}
                  {conversation?.active_turn?.stop_requested
                    ? "Stopping"
                    : "Working"}
                </Label>
                <AssistantActivityDisclosure
                  key={
                    conversation?.active_turn?.turn_id ??
                    assistant.live?.turnId ??
                    "pending-turn"
                  }
                  activities={liveActivities}
                  onToggle={() => {
                    following.current = false;
                  }}
                />
                {liveText ? (
                  <Markdown
                    markdownit={assistantMarkdown}
                    style={markdownStyle}
                    rules={markdownRules}
                    onLinkPress={openSafeLink}
                  >
                    {liveText}
                  </Markdown>
                ) : (
                  <View style={[styles.row, styles.thinking]}>
                    <ActivityIndicator size="small" color={colors.primary} />
                    {waitingActivity ? (
                      <Label
                        muted
                        numberOfLines={1}
                        style={styles.runningActivity}
                      >
                        {describeToolActivity(
                          waitingActivity.label,
                          waitingActivity.status === "completed"
                            ? "done"
                            : waitingActivity.status,
                        )}
                        {waitingActivity.status === "running" ? "…" : ""}
                      </Label>
                    ) : null}
                  </View>
                )}
                {assistant.live?.notice ? (
                  <Label muted>{assistant.live.notice}</Label>
                ) : null}
                <AssistantCopyButton text={liveText} partial />
              </View>
            ) : null}
            {assistant.history?.waiting.map((waiting, index) => (
              <Notice
                key={`${waiting.title}:${index}`}
                message={`${waiting.title}${waiting.detail ? ` · ${waiting.detail}` : ""}`}
              />
            ))}
            {assistant.history?.acknowledgements.map(renderAcknowledgement)}
            {assistant.history?.approvals.map((approval) => (
              <Card key={approval.id}>
                <Label style={styles.cardTitle}>
                  {approval.service_name} · Approval needed
                </Label>
                <Label>{approval.summary}</Label>
                <Label muted mono>
                  Expires {formatDate(approval.expires_at)}
                </Label>
                <ActionButton
                  label="Review request"
                  variant="secondary"
                  onPress={() =>
                    navigation.push("ActivityDetail", {
                      challengeId: approval.id,
                    })
                  }
                />
              </Card>
            ))}
          </View>
        }
      />
      <View
        style={[
          styles.composer,
          { backgroundColor: colors.card, borderColor: colors.border },
        ]}
      >
        <AssistantPendingUploads
          files={uploads.files}
          remove={uploads.remove}
          disabled={running || uploads.session.sending}
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
        <TextInput
          accessibilityLabel="Message to assistant"
          value={draft}
          onChangeText={updateDraft}
          multiline
          editable={
            !destroyed &&
            !assistant.recoveryRequired &&
            !route?.params?.openLatest
          }
          placeholder={`Message ${agentName}…`}
          placeholderTextColor={colors.textMuted}
          selectionColor={colors.primary}
          style={[styles.composerInput, { color: colors.textPrimary }]}
        />
        <View style={styles.composerActions}>
          <IconButton
            label="Attach files"
            disabled={
              !canCompose ||
              uploads.picking ||
              uploads.session.sending ||
              uploads.files.length >= 10
            }
            onPress={() => void uploads.pick()}
          >
            {uploads.picking ? (
              <ActivityIndicator size="small" color={colors.primary} />
            ) : (
              <Paperclip size={20} color={colors.textSecondary} />
            )}
          </IconButton>
          <Label muted style={styles.composerHint}>
            {running
              ? "You can leave this chat and return later."
              : assistant.recoveryRequired
                ? "Open the saved chat before continuing."
                : uploads.blocked
                  ? "Wait for uploads or remove failed files."
                  : "Attach images or documents · 20 MB each"}
          </Label>
          {running && assistant.conversationId ? (
            <IconButton
              label="Stop assistant"
              disabled={
                stopProtected ||
                assistant.mutating ||
                conversation?.active_turn?.stop_requested
              }
              onPress={() => {
                if (composerPress.current("stop")) void assistant.stop();
              }}
            >
              <Square size={18} color={colors.danger} />
            </IconButton>
          ) : (
            <IconButton
              label="Send message"
              disabled={!canSend}
              onPress={() => void sendDraft()}
            >
              <Send
                size={20}
                color={canSend ? colors.primary : colors.textTertiary}
              />
            </IconButton>
          )}
        </View>
        {[...draft].length > 32768 ? (
          <Label style={{ color: colors.danger }}>
            Keep the message under 32,768 characters.
          </Label>
        ) : null}
      </View>

      <Sheet
        visible={agentsOpen}
        title="Choose agent"
        onClose={() => setAgentsOpen(false)}
      >
        {assistant.agentsLoading ? (
          <ActivityIndicator color={colors.primary} />
        ) : null}
        {assistant.agentsError ? (
          <Notice message={assistant.agentsError} tone="error" />
        ) : null}
        {assistant.agents
          .filter(
            (agent) => agent.status !== "destroyed" && agent.can_use === true,
          )
          .map((agent) => (
            <Pressable
              key={agent.id}
              accessibilityRole="button"
              accessibilityLabel={`Chat with ${chatAgentLabel(agent)}`}
              onPress={() => {
                setComposerGeneration((value) => value + 1);
                assistant.openConversation();
                assistant.setModel(agent.model ?? "nyxagent/chat");
                navigation.setParams({
                  agentId: agent.id,
                  conversationId: undefined,
                  acknowledgementId: undefined,
                  draft: false,
                  openLatest: true,
                });
                updateDraft("");
                setAgentsOpen(false);
              }}
            >
              <Card>
                <Label>
                  {chatAgentLabel(agent)}
                  {assistant.agentId === agent.id ? " · Selected" : ""}
                </Label>
                <Label muted>
                  {agent.owner_kind === "org"
                    ? (agent.owner_name ?? "Organization")
                    : "Personal"}{" "}
                  · @{agent.name}
                </Label>
              </Card>
            </Pressable>
          ))}
        <ActionButton
          label="Manage agents"
          variant="secondary"
          onPress={() => {
            setAgentsOpen(false);
            if (contextual) navigation.popTo("Workspace", { screen: "Agents" });
            else navigation.navigate("Agents");
          }}
        />
      </Sheet>

      <Sheet
        visible={historyOpen}
        title={`${agentName} · Chat history`}
        onClose={() => setHistoryOpen(false)}
      >
        <ActionButton
          label="New chat"
          onPress={newChat}
          disabled={newChatDisabled}
        />
        {!assistant.conversations.length && !assistant.loadingIndex ? (
          <Label muted>Your saved chats will appear here.</Label>
        ) : null}
        {assistant.conversations.map((item) => (
          <Pressable
            key={item.id}
            accessibilityRole="button"
            accessibilityLabel={`Open chat ${item.title}${item.active_turn ? ", Working" : ""}${item.pending_acknowledgements ? `, ${item.pending_acknowledgements} requests` : ""}, ${formatDate(item.last_message_at)}`}
            accessibilityState={{
              selected: item.id === assistant.conversationId,
            }}
            onPress={() => {
              assistant.openConversation(item.id);
              if (item.id !== assistant.conversationId) {
                navigation.setParams({
                  conversationId: item.id,
                  acknowledgementId: undefined,
                  agentId: item.agent?.id ?? assistant.agentId,
                  openLatest: false,
                });
                updateDraft("");
              }
              setHistoryOpen(false);
            }}
          >
            <Card
              style={
                item.id === assistant.conversationId
                  ? {
                      borderColor: colors.primary,
                      backgroundColor: colors.primaryGlow,
                    }
                  : undefined
              }
            >
              <View style={styles.row}>
                <Label
                  numberOfLines={2}
                  style={[styles.cardTitle, styles.fill]}
                >
                  {item.title}
                </Label>
                {item.id === assistant.conversationId ? (
                  <View style={styles.currentChatBadge}>
                    <Check size={16} color={colors.primaryOnTint} />
                    <Label
                      style={[
                        styles.actionLabel,
                        { color: colors.primaryOnTint },
                      ]}
                    >
                      Current
                    </Label>
                  </View>
                ) : null}
              </View>
              <Label muted>
                {item.agent?.display_name ?? item.agent?.name ?? "NyxBot"}
                {item.active_turn ? " · Working" : ""}
                {item.pending_acknowledgements
                  ? ` · ${item.pending_acknowledgements} requests`
                  : ""}
              </Label>
              <Label muted mono>
                {formatDate(item.last_message_at)}
              </Label>
            </Card>
          </Pressable>
        ))}
        {assistant.loadingIndex ? (
          <ActivityIndicator color={colors.primary} />
        ) : null}
        {assistant.hasMoreConversations ? (
          <ActionButton
            label="Load more chats"
            variant="secondary"
            loading={assistant.loadingIndex}
            onPress={() => void assistant.loadMoreConversations()}
          />
        ) : null}
      </Sheet>
      <Sheet
        visible={modelOpen}
        title="Choose a model"
        onClose={() => setModelOpen(false)}
      >
        {assistant.models.length ? (
          assistant.models.map((item) => (
            <Pressable
              key={item.id}
              accessibilityRole="button"
              disabled={!item.available || modelLocked}
              onPress={() => {
                if (modelLocked) return;
                assistant.setModel(item.id);
                setModelOpen(false);
              }}
            >
              <Card>
                <Label
                  style={{
                    color:
                      item.id === assistant.model
                        ? colors.primaryOnTint
                        : colors.textPrimary,
                  }}
                >
                  {item.label}
                  {item.id === assistant.model ? " · Selected" : ""}
                </Label>
                {item.description ? (
                  <Label muted>{item.description}</Label>
                ) : null}
                {!item.available ? <Label muted>Unavailable</Label> : null}
              </Card>
            </Pressable>
          ))
        ) : (
          <Notice message="Model choices are unavailable. The default assistant model will be used." />
        )}
      </Sheet>
      <Sheet
        visible={detailsOpen}
        title="Chat details"
        onClose={() => setDetailsOpen(false)}
      >
        <Card>
          <Label style={styles.cardTitle}>Agent</Label>
          <Label>{agentName}</Label>
          <Label muted>
            {assistant.agent?.owner_kind === "org"
              ? (assistant.agent.owner_name ?? "Organization")
              : currentAgent?.kind === "specialist"
                ? "Personal specialist"
                : "Your personal assistant"}
          </Label>
        </Card>
        <Card>
          <Label style={styles.cardTitle}>Model</Label>
          <Label>{currentModelLabel}</Label>
          <Label muted>
            {assistant.conversationId
              ? "The model used by this saved chat. Model selection is available when starting a new chat."
              : "Choose the model for this new chat."}
          </Label>
          {!assistant.conversationId ? (
            <ActionButton
              label="Choose model"
              variant="secondary"
              disabled={modelLocked}
              onPress={() => {
                setDetailsOpen(false);
                setModelOpen(true);
              }}
            />
          ) : null}
        </Card>
        {currentAgent?.id ? (
          <ActionButton
            label="Agent details"
            variant="secondary"
            onPress={() => {
              setDetailsOpen(false);
              navigation.push("AgentDetail", { agentId: currentAgent.id });
            }}
          />
        ) : null}
        {assistant.conversationId ? (
          <View style={styles.detailActions}>
            <Field
              label="Chat title"
              value={title}
              onChangeText={setTitle}
              maxLength={200}
              autoCapitalize="sentences"
            />
            <ActionButton
              label="Save title"
              loading={assistant.mutating}
              disabled={!title.trim()}
              onPress={() =>
                void perform(async () => {
                  if (await assistant.rename(title)) setDetailsOpen(false);
                })
              }
            />
            <ActionButton
              label="Delete chat"
              variant="danger"
              disabled={assistant.mutating}
              onPress={() =>
                Alert.alert(
                  "Delete this chat?",
                  "This deletes its saved messages and permissions. Active work will be stopped.",
                  [
                    { text: "Cancel", style: "cancel" },
                    {
                      text: "Delete",
                      style: "destructive",
                      onPress: () => {
                        setDetailsOpen(false);
                        void assistant.remove().then((removed) => {
                          if (removed)
                            navigation.setParams({
                              conversationId: undefined,
                              acknowledgementId: undefined,
                            });
                          if (removed) updateDraft("");
                        });
                      },
                    },
                  ],
                )
              }
            />
          </View>
        ) : null}
      </Sheet>
    </WorkspacePage>
  );
}

function TranscriptSeparator() {
  return <View style={styles.transcriptSeparator} />;
}

function IconButton({
  children,
  label,
  onPress,
  disabled,
}: {
  children: React.ReactNode;
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.iconButton,
        { opacity: disabled ? 0.4 : pressed ? 0.65 : 1 },
      ]}
    >
      {children}
    </Pressable>
  );
}

function Sheet({
  visible,
  title,
  onClose,
  children,
}: {
  visible: boolean;
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <Modal
      visible={visible}
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={onClose}
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
        <ScrollView
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.sheetContent}
        >
          {children}
        </ScrollView>
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
const styles = StyleSheet.create({
  fill: { flex: 1 },
  row: { flexDirection: "row", alignItems: "center", gap: 8 },
  chatHeading: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  currentChatLabel: { fontSize: 10, lineHeight: 15 },
  chatTitle: { fontFamily: fonts.displaySemi, fontSize: 22, lineHeight: 30 },
  chatNavigation: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  agentSelector: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    minHeight: 44,
    paddingHorizontal: 10,
    paddingVertical: 6,
    borderRadius: radius.md,
  },
  agentName: { fontSize: 12, fontFamily: fonts.bodySemi },
  ownerLabel: { fontSize: 10, lineHeight: 13 },
  labeledAction: {
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 5,
    paddingHorizontal: 8,
    borderRadius: radius.md,
  },
  actionLabel: { fontSize: 11, fontFamily: fonts.bodySemi },
  newChatAction: { paddingHorizontal: 10 },
  currentChatBadge: { flexDirection: "row", alignItems: "center", gap: 4 },
  detailActions: { gap: 14 },
  iconButton: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  transcript: { paddingVertical: 8, flexGrow: 1 },
  transcriptHeader: { marginBottom: 20 },
  transcriptSeparator: { height: 20 },
  message: { gap: 8, paddingVertical: 10 },
  userMessage: {
    padding: 14,
    borderWidth: 1,
    borderRadius: radius.lg,
    marginLeft: 26,
  },
  messageLabel: { fontSize: 11, fontFamily: fonts.bodySemi },
  messageText: { fontFamily: fonts.bodyRegular, fontSize: 14, lineHeight: 23 },
  metadata: { fontSize: 11 },
  runningActivity: { flexShrink: 1, fontSize: 11 },
  timestamp: { fontSize: 10 },
  footer: { gap: 16, paddingTop: 20 },
  welcome: { alignItems: "center", gap: 16, paddingVertical: 36 },
  welcomeIcon: {
    width: 64,
    height: 64,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 20,
  },
  welcomeTitle: {
    fontFamily: fonts.displaySemi,
    fontSize: 24,
    lineHeight: 32,
    textAlign: "center",
  },
  suggestion: {
    width: "100%",
    padding: 14,
    borderWidth: 1,
    borderRadius: radius.lg,
  },
  thinking: { alignSelf: "flex-start", marginVertical: 10 },
  cardTitle: { fontFamily: fonts.bodySemi },
  composer: { borderWidth: 1, borderRadius: radius.lg, padding: 12, gap: 8 },
  composerInput: {
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 22,
    minHeight: 44,
    maxHeight: 140,
    paddingVertical: 8,
    textAlignVertical: "top",
  },
  composerActions: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 10,
  },
  composerHint: { flex: 1, fontSize: 10, lineHeight: 15 },
  errorRow: { gap: 8 },
  sheet: { flex: 1, paddingTop: 20 },
  sheetHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 20,
    gap: 16,
  },
  sheetTitle: { fontFamily: fonts.displaySemi, fontSize: 22, lineHeight: 30 },
  sheetContent: { padding: 20, paddingBottom: 40, gap: 14 },
});
