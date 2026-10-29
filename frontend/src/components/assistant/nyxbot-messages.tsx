import { useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
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
} from "lucide-react";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import { MessageTime } from "@/components/assistant/message-time";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import {
  describeEventNotice,
  eventNotices,
  type EventNotice,
  type EventNoticeTone,
} from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import { ACCORDION_MAX_W, BANNER_CHEVRON, BANNER_FRAME, BANNER_NEUTRAL, BANNER_ROW } from "@/lib/assistant/banner";

const TONE: Record<EventNoticeTone, string> = {
  neutral: "text-text-tertiary",
  success: "text-success",
  warning: "text-warning",
  destructive: "text-destructive",
};

function NoticeIcon({ notice }: { readonly notice: EventNotice }) {
  const className = cn("h-3.5 w-3.5 shrink-0", TONE[notice.tone]);
  switch (notice.kind) {
    case "reply":
      return <MessageSquareReply aria-hidden="true" className={className} />;
    case "request":
      return <ShieldQuestionMark aria-hidden="true" className={className} />;
    case "decision":
    case "approval":
      return notice.tone === "success" ? (
        <ShieldCheck aria-hidden="true" className={className} />
      ) : (
        <ShieldX aria-hidden="true" className={className} />
      );
    case "connection":
      return <PlugZap aria-hidden="true" className={className} />;
    case "channel":
      return <Radio aria-hidden="true" className={className} />;
    case "message":
      return <MessageCircle aria-hidden="true" className={className} />;
    case "task":
      return <ListChecks aria-hidden="true" className={className} />;
    case "notice":
      return <BellRing aria-hidden="true" className={className} />;
  }
}

/** One NyxID notice as a compact row: icon, one line, details on demand. */
function EventRow({
  notice,
  icon,
  action,
}: {
  readonly notice: EventNotice;
  readonly icon?: ReactNode;
  readonly action?: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const head = (
    <>
      {icon ?? <NoticeIcon notice={notice} />}
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{notice.summary}</span>
    </>
  );
  return (
    <div role="note" aria-label={`NyxID event: ${notice.summary}`}>
      {notice.detail ? (
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className={cn(BANNER_ROW, "transition-colors hover:bg-overlay")}
        >
          {head}
          <ChevronRight
            aria-hidden="true"
            className={cn(BANNER_CHEVRON, open && "rotate-90")}
          />
        </button>
      ) : (
        <div className={BANNER_ROW}>{head}</div>
      )}
      {open && notice.detail ? (
        <div className="border-t border-hairline px-3 py-2">
          <p className="max-h-64 overflow-y-auto whitespace-pre-wrap break-words leading-relaxed text-muted-foreground">
            {notice.detail}
          </p>
          {action}
        </div>
      ) : null}
    </div>
  );
}

/** The message an automation run starts with: its name, the instruction on demand. */
function AutomationNotice({ message }: { readonly message: ChatMessage }) {
  // A webhook automation records the event payload as its own message.
  const payload = message.id.startsWith("trigger-event:");
  const label = message.automation?.label;
  const summary = `${payload ? "Webhook event" : "Automation"}${label ? ` · ${label}` : " run"}`;
  return (
    <div role="group" aria-label="NyxID updates" className={cn(ACCORDION_MAX_W, BANNER_FRAME, BANNER_NEUTRAL)}>
      <EventRow
        notice={{ kind: "notice", tone: "neutral", summary, detail: message.content.trim() || undefined }}
        icon={<CalendarClock aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />}
        action={
          <Link
            to="/assistant/automations"
            className="mt-2 inline-block text-11 text-muted-foreground underline hover:text-foreground"
          >
            Open Automations
          </Link>
        }
      />
    </div>
  );
}

/**
 * The NyxID notices that woke the agent (a specialist replied or asked for
 * access, a decision was made, a connection or channel bot finished, a
 * background task returned) or the automation run that started it. Not
 * something the user said, so it is one compact frame with a row per notice
 * rather than a bubble.
 */
export function NyxBotEventNotice({ message }: { readonly message: ChatMessage }) {
  if (message.origin === "trigger" || message.automation) {
    return <AutomationNotice message={message} />;
  }
  const notices = eventNotices(message.content).map(describeEventNotice);
  return (
    <div
      role="group"
      aria-label="NyxID updates"
      className={cn(ACCORDION_MAX_W, BANNER_FRAME, BANNER_NEUTRAL, "divide-y divide-hairline")}
    >
      {notices.map((notice, index) => (
        <EventRow key={index} notice={notice} />
      ))}
    </div>
  );
}

const NYXBOT_MARK = { id: "nyxbot", name: "NyxBot", kind: "nyxbot" } as const;

/** An instruction NyxBot sent to this specialist: shaped like a user turn, labelled as NyxBot's. */
export function NyxBotOrchestratorMessage({ message }: { readonly message: ChatMessage }) {
  return (
    <article aria-label="Message from NyxBot" className="group/message flex flex-col items-end gap-1">
      <div className="flex items-center gap-1.5 text-10 text-text-tertiary">
        <AgentAvatar agent={NYXBOT_MARK} size="xs" />
        From NyxBot
      </div>
      <div className="ml-auto max-w-[78%] whitespace-pre-wrap rounded-lg border border-hairline bg-overlay px-3 py-2 text-12 leading-relaxed text-foreground">
        {message.content}
      </div>
      <MessageTime value={message.timestamp} reveal />
    </article>
  );
}
