import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
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
  useUiTranslation();
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
        <Button size={compact ? "icon" : "sm"} variant="ghost" aria-label={translateUi("Working context")} title={translateUi("Working context")}>
          <NotebookText aria-hidden="true" />
          <span className={compact ? "sr-only" : undefined}>{translateUi("Working context")}</span>
        </Button>
      </SheetTrigger>
      <SheetContent>
        <SheetHeader>
          <SheetTitle>{translateUi("Working context")}</SheetTitle>
        </SheetHeader>
        <div className="mt-4 space-y-2 text-13 text-text-tertiary">
          {query.isPending ? <p>{translateUi("Loading working context…")}</p> : null}
          {query.isError ? (
            <p role="alert" className="text-destructive">{translateUi("Unable to load the working context.")}{" "}
              <button type="button" className="underline" onClick={() => void query.refetch()}>{translateUi("Try again")}</button>
            </p>
          ) : null}
          {context ? (
            <>
              <div>{translateUi("Goal: ")}{checkpoint?.goal ?? translateUi("No goal recorded")}</div>
              <div>{translateUi("Last checkpoint:")}{" "}
                {checkpoint
                  ? `${checkpoint.status === "completed" ? "completed" : `failed (${checkpoint.error_code ?? "unknown"})`} · ${new Date(checkpoint.updated_at).toLocaleString(getFormattingLocale())}`
                  : translateUi("None yet")}
              </div>
              <div>{translateUi("Last successful checkpoint:")}{" "}
                {checkpoint?.last_successful_at
                  ? new Date(checkpoint.last_successful_at).toLocaleString(getFormattingLocale())
                  : translateUi("None")}
              </div>
              <div>{translateUi("Completed since then: ")}{labels(checkpoint?.completed_actions)}</div>
              <div>{translateUi("Failed or interrupted: ")}{labels(checkpoint?.failed_actions)}</div>
              <div>{translateUi("Running now: ")}{context.pending_actions.join(", ") || translateUi("None")}</div>
              <div>{translateUi("Live context: ")}{context.binding_state === "bound" ? translateUi("kept") : translateUi("starts fresh next turn")}</div>
              {context.reset_reason ? <div>{translateUi("Last reset: ")}{contextResetCopy(context.reset_reason)}</div> : null}
              {reset.isError ? (
                <p role="alert" className="text-destructive">
                  {context.active_turn_id
                    ? translateUi("A turn is running. Stop it or wait, then reset.")
                    : translateUi("Unable to reset the working context. Try again.")}
                </p>
              ) : null}
              {confirming ? (
                <div role="group" aria-label={translateUi("Confirm reset")} className="space-y-1 rounded border border-hairline p-2">
                  <p>{translateUi("Reset clears the live context and the checkpoint. The transcript stays; the next turn starts without a recap.")}</p>
                  <button type="button" className="underline" disabled={reset.isPending} onClick={() => reset.mutate()}>{translateUi("Confirm reset")}</button>{" "}
                  <button type="button" className="underline" onClick={() => setConfirming(false)}>{translateUi("Cancel")}</button>
                </div>
              ) : (
                <button type="button" className="underline" disabled={Boolean(context.active_turn_id)} onClick={() => setConfirming(true)}>{translateUi("Reset context")}</button>
              )}{" "}
              {onManageMemory && context.agent_id ? (
                <button type="button" className="underline" onClick={() => onManageMemory(context.agent_id!)}>{translateUi("Manage memory notes")}</button>
              ) : null}
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}
