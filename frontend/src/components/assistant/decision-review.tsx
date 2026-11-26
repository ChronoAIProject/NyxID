import { useContext, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useCardTranslation } from "./card-language";
import { DecisionCardActions } from "./decision-card-actions";
import { DecisionCardActive, DecisionCardInteractionBlocked } from "./decision-card-presentation";

/** Long requests are reviewed in full before an Allow control is offered. */
export function DecisionReview({ title, summary, preview, children, actions, onDeny, denyLabel, busy = false, ready = true, reviewLabel }: {
  readonly title: string;
  readonly summary: ReactNode;
  readonly preview?: ReactNode;
  readonly children: ReactNode;
  readonly actions: ReactNode;
  readonly onDeny?: () => void;
  readonly denyLabel?: string;
  readonly busy?: boolean;
  readonly ready?: boolean;
  readonly reviewLabel?: string;
}) {
  const { t: translateUi, language } = useCardTranslation();
  const active = useContext(DecisionCardActive);
  const guarded = useContext(DecisionCardInteractionBlocked);
  const [open, setOpen] = useState(false);
  if (!active && open) setOpen(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  return <>
    <section lang={language} aria-label={title} className="space-y-2 p-4">
      <p className="type-card-title break-words">{title}</p>
      <div className="space-y-1 type-secondary text-muted-foreground">{summary}</div>
      {preview}
    </section>
    <DecisionCardActions>
      <div className="flex flex-wrap justify-end gap-2">
        {onDeny ? <Button variant="outline" disabled={busy || guarded || open} onClick={onDeny}>{denyLabel ?? translateUi("Deny")}</Button> : null}
        <Button ref={trigger} variant="primary" disabled={!ready || busy || guarded || open} onClick={() => setOpen(true)}>{reviewLabel ?? translateUi("Review full request")}</Button>
      </div>
    </DecisionCardActions>
    <Dialog open={open && active} onOpenChange={setOpen}>
      <DialogContent lang={language} scrollMode="body" aria-describedby={undefined}
        className="md:h-[min(85dvh,56rem)] md:max-w-3xl"
        onOpenAutoFocus={(event) => { event.preventDefault(); heading.current?.focus(); }}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          if (trigger.current?.isConnected && !trigger.current.closest("[hidden]")) trigger.current.focus();
        }}
      >
        <DialogHeader className="shrink-0 border-b border-border/50 pb-3 pr-10">
          <DialogTitle ref={heading} tabIndex={-1} className="break-words outline-none">{title}</DialogTitle>
        </DialogHeader>
        <DialogBody data-decision-review-body="" className="space-y-4 pb-4">{children}</DialogBody>
        <DialogFooter className="shrink-0 border-t border-border/50 md:shrink-0">
          <fieldset disabled={guarded} className="min-w-0 w-full border-0 p-0" data-decision-review-actions="">{actions}</fieldset>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </>;
}
