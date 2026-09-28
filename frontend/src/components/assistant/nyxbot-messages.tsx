import { useState } from "react";
import { BellRing, ChevronRight } from "lucide-react";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { eventNotices } from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";

const COLLAPSE_LINES = 2;
const COLLAPSE_CHARS = 240;

/**
 * A NyxID notice that woke the agent (a subagent replied, a permission was
 * decided). It is not something the user said, so it is a quiet note rather
 * than a bubble; long batches start collapsed.
 */
export function NyxBotEventNotice({ message }: { readonly message: ChatMessage }) {
  const notices = eventNotices(message.content);
  const long = notices.length > COLLAPSE_LINES || message.content.length > COLLAPSE_CHARS;
  const [open, setOpen] = useState(!long);
  const shown = open ? notices : notices.slice(0, 1);
  return (
    <div
      role="note"
      aria-label="NyxID event"
      className="ml-[30px] rounded-lg border border-hairline bg-overlay/35 px-3 py-2 text-[11px] leading-relaxed text-muted-foreground"
    >
      <div className="flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary">
        <BellRing aria-hidden="true" className="h-3 w-3" />
        NyxID update
      </div>
      <ul className="mt-1 space-y-0.5">
        {shown.map((notice, index) => (
          <li key={index} className={cn("break-words", !open && "line-clamp-2")}>
            {notice}
          </li>
        ))}
      </ul>
      {long ? (
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
          className="mt-1 flex items-center gap-1 text-[11px] text-text-tertiary transition-colors hover:text-muted-foreground"
        >
          <ChevronRight className={cn("h-3 w-3 transition-transform", open && "rotate-90")} />
          {open ? "Show less" : `Show all ${String(notices.length)}`}
        </button>
      ) : null}
    </div>
  );
}

/** An instruction NyxBot sent to this subagent: shaped like a user turn, labelled as NyxBot's. */
export function NyxBotOrchestratorMessage({ message }: { readonly message: ChatMessage }) {
  return (
    <article aria-label="Message from NyxBot" className="flex flex-col items-end gap-1">
      <div className="text-[10px] text-text-tertiary">From NyxBot (orchestrator)</div>
      <div className="ml-auto max-w-[78%] rounded-lg border border-hairline bg-overlay px-3 py-2 text-[12px] leading-relaxed text-foreground whitespace-pre-wrap">
        {message.content}
      </div>
    </article>
  );
}
