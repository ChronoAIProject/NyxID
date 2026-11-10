import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { cmaReadableLabel } from "@/lib/assistant/cma-display";
import type { CmaDisplayEvent } from "@/lib/assistant/cma-events";
import {
  cmaPublicText,
  type CmaTranscriptTurn,
} from "@/schemas/assistant-cma-agent";
import { CmaStatus, CmaTime } from "./cma-execution-content";

export function CmaConversationDetails({
  turn,
  events,
  status,
}: {
  readonly turn: CmaTranscriptTurn;
  readonly events: readonly CmaDisplayEvent[];
  readonly status: string;
}) {
  const activity = events.filter((event) => event.type !== "message");
  const technicalMessages = turn.messages.filter(
    (message) => !["user", "assistant"].includes(message.role),
  );
  const count = activity.length + technicalMessages.length;
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="type-button text-text-tertiary"
        >
          <ChevronRight aria-hidden />
          Execution details
          {count > 0 && (
            <span className="rounded bg-overlay px-1.5 type-badge">
              {count}
            </span>
          )}
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Execution details</DialogTitle>
          <DialogDescription>
            Activity for this request. Other requests stay in the conversation.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-wrap items-center gap-3">
          <CmaStatus status={status} />
          <CmaTime value={turn.started_at} />
        </div>
        <div className="divide-y divide-hairline">
          {technicalMessages.map((message, index) => (
            <article
              key={message.sequence ?? message.id ?? index}
              className="space-y-2 py-3 first:pt-0"
            >
              <h3 className="type-group-label">
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
                <h3 className="type-group-label">
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
        {!count && (
          <p className="text-12 text-text-tertiary">
            No execution details in the loaded activity.
          </p>
        )}
        <details className="border-t border-hairline pt-3 type-secondary text-text-tertiary">
          <summary className="cursor-pointer">Technical reference</summary>
          <p className="mt-3">Work reference: {turn.turn_sequence}</p>
        </details>
      </DialogContent>
    </Dialog>
  );
}
