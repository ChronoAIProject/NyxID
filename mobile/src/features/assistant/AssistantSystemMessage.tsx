import { useState } from "react";
import { Pressable, View } from "react-native";
import {
  BellRing,
  CalendarClock,
  ChevronRight,
  ListChecks,
  MessageCircle,
  MessageSquareReply,
  PlugZap,
  Radio,
  ShieldCheck,
  ShieldQuestionMark,
  ShieldX,
} from "lucide-react-native";
import { AgentAvatar } from "../../components/AgentAvatar";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { AssistantMessageTime } from "./AssistantMessageFooter";
import {
  describeEventNotice,
  eventNotices,
  type EventNotice,
} from "./assistantEventNotices";
import type { AssistantMessage } from "./assistantTypes";
import { AssistantAttachmentView } from "./AssistantAttachmentView";

function EventRow({
  notice,
  automation,
  onOpenAutomations,
}: {
  notice: EventNotice;
  automation?: boolean;
  onOpenAutomations: () => void;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const Icon = automation
    ? CalendarClock
    : notice.kind === "reply"
      ? MessageSquareReply
      : notice.kind === "request"
        ? ShieldQuestionMark
        : ["decision", "approval"].includes(notice.kind)
          ? notice.tone === "success"
            ? ShieldCheck
            : ShieldX
          : notice.kind === "connection"
            ? PlugZap
            : notice.kind === "channel"
              ? Radio
              : notice.kind === "message"
                ? MessageCircle
                : notice.kind === "task"
                  ? ListChecks
                  : BellRing;
  const tone =
    notice.tone === "success"
      ? colors.successTone.text
      : notice.tone === "destructive"
        ? colors.dangerTone.text
        : notice.tone === "warning"
          ? colors.warningTone.text
          : colors.textMuted;
  const label = `NyxID event: ${notice.summary}`;
  const head = (
    <>
      <Icon size={14} color={tone} />
      <Label
        muted
        numberOfLines={1}
        maxFontSizeMultiplier={1.3}
        style={{ fontSize: 12, flex: 1 }}
      >
        {notice.summary}
      </Label>
      {notice.detail ? (
        <ChevronRight
          size={14}
          color={colors.textMuted}
          style={{ transform: [{ rotate: open ? "90deg" : "0deg" }] }}
        />
      ) : null}
    </>
  );
  const style = {
    flexDirection: "row" as const,
    alignItems: "center" as const,
    gap: 8,
    minHeight: 34,
    paddingHorizontal: 12,
    paddingVertical: 4,
  };
  return (
    <View>
      {notice.detail ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={label}
          accessibilityState={{ expanded: open }}
          hitSlop={{ top: 5, bottom: 5 }}
          onPress={() => setOpen((value) => !value)}
          style={style}
        >
          {head}
        </Pressable>
      ) : (
        <View accessibilityLabel={label} style={style}>
          {head}
        </View>
      )}
      {open && notice.detail ? (
        <View
          style={{
            paddingHorizontal: 12,
            paddingBottom: 8,
            gap: 4,
          }}
        >
          <Label muted style={{ fontSize: 11 }}>
            {notice.detail}
          </Label>
          {automation ? (
            <Pressable
              accessibilityRole="link"
              accessibilityLabel="Open Automations"
              hitSlop={{ top: 7, bottom: 7 }}
              style={{ minHeight: 30, justifyContent: "center", alignSelf: "flex-start" }}
              onPress={onOpenAutomations}
            >
              <Label style={{ fontSize: 13, color: colors.primary }}>
                Open Automations
              </Label>
            </Pressable>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}
export function AssistantSystemMessage({
  message,
  conversationId,
  onOpenAutomations,
}: {
  message: AssistantMessage;
  conversationId?: string;
  onOpenAutomations: () => void;
}) {
  const { colors } = useTheme();
  if (message.role === "orchestrator")
    return (
      <View
        accessibilityLabel="Message from NyxBot"
        style={{ alignItems: "flex-end", gap: 4 }}
      >
        <View style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
          <AgentAvatar
            agent={{ id: "nyxbot", name: "NyxBot", kind: "nyxbot" }}
            size="xs"
          />
          <Label muted style={{ fontSize: 10 }}>
            From NyxBot
          </Label>
        </View>
        <View
          style={{
            maxWidth: "78%",
            borderWidth: 1,
            borderColor: colors.border,
            backgroundColor: colors.cardSoft,
            paddingHorizontal: 12,
            paddingVertical: 8,
            borderRadius: 8,
          }}
        >
          <Label>{message.text}</Label>
        </View>
        <AssistantMessageTime value={message.created_at} />
      </View>
    );
  const automation = message.origin === "trigger" || !!message.automation;
  const notices = automation
    ? [
        {
          kind: "notice" as const,
          tone: "neutral" as const,
          summary: `${message.id.startsWith("trigger-event:") ? "Webhook event" : "Automation"}${message.automation?.label ? ` · ${message.automation.label}` : " run"}`,
          detail: message.text.trim() || undefined,
        },
      ]
    : eventNotices(message.text).map(describeEventNotice);
  return (
    <View
      accessibilityLabel="NyxID updates"
      accessibilityLiveRegion="polite"
      style={{
        backgroundColor: colors.segmentTrack,
        borderRadius: 12,
        overflow: "hidden",
      }}
    >
      {notices.map((notice, index) => (
        <EventRow
          key={index}
          notice={notice}
          automation={automation}
          onOpenAutomations={onOpenAutomations}
        />
      ))}
      {conversationId && message.attachments.length ? (
        <View style={{ padding: 12, gap: 8 }}>
          {message.attachments.map((attachment) => (
            <AssistantAttachmentView
              key={attachment.id}
              attachment={attachment}
              conversationId={conversationId}
            />
          ))}
        </View>
      ) : null}
    </View>
  );
}
