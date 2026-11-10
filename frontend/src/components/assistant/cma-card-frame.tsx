import {
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { ChevronDown, Maximize2, Minimize2 } from "lucide-react";
import { CmaGlyph } from "@/components/service-icons/cma";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export function CmaCardFrame({
  title,
  ariaLabel,
  controls,
  collapsed,
  onToggle,
  detailsId,
  scrollBody,
  operationId,
  controlSubject = "CMA session",
  children,
}: {
  readonly title: string;
  readonly ariaLabel: string;
  readonly controls?: ReactNode;
  readonly collapsed: boolean;
  readonly onToggle: () => void;
  readonly detailsId: string;
  readonly scrollBody: RefObject<HTMLDivElement | null>;
  readonly operationId?: string;
  readonly controlSubject?: string;
  readonly children: ReactNode;
}) {
  const toggleLabel = `${collapsed ? "Maximise" : "Minimise"} ${controlSubject}`;
  return (
    <section
      aria-label={ariaLabel}
      data-cma-operation={operationId}
      data-private="true"
      data-ph-no-capture
      className="flex max-h-[min(440px,40dvh)] min-h-0 w-full max-w-[500px] shrink-0 flex-col self-start gap-2 overflow-hidden rounded-xl border border-border/50 bg-card p-3 text-14 lg:max-h-[480px] lg:rounded-2xl lg:border-0 lg:shadow-sm"
    >
      <div className="flex shrink-0 flex-wrap items-center gap-x-2 gap-y-1">
        <CmaGlyph
          aria-hidden="true"
          className="h-4 w-4 shrink-0 text-muted-foreground"
        />
        <h2
          className="min-w-0 flex-1 basis-32 line-clamp-2 break-words type-card-title"
          title={title}
        >
          {title}
        </h2>
        <div
          className="ml-auto flex max-w-full shrink-0 flex-wrap items-center justify-end gap-1"
          role="group"
          aria-label={`${controlSubject} controls`}
        >
          {controls}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label={toggleLabel}
                aria-expanded={!collapsed}
                aria-controls={detailsId}
                onClick={onToggle}
                className="shrink-0 text-foreground hover:bg-overlay-strong focus-visible:ring-2 focus-visible:ring-foreground focus-visible:ring-offset-2 focus-visible:ring-offset-card"
              >
                {collapsed ? <Maximize2 /> : <Minimize2 />}
              </Button>
            </TooltipTrigger>
            <TooltipContent>{toggleLabel}</TooltipContent>
          </Tooltip>
        </div>
      </div>
      <div ref={scrollBody} className="min-h-0 space-y-2 overflow-y-auto">
        {children}
      </div>
    </section>
  );
}

export function CmaCardDetails({
  label,
  scrollBody,
  visible,
  children,
}: {
  readonly label: string;
  readonly scrollBody: RefObject<HTMLDivElement | null>;
  readonly visible: boolean;
  readonly children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const content = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const contentId = useId();
  useLayoutEffect(() => {
    if (
      !open ||
      !visible ||
      !content.current ||
      !button.current ||
      !scrollBody.current
    )
      return;
    const body = scrollBody.current;
    const viewport = body.getBoundingClientRect();
    const top = button.current.getBoundingClientRect().top;
    const bottom = content.current.getBoundingClientRect().bottom;
    // Keep metadata reachable within the card without scrolling the chat.
    body.scrollBy({
      top: Math.min(top - viewport.top, Math.max(0, bottom - viewport.bottom)),
      behavior: "instant",
    });
  }, [open, visible, scrollBody]);

  return (
    <div className="pt-1 text-13 text-text-tertiary">
      <Button
        ref={button}
        type="button"
        variant="ghost"
        size="sm"
        aria-expanded={open}
        aria-controls={contentId}
        onClick={() => setOpen((value) => !value)}
        className="w-full justify-between px-0 focus-visible:ring-2 focus-visible:ring-ring"
      >
        {label}
        <ChevronDown
          aria-hidden="true"
          className={cn(
            "transition-transform motion-reduce:transition-none",
            open && "rotate-180",
          )}
        />
      </Button>
      <div ref={content} id={contentId} hidden={!open}>
        <div className="space-y-2 px-1 pb-1 pt-2">{children}</div>
      </div>
    </div>
  );
}
