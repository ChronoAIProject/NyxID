import { getFormattingLocale } from "@/lib/formatting-locale";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
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
  const { t } = useUiTranslation();
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
    <div className="rounded border border-hairline px-3 py-2 text-11">
      <button type="button" aria-expanded={open} className="underline" onClick={() => setOpen(true)}>{t("Working context")}</button>
      <Sheet open={open} onOpenChange={(next) => { setOpen(next); setConfirming(false); }}>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>{t("Working context")}</SheetTitle>
          </SheetHeader>
          <div className="mt-4 space-y-2 text-text-tertiary">
            {query.isPending ? <p>{t("Loading working context…")}</p> : null}
            {query.isError ? (
              <p role="alert" className="text-destructive">{t("Unable to load the working context.")}{" "}
                <button type="button" className="underline" onClick={() => void query.refetch()}>{t("Try again")}</button>
              </p>
            ) : null}
            {context ? (
              <>
                <div>{t("Goal: ")}{checkpoint?.goal ?? t("No goal recorded")}</div>
                <div>{t("Last checkpoint:")}{" "}
                  {checkpoint
                    ? t("{{value}} · {{value1}}", { value: checkpoint.status === "completed" ? t("completed") : t("failed ({{code}})", { code: checkpoint.error_code ?? t("unknown") }), value1: new Date(checkpoint.updated_at).toLocaleString(getFormattingLocale()) })
                    : t("None yet")}
                </div>
                <div>{t("Last successful checkpoint:")}{" "}
                  {checkpoint?.last_successful_at
                    ? new Date(checkpoint.last_successful_at).toLocaleString(getFormattingLocale())
                    : t("None")}
                </div>
                <div>{t("Completed since then: ")}{labels(checkpoint?.completed_actions)}</div>
                <div>{t("Failed or interrupted: ")}{labels(checkpoint?.failed_actions)}</div>
                <div>{t("Running now: ")}{context.pending_actions.join(", ") || t("None")}</div>
                <div>{t("Live context: ")}{context.binding_state === "bound" ? t("kept") : t("starts fresh next turn")}</div>
                {context.reset_reason ? <div>{t("Last reset: ")}{contextResetCopy(context.reset_reason)}</div> : null}
                {reset.isError ? (
                  <p role="alert" className="text-destructive">
                    {context.active_turn_id
                      ? t("A turn is running. Stop it or wait, then reset.")
                      : t("Unable to reset the working context. Try again.")}
                  </p>
                ) : null}
                {confirming ? (
                  <div role="group" aria-label={t("Confirm reset")} className="space-y-1 rounded border border-hairline p-2">
                    <p>{t("Reset clears the live context and the checkpoint. The transcript stays; the next turn starts without a recap.")}</p>
                    <button type="button" className="underline" disabled={reset.isPending} onClick={() => reset.mutate()}>{t("Confirm reset")}</button>{" "}
                    <button type="button" className="underline" onClick={() => setConfirming(false)}>{t("Cancel")}</button>
                  </div>
                ) : (
                  <button type="button" className="underline" disabled={Boolean(context.active_turn_id)} onClick={() => setConfirming(true)}>{t("Reset context")}</button>
                )}{" "}
                {onManageMemory && context.agent_id ? (
                  <button type="button" className="underline" onClick={() => onManageMemory(context.agent_id!)}>{t("Manage memory notes")}</button>
                ) : null}
              </>
            ) : null}
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
}
