import { useEffect, useId, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { cmaReadableLabel } from "@/lib/assistant/cma-display";
import { cmaTurnEvents, cmaTurnStatus } from "@/lib/assistant/cma-conversation";
import type { CmaDisplayEvent } from "@/lib/assistant/cma-events";
import {
  cmaPublicText,
  type CmaTranscriptTurn,
} from "@/schemas/nyxbot-cma-agent";
import {
  CmaDisclosure,
  CmaMessage,
  CmaStatus,
  CmaTime,
} from "./cma-execution-content";

export function CmaConversation({
  turns,
  events,
  linkedTurn,
  hasLinkedRequest,
}: {
  readonly turns: readonly CmaTranscriptTurn[];
  readonly events: readonly CmaDisplayEvent[];
  readonly linkedTurn?: number;
  readonly hasLinkedRequest: boolean;
}) {
  const ordered = [...turns].sort((a, b) => b.turn_sequence - a.turn_sequence);
  const latest = ordered[0]?.turn_sequence;
  // Live arrivals do not collapse or scroll away from the entry being read.
  const [focus, setFocus] = useState(
    linkedTurn ?? (hasLinkedRequest ? undefined : latest),
  );
  const [scrollTarget, setScrollTarget] = useState(linkedTurn);
  const [expanded, setExpanded] = useState<Record<number, boolean>>({});
  return (
    <div className="space-y-3">
      {!hasLinkedRequest && latest !== focus && (
        <Button
          variant="outline"
          onClick={() => {
            setFocus(latest);
            setScrollTarget(latest);
            if (latest !== undefined)
              setExpanded((previous) => ({ ...previous, [latest]: true }));
          }}
        >
          View new activity
        </Button>
      )}
      {ordered.map((turn) => (
        <ConversationEntry
          key={turn.turn_sequence}
          turn={turn}
          events={cmaTurnEvents(turn, turns, events)}
          latest={turn.turn_sequence === latest}
          linked={turn.turn_sequence === linkedTurn}
          scrollRequested={turn.turn_sequence === scrollTarget}
          open={expanded[turn.turn_sequence] ?? turn.turn_sequence === focus}
          toggle={() =>
            setExpanded((previous) => ({
              ...previous,
              [turn.turn_sequence]: !(
                previous[turn.turn_sequence] ?? turn.turn_sequence === focus
              ),
            }))
          }
        />
      ))}
    </div>
  );
}

function ConversationEntry({
  turn,
  events,
  latest,
  linked,
  scrollRequested,
  open,
  toggle,
}: {
  readonly turn: CmaTranscriptTurn;
  readonly events: readonly CmaDisplayEvent[];
  readonly latest: boolean;
  readonly linked: boolean;
  readonly scrollRequested: boolean;
  readonly open: boolean;
  readonly toggle: () => void;
}) {
  const id = useId();
  const ref = useRef<HTMLElement>(null);
  const didFocus = useRef(false);
  useEffect(() => {
    if (scrollRequested && !didFocus.current) {
      didFocus.current = true;
      ref.current?.scrollIntoView?.({ block: "center" });
    }
  }, [scrollRequested]);
  const status = cmaTurnStatus(turn, events);
  const request = turn.messages.find((message) => message.role === "user");
  const preview = cmaPublicText(request?.content ?? turn.messages[0]?.content)
    .replace(/\s+/g, " ")
    .trim();
  const activity = events.filter((event) => event.type !== "message");
  const messages = turn.messages.filter((message) =>
    ["user", "assistant"].includes(message.role),
  );
  const technicalMessages = turn.messages.filter(
    (message) => !["user", "assistant"].includes(message.role),
  );
  const label = linked
    ? "Linked request"
    : latest
      ? "Latest request"
      : "Earlier request";
  return (
    <section
      ref={ref}
      aria-label={label}
      className={cn(
        "scroll-m-6 overflow-hidden rounded-xl border bg-card shadow-sm",
        linked ? "border-ring" : "border-hairline",
      )}
    >
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={toggle}
        className="flex w-full items-start gap-3 p-4 text-left hover:bg-overlay focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
      >
        <ChevronDown
          aria-hidden
          className={cn(
            "mt-0.5 size-4 shrink-0 text-text-tertiary transition-transform",
            !open && "-rotate-90",
          )}
        />
        <span className="min-w-0 flex-1 space-y-2">
          <span className="flex flex-wrap items-center gap-2">
            <span className="text-12 font-medium">{label}</span>
            <CmaStatus status={status} />
            <span className="ml-auto">
              <CmaTime value={turn.started_at} />
            </span>
          </span>
          {!open && (
            <span className="line-clamp-2 break-words text-13 text-text-tertiary [overflow-wrap:anywhere]">
              {preview || "No published messages"}
            </span>
          )}
        </span>
      </button>
      {open && (
        <div id={id} className="space-y-3 border-t border-hairline p-3 sm:p-4">
          {status === "waiting_approval" && (
            <p className="text-12 text-text-tertiary">
              This request is waiting for confirmation before it can continue.
            </p>
          )}
          {status === "interrupted" && (
            <p className="text-12 text-text-tertiary">
              This request was interrupted. Its available replies are kept
              below.
            </p>
          )}
          {messages.map((message, index) => (
            <CmaMessage
              key={message.sequence ?? message.id ?? index}
              role={message.role}
              text={cmaPublicText(message.content)}
              time={message.created_at}
            />
          ))}
          {!messages.some((message) => message.role === "assistant") && (
            <p className="p-3 text-12 text-text-tertiary">
              {turn.status === "active"
                ? "No assistant reply yet."
                : "No assistant reply was published."}
            </p>
          )}
          <CmaDisclosure
            title="Execution details"
            count={activity.length + technicalMessages.length}
          >
            <p className="mb-3 text-11 text-text-tertiary">
              Work reference: {turn.turn_sequence}
            </p>
            <div className="max-h-[32rem] divide-y divide-hairline overflow-y-auto">
              {technicalMessages.map((message, index) => (
                <article
                  key={message.sequence ?? message.id ?? index}
                  className="space-y-2 py-3 first:pt-0"
                >
                  <h3 className="text-12 font-medium">
                    {cmaReadableLabel(message.role)} message
                  </h3>
                  <p className="whitespace-pre-wrap break-words text-12 text-text-tertiary [overflow-wrap:anywhere]">
                    {cmaPublicText(message.content)}
                  </p>
                </article>
              ))}
              {activity.map((event) => (
                <article key={event.key} className="space-y-2 py-3 first:pt-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="text-12 font-medium">
                      {cmaReadableLabel(event.type)}
                    </h3>
                    {event.status && <CmaStatus status={event.status} />}
                    <span className="ml-auto">
                      <CmaTime value={event.createdAt} />
                    </span>
                  </div>
                  {event.message && (
                    <p className="whitespace-pre-wrap break-words text-12 text-text-tertiary [overflow-wrap:anywhere]">
                      {event.message}
                    </p>
                  )}
                </article>
              ))}
            </div>
            {!activity.length && !technicalMessages.length && (
              <p className="text-12 text-text-tertiary">
                No execution details in the loaded activity.
              </p>
            )}
          </CmaDisclosure>
        </div>
      )}
    </section>
  );
}
