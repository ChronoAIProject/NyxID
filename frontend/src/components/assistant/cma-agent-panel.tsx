import { lazy, Suspense, useRef, useState } from "react";
import {
  ArrowLeft,
  ClipboardList,
  Maximize2,
  Minimize2,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { cn } from "@/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

const CmaAgentView = lazy(() =>
  import("./cma-agent-view").then((module) => ({
    default: module.CmaAgentView,
  })),
);

export function CmaAgentPanel({
  agentId,
  responseId,
}: {
  readonly agentId: string;
  readonly responseId?: string;
}) {
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const focusComposer = useRef(false);

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <SheetTrigger asChild>
            <Button
              ref={trigger}
              variant="ghost"
              size="icon"
              aria-label="View execution"
              className="shrink-0 text-foreground hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card"
            >
              <ClipboardList aria-hidden="true" />
            </Button>
          </SheetTrigger>
        </TooltipTrigger>
        <TooltipContent>View execution</TooltipContent>
      </Tooltip>
      <SheetContent
        showCloseButton={false}
        className={cn(
          "flex min-h-0 w-full flex-col gap-4 overflow-hidden px-4 pb-4 pt-5 sm:w-4/5 sm:max-w-2xl sm:px-6",
          expanded && "sm:w-[calc(100vw-5rem)] sm:max-w-5xl",
        )}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          const composer = focusComposer.current
            ? trigger.current
                ?.closest("main")
                ?.querySelector<HTMLElement>(
                  '[data-composer-textbox]:not([disabled]):not([contenteditable="false"]):not([aria-disabled="true"])',
                )
            : null;
          (composer ?? trigger.current)?.focus({ preventScroll: true });
          focusComposer.current = false;
        }}
      >
        <div className="flex shrink-0 items-center justify-between gap-3">
          <SheetHeader className="min-w-0 text-left">
            <SheetTitle className="type-secondary text-text-tertiary">
              Cloud assistant
            </SheetTitle>
            <SheetDescription className="sr-only">
              View this assistant’s conversation and shared desktop without
              leaving your NyxBot chat.
            </SheetDescription>
          </SheetHeader>
          <div className="flex shrink-0 items-center gap-1">
            <div className="hidden sm:block">
              <Button
                variant="ghost"
                size="icon"
                aria-label={expanded ? "Restore panel width" : "Expand panel"}
                title={expanded ? "Restore panel width" : "Expand panel"}
                aria-pressed={expanded}
                onClick={() => setExpanded((value) => !value)}
              >
                {expanded ? (
                  <Minimize2 aria-hidden />
                ) : (
                  <Maximize2 aria-hidden />
                )}
              </Button>
            </div>
            <SheetClose asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label="Close"
                title="Close"
              >
                <X aria-hidden />
              </Button>
            </SheetClose>
          </div>
        </div>
        <div className="min-h-0 flex-1">
          {open && (
            <Suspense
              fallback={
                <p role="status" className="py-6 text-13 text-text-tertiary">
                  Loading assistant…
                </p>
              }
            >
              <CmaAgentView
                key={`${agentId}:${responseId ?? ""}`}
                agentId={agentId}
                responseId={responseId}
                presentation="panel"
                footerAction={
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      focusComposer.current = true;
                      setOpen(false);
                    }}
                  >
                    <ArrowLeft aria-hidden /> Back to chat
                  </Button>
                }
              />
            </Suspense>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
