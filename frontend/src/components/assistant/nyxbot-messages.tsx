import { useState } from "react";
import {
  BellRing,
  ChevronRight,
  MessageCircle,
  MessageSquareReply,
  PlugZap,
  Radio,
  ShieldCheck,
  ShieldQuestionMark,
  ShieldX,
} from "lucide-react";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import {
  describeEventNotice,
  eventNotices,
  type EventNotice,
  type EventNoticeTone,
} from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";

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
    case "notice":
      return <BellRing aria-hidden="true" className={className} />;
  }
}

/** One NyxID notice as a compact activity card: icon, one line, details on demand. */
function EventCard({ notice }: { readonly notice: EventNotice }) {
  const [open, setOpen] = useState(false);
  const head = (
    <>
      <NoticeIcon notice={notice} />
      <span className="min-w-0 flex-1 truncate text-muted-foreground">{notice.summary}</span>
    </>
  );
  return (
    <div
      role="note"
      aria-label={`NyxID event: ${notice.summary}`}
      className="overflow-hidden rounded-lg border border-hairline bg-overlay/35 text-[11px]"
    >
      {notice.detail ? (
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors hover:bg-overlay"
        >
          {head}
          <ChevronRight
            aria-hidden="true"
            className={cn(
              "h-3 w-3 shrink-0 text-text-tertiary transition-transform",
              open && "rotate-90",
            )}
          />
        </button>
      ) : (
        <div className="flex items-center gap-2 px-3 py-1.5">{head}</div>
      )}
      {open && notice.detail ? (
        <p className="whitespace-pre-wrap break-words border-t border-hairline px-3 py-2 leading-relaxed text-muted-foreground">
          {notice.detail}
        </p>
      ) : null}
    </div>
  );
}

/**
 * A NyxID notice that woke the agent (a specialist replied or asked for
 * access, a decision was made, a connection or channel bot finished). It is
 * not something the user said, so each notice is a compact activity card
 * rather than a bubble.
 */
export function NyxBotEventNotice({ message }: { readonly message: ChatMessage }) {
  const notices = eventNotices(message.content).map(describeEventNotice);
  return (
    <div role="group" aria-label="NyxID updates" className="ml-[30px] space-y-1.5">
      {notices.map((notice, index) => (
        <EventCard key={index} notice={notice} />
      ))}
    </div>
  );
}

const NYXBOT_MARK = { id: "nyxbot", name: "NyxBot", kind: "nyxbot" } as const;

/** An instruction NyxBot sent to this specialist: shaped like a user turn, labelled as NyxBot's. */
export function NyxBotOrchestratorMessage({ message }: { readonly message: ChatMessage }) {
  return (
    <article aria-label="Message from NyxBot" className="flex flex-col items-end gap-1">
      <div className="flex items-center gap-1.5 text-[10px] text-text-tertiary">
        <AgentAvatar agent={NYXBOT_MARK} size="xs" />
        From NyxBot
      </div>
      <div className="ml-auto max-w-[78%] whitespace-pre-wrap rounded-lg border border-hairline bg-overlay px-3 py-2 text-[12px] leading-relaxed text-foreground">
        {message.content}
      </div>
    </article>
  );
}
