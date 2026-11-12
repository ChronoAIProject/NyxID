import { useMemo, useRef } from "react";
import {
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import {
  replayScript,
  RESEARCH,
  type LiveReplayKind,
} from "../../../../shared/onboardingReplay";
import { AgentAvatar } from "../../components/AgentAvatar";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";
import { chatBubbleColors } from "../../theme/chatAppearance";
import { fonts } from "../../theme/designTokens";
import { ChatMessageTurn, chatSurfaceStyles } from "../assistant/ChatSurface";
import { AssistantReplyBody } from "../assistant/AssistantReplyBody";
import { AssistantAcknowledgementCard } from "../assistant/AssistantAcknowledgementCard";
import {
  ChatComposerFrame,
  ChatComposerRow,
  ComposerSubmitButton,
} from "../assistant/ChatComposer";
import { useReplayScene } from "./useReplayScene";

/** The web fixture timeline, rendered at the actual mobile text size and width. */
export function OnboardingReplay({
  kind,
  topics = [],
  label,
}: {
  kind: LiveReplayKind;
  topics?: readonly string[];
  label: string;
}) {
  const { colors, mode, bubblePalette } = useTheme();
  const bubble = chatBubbleColors(mode, bubblePalette);
  const { height } = useWindowDimensions();
  const reducedMotion = useReducedMotion();
  const topicKey = topics.join(",");
  const beats = useMemo(
    () => replayScript(kind, topicKey.split(",")),
    [kind, topicKey],
  );
  const scene = useReplayScene(beats, reducedMotion);
  const transcript = useRef<ScrollView>(null);
  return (
    <View
      testID="onboarding-replay"
      accessibilityLabel={label}
      style={[
        styles.frame,
        {
          height: Math.min(380, height * 0.44),
          borderColor: colors.border,
          backgroundColor: colors.bg,
        },
      ]}
    >
      <View style={[styles.header, { borderColor: colors.borderSoft }]}>
        <AgentAvatar
          agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }}
          size="sm"
        />
        <Label style={styles.name}>NyxBot</Label>
      </View>
      <ScrollView
        ref={transcript}
        style={styles.transcript}
        contentContainerStyle={styles.messages}
        nestedScrollEnabled
        showsVerticalScrollIndicator={false}
        onContentSizeChange={() =>
          transcript.current?.scrollToEnd({ animated: !reducedMotion })
        }
      >
        <View
          pointerEvents="none"
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        >
          {scene.messages.map((entry) => (
            <View key={entry.id}>
              {entry.continuation_of ? (
                <Label muted style={chatSurfaceStyles.message}>
                  Continuing after your decision
                </Label>
              ) : (
                <ChatMessageTurn user={entry.role === "user"}>
                  {entry.role === "user" ? (
                    <Text
                      style={[
                        chatSurfaceStyles.messageText,
                        { color: bubble.foreground },
                      ]}
                    >
                      {entry.content}
                    </Text>
                  ) : (
                    <AssistantReplyBody text={entry.content} />
                  )}
                </ChatMessageTurn>
              )}
              {scene.specialist?.after === entry.id ? (
                <View style={styles.specialist}>
                  <AgentAvatar agent={RESEARCH} size="xs" />
                  <Label muted style={styles.flex}>
                    {scene.specialist.state === "working"
                      ? "Research is working on it"
                      : "Research replied to NyxBot"}
                  </Label>
                </View>
              ) : null}
              {scene.card?.after === entry.id ? (
                scene.card.row.status === "pending" ? (
                  <AssistantAcknowledgementCard
                    row={scene.card.row}
                    selected={false}
                    disabled={false}
                    deciding={false}
                    onDecision={() => {}}
                  />
                ) : (
                  <Label muted style={chatSurfaceStyles.message}>
                    You allowed: {scene.card.row.summary}
                  </Label>
                )
              ) : null}
            </View>
          ))}
          {scene.working ? <Label muted>{scene.working}</Label> : null}
        </View>
      </ScrollView>
      <View
        style={styles.composer}
        pointerEvents="none"
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        <ChatComposerFrame bounce={false}>
          <ChatComposerRow>
            <Text
              numberOfLines={1}
              style={[
                styles.typed,
                {
                  color: scene.typed
                    ? colors.textPrimary
                    : colors.textSecondary,
                },
              ]}
            >
              {scene.typed || "Message NyxBot"}
            </Text>
            <ComposerSubmitButton
              kind="send"
              label="Replay send"
              disabled={!scene.typed}
              onPress={() => {}}
            />
          </ChatComposerRow>
        </ChatComposerFrame>
      </View>
    </View>
  );
}
const styles = StyleSheet.create({
  frame: {
    width: "100%",
    minWidth: 0,
    maxWidth: "100%",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    overflow: "hidden",
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  name: { fontFamily: fonts.bodySemi },
  flex: { flex: 1, minWidth: 0 },
  transcript: { flex: 1, minHeight: 0 },
  messages: {
    paddingHorizontal: 12,
    paddingVertical: 8,
    flexGrow: 1,
    justifyContent: "flex-end",
  },
  specialist: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingVertical: 8,
  },
  composer: { padding: 8 },
  typed: {
    flex: 1,
    minWidth: 0,
    paddingHorizontal: 12,
    fontFamily: fonts.bodyRegular,
    fontSize: 14,
    lineHeight: 20,
  },
});
