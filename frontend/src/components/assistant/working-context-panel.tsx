import { useContext, useState } from "react";
import { ChatHeaderCompactContext } from "./chat-header-context";
import { NotebookText } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from "@/components/ui/sheet";
import { useAssistantContext, useResetAssistantContext } from "@/hooks/use-assistant-context";
import { contextResetCopy } from "@/lib/assistant/nyxagent-transport";

function labels(actions: readonly { readonly label: string }[] | undefined): string {
  return actions?.length ? actions.map((action) => action.label).join(", ") : "None";
}

/** Inspect or reset what the assistant will resume from. `onReset` refreshes
 * the transcript so the reset note appears; `onManageMemory` opens the
 * agent's memory notes (only offered when the thread's agent is known). */
export function WorkingContextPanel({
  conversationId,
  onManageMemory,
  onReset,
}: {
  readonly conversationId: string;
  readonly onManageMemory?: (agentId: string) => void;
  readonly onReset?: () => void;
}) {
  const compact = useContext(ChatHeaderCompactContext);
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const query = useAssistantContext(conversationId, open);
  const reset = useResetAssistantContext(conversationId, () => {
    setConfirming(false);
    onReset?.();
  });
  const context = query.data;
  const checkpoint = context?.checkpoint;
  return (
    <Sheet open={open} onOpenChange={(next) => { setOpen(next); setConfirming(false); }}>
      <SheetTrigger asChild>
        <Button size="sm" variant="ghost" aria-label="Working context" title="Working context" className={compact ? "h-7 w-7 px-0" : undefined}>
          <NotebookText aria-hidden="true" />
          <span className={compact ? "sr-only" : undefined}>Working context</span>
        </Button>
      </SheetTrigger>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>Working context</SheetTitle>
        </SheetHeader>
        <div className="mt-4 space-y-2 text-11 text-text-tertiary">
          {query.isPending ? <p>Loading working context…</p> : null}
          {query.isError ? (
            <p role="alert" className="text-destructive">
              Unable to load the working context.{" "}
              <button type="button" className="underline" onClick={() => void query.refetch()}>
                Try again
              </button>
            </p>
          ) : null}
          {context ? (
            <>
              <div>Goal: {checkpoint?.goal ?? "No goal recorded"}</div>
              <div>
                Last checkpoint:{" "}
                {checkpoint
                  ? `${checkpoint.status === "completed" ? "completed" : `failed (${checkpoint.error_code ?? "unknown"})`} · ${new Date(checkpoint.updated_at).toLocaleString()}`
                  : "None yet"}
              </div>
              <div>
                Last successful checkpoint:{" "}
                {checkpoint?.last_successful_at
                  ? new Date(checkpoint.last_successful_at).toLocaleString()
                  : "None"}
              </div>
              <div>Completed since then: {labels(checkpoint?.completed_actions)}</div>
              <div>Failed or interrupted: {labels(checkpoint?.failed_actions)}</div>
              <div>Running now: {context.pending_actions.join(", ") || "None"}</div>
              <div>Live context: {context.binding_state === "bound" ? "kept" : "starts fresh next turn"}</div>
              {context.reset_reason ? <div>Last reset: {contextResetCopy(context.reset_reason)}</div> : null}
              {reset.isError ? (
                <p role="alert" className="text-destructive">
                  {context.active_turn_id
                    ? "A turn is running. Stop it or wait, then reset."
                    : "Unable to reset the working context. Try again."}
                </p>
              ) : null}
              {confirming ? (
                <div role="group" aria-label="Confirm reset" className="space-y-1 rounded border border-hairline p-2">
                  <p>Reset clears the live context and the checkpoint. The transcript stays; the next turn starts without a recap.</p>
                  <button type="button" className="underline" disabled={reset.isPending} onClick={() => reset.mutate()}>
                    Confirm reset
                  </button>{" "}
                  <button type="button" className="underline" onClick={() => setConfirming(false)}>
                    Cancel
                  </button>
                </div>
              ) : (
                <button type="button" className="underline" disabled={Boolean(context.active_turn_id)} onClick={() => setConfirming(true)}>
                  Reset context
                </button>
              )}{" "}
              {onManageMemory && context.agent_id ? (
                <button type="button" className="underline" onClick={() => onManageMemory(context.agent_id!)}>
                  Manage memory notes
                </button>
              ) : null}
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}
