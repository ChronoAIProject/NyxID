import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cmaTurnEvents, cmaTurnStatus } from "@/lib/assistant/cma-conversation";
import type { CmaDisplayEvent } from "@/lib/assistant/cma-events";
import {
  cmaPublicText,
  type CmaTranscriptTurn,
} from "@/schemas/nyxbot-cma-agent";
import { CmaMessage, CmaStatus } from "./cma-execution-content";
import { CmaConversationDetails } from "./cma-conversation-details";

function turnDate(turn: CmaTranscriptTurn): string | undefined {
  const value =
    turn.started_at ??
    turn.messages.find((message) => message.created_at)?.created_at;
  if (!value || Number.isNaN(Date.parse(value))) return undefined;
  return new Date(value).toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

export function CmaConversation({
  turns,
  events,
  linkedTurn,
  hasLinkedRequest,
  assistantName,
  active = true,
  historyControl,
}: {
  readonly turns: readonly CmaTranscriptTurn[];
  readonly events: readonly CmaDisplayEvent[];
  readonly linkedTurn?: number;
  readonly hasLinkedRequest: boolean;
  readonly assistantName?: string;
  readonly active?: boolean;
  readonly historyControl?: ReactNode;
}) {
  const entries = [...turns]
    .sort((a, b) => a.turn_sequence - b.turn_sequence)
    .map((turn) => ({
      turn,
      events: cmaTurnEvents(turn, turns, events),
      date: turnDate(turn),
    }));
  const latest = entries.at(-1);
  const lastMessage = latest?.turn.messages.at(-1);
  // A reply or status can arrive within the same turn. Loading older pages
  // must not count as new activity or move the reader away from their place.
  const activity = JSON.stringify([
    latest?.turn.turn_sequence,
    latest && cmaTurnStatus(latest.turn, latest.events),
    latest?.turn.messages.length,
    lastMessage?.sequence ?? lastMessage?.id,
    cmaPublicText(lastMessage?.content),
    latest?.events.at(-1)?.key,
  ]);
  const [seenActivity, setSeenActivity] = useState(activity);
  const viewport = useRef<HTMLDivElement>(null);
  const end = useRef<HTMLDivElement>(null);
  const positioned = useRef(false);
  useEffect(() => {
    if (!active || positioned.current || !latest) return;
    if (hasLinkedRequest && linkedTurn === undefined) return;
    const target =
      linkedTurn !== undefined
        ? viewport.current?.querySelector<HTMLElement>(
            `[data-cma-turn="${linkedTurn}"]`,
          )
        : end.current;
    if (!target) return;
    target.scrollIntoView?.({
      block: linkedTurn !== undefined ? "start" : "end",
    });
    positioned.current = true;
  }, [active, hasLinkedRequest, linkedTurn, latest]);

  function jumpToLatest() {
    setSeenActivity(activity);
    end.current?.scrollIntoView?.({ block: "end" });
  }

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 pb-3">
        <p className="min-w-0 flex-1 text-11 leading-relaxed text-text-tertiary">
          Requests and replies from all NyxBot chats that use this assistant, in
          time order.
        </p>
        <Button variant="ghost" size="sm" onClick={jumpToLatest}>
          <ArrowDown aria-hidden />
          Jump to latest
        </Button>
      </div>
      <div
        ref={viewport}
        role="region"
        aria-label="Conversation messages"
        tabIndex={0}
        data-scroll-fade=""
        className="assistant-scrollbar min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain px-1 py-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
        onScroll={(event) => {
          const node = event.currentTarget;
          if (
            node.clientHeight > 0 &&
            node.scrollHeight - node.scrollTop - node.clientHeight < 48
          )
            setSeenActivity(activity);
        }}
      >
        <div className="mx-auto max-w-3xl space-y-9">
          {historyControl}
          {entries.map(({ turn, events: turnEvents, date }, index) => (
            <Fragment key={turn.turn_sequence}>
              {date && date !== entries[index - 1]?.date && (
                <p className="flex items-center justify-center gap-4 text-11 text-text-tertiary">
                  <span aria-hidden className="h-px w-12 bg-hairline" />
                  {date}
                  <span aria-hidden className="h-px w-12 bg-hairline" />
                </p>
              )}
              <ConversationEntry
                turn={turn}
                events={turnEvents}
                assistantName={assistantName}
                linked={turn.turn_sequence === linkedTurn}
                latest={turn.turn_sequence === latest?.turn.turn_sequence}
              />
            </Fragment>
          ))}
          <div ref={end} aria-hidden className="h-1" />
        </div>
      </div>
      {activity !== seenActivity && (
        <div className="pointer-events-none absolute inset-x-0 bottom-4 flex justify-center">
          <Button
            className="pointer-events-auto rounded-full bg-card shadow-lg"
            onClick={jumpToLatest}
          >
            <ArrowDown aria-hidden />
            View new activity
          </Button>
        </div>
      )}
    </div>
  );
}

function ConversationEntry({
  turn,
  events,
  assistantName,
  latest,
  linked,
}: {
  readonly turn: CmaTranscriptTurn;
  readonly events: readonly CmaDisplayEvent[];
  readonly assistantName?: string;
  readonly latest: boolean;
  readonly linked: boolean;
}) {
  const status = cmaTurnStatus(turn, events);
  const messages = turn.messages.filter((message) =>
    ["user", "assistant"].includes(message.role),
  );
  const requestIndex = messages.findIndex((message) => message.role === "user");
  return (
    <section
      data-cma-turn={turn.turn_sequence}
      aria-label={
        linked
          ? "Linked request"
          : latest
            ? "Latest request"
            : "Earlier request"
      }
      className="scroll-m-4 space-y-6"
    >
      {linked && requestIndex === -1 && (
        <p className="text-11 text-nyx-secondary-400">From your chat</p>
      )}
      {messages.map((message, index) => (
        <CmaMessage
          key={message.sequence ?? message.id ?? index}
          role={message.role}
          text={cmaPublicText(message.content)}
          time={message.created_at}
          assistantName={assistantName}
          linked={linked && index === requestIndex}
        />
      ))}
      <div className="space-y-3 pl-9 sm:pl-10">
        {status === "waiting_approval" && (
          <p className="text-12 text-text-tertiary">
            This request is waiting for confirmation before it can continue.
          </p>
        )}
        {status === "interrupted" && (
          <p className="text-12 text-text-tertiary">
            This request was interrupted. Its available replies are kept above.
          </p>
        )}
        {!messages.some((message) => message.role === "assistant") && (
          <p className="text-12 text-text-tertiary">
            {turn.status === "active"
              ? "No assistant reply yet."
              : "No assistant reply was published."}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <CmaStatus status={status} />
          <CmaConversationDetails turn={turn} events={events} status={status} />
        </div>
      </div>
    </section>
  );
}
