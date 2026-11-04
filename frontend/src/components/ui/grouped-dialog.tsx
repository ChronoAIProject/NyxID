import type { ComponentProps, ReactNode } from "react";
import { Dialog, DialogContent, DialogClose } from "./dialog";
import { Sheet, SheetContent } from "./sheet";
import { Button } from "./button";
import { GroupedDraftBoundary, GroupedDraftDialog } from "./grouped-drafts";
import { useGroupedDraft, useGroupedDraftGuard } from "@/lib/grouped-drafts";
import { GroupedFormContext } from "@/lib/grouped-form";
import { cn } from "@/lib/utils";

function GuardedDialog({
  onOpenChange,
  ...props
}: ComponentProps<typeof Dialog>) {
  const guard = useGroupedDraftGuard();
  return (
    <Dialog
      {...props}
      onOpenChange={(open) => {
        if (open) onOpenChange?.(true);
        else
          void guard?.confirmDiscard().then((discard) => {
            if (discard) onOpenChange?.(false);
          });
      }}
    />
  );
}

export function GroupedDialog(props: ComponentProps<typeof Dialog>) {
  return (
    <GroupedDraftBoundary navigation={Boolean(props.open)}>
      <GroupedFormContext.Provider value={null}>
        <GuardedDialog {...props} />
      </GroupedFormContext.Provider>
    </GroupedDraftBoundary>
  );
}

export function GroupedDialogContent({
  children,
  className,
  ...props
}: ComponentProps<typeof DialogContent>) {
  return (
    <DialogContent {...props} className={cn("bg-background", className)}>
      {children}
      <GroupedDraftDialog />
    </DialogContent>
  );
}

export function GroupedDialogCancel(props: ComponentProps<typeof Button>) {
  return (
    <DialogClose asChild>
      <Button type="button" variant="ghost" {...props} />
    </DialogClose>
  );
}

function GuardedSheet({
  onOpenChange,
  ...props
}: ComponentProps<typeof Sheet>) {
  const guard = useGroupedDraftGuard();
  return (
    <Sheet
      {...props}
      onOpenChange={(open) => {
        if (open) onOpenChange?.(true);
        else
          void guard?.confirmDiscard().then((discard) => {
            if (discard) onOpenChange?.(false);
          });
      }}
    />
  );
}

export function GroupedSheet(props: ComponentProps<typeof Sheet>) {
  return (
    <GroupedDraftBoundary navigation={Boolean(props.open)}>
      <GroupedFormContext.Provider value={null}>
        <GuardedSheet {...props} />
      </GroupedFormContext.Provider>
    </GroupedDraftBoundary>
  );
}

export function GroupedSheetContent({
  children,
  className,
  ...props
}: ComponentProps<typeof SheetContent>) {
  return (
    <SheetContent {...props} className={cn("bg-background", className)}>
      {children}
      <GroupedDraftDialog />
    </SheetContent>
  );
}

export function GroupedPage({ children }: { readonly children: ReactNode }) {
  return (
    <GroupedDraftBoundary navigation>
      {children}
      <GroupedDraftDialog />
    </GroupedDraftBoundary>
  );
}

export function GroupedDecisionDraft({
  dirty,
  pending,
  discard,
}: {
  readonly dirty: boolean;
  readonly pending?: boolean;
  readonly discard: () => void;
}) {
  useGroupedDraft({ dirty, pending, discard });
  return null;
}
