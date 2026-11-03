import {
  useCallback,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { ChevronLeft } from "lucide-react";
import {
  coverGroupedContent,
  GroupedDetailContext,
} from "@/lib/grouped-detail";
import { GroupedFormBar } from "./grouped-form";
import { Button } from "./button";
import { GroupedDraftOwnerContext, useGroupedDraftGuard } from "@/lib/grouped-drafts";

/** A retained sheet page: navigation changes visibility without discarding its forms. */
export function GroupedDetail({
  open,
  title,
  backLabel,
  onBack,
  children,
  level: requestedLevel,
}: {
  readonly open: boolean;
  readonly title: string;
  readonly backLabel: string;
  readonly onBack: () => void;
  readonly children: ReactNode;
  readonly level?: number;
}) {
  const context = useContext(GroupedDetailContext);
  const parentOwner = useContext(GroupedDraftOwnerContext);
  const id = useId();
  const owner = `${parentOwner}/${id}`;
  const guard = useGroupedDraftGuard();
  const level = requestedLevel ?? (context.level ?? 0) + 1;
  const page = useRef<HTMLElement>(null);
  const [fallback, setFallback] = useState<HTMLElement | null>(null);
  const attach = useCallback((element: HTMLSpanElement | null) => {
    if (element)
      setFallback(
        element.closest<HTMLElement>('[role="dialog"]') ?? document.body,
      );
  }, []);
  const container = context.container ?? fallback;
  const [visited, setVisited] = useState(open);
  if (open && !visited) setVisited(true);
  const active = open && context.visible;
  useEffect(() => {
    if (!active || !page.current) return;
    const previous =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const release: (() => void)[] = [];
    const boundary = container;
    let current: HTMLElement = page.current;
    while (current.parentElement && current !== boundary) {
      const parent = current.parentElement;
      for (const sibling of parent.children) {
        if (sibling === current || !(sibling instanceof HTMLElement)) continue;
        // Retained portals may mount in a different order from their navigation depth.
        if (
          sibling.matches("[data-grouped-detail], [data-operation-view]") &&
          Number(sibling.getAttribute("data-grouped-level")) > level
        )
          continue;
        release.push(coverGroupedContent(sibling));
      }
      if (parent === boundary) break;
      current = parent;
    }
    if (!page.current.inert)
      page.current
        .querySelector<HTMLElement>("h2")
        ?.focus({ preventScroll: true });
    return () => {
      release.forEach((restore) => restore());
      if (previous?.isConnected && !previous.closest("[hidden], [inert]"))
        previous.focus({ preventScroll: true });
    };
  }, [active, container, level]);
  const content = visited ? (
    <section
      ref={page}
      hidden={!active}
      style={{ display: active ? undefined : "none", zIndex: 10 + level }}
      aria-label={title}
      data-grouped-detail=""
      data-grouped-level={level}
      className={`${container === document.body ? "fixed" : "absolute"} inset-0 z-10 flex flex-col bg-background p-4`}
    >
      <div className="shrink-0 pb-4">
        <Button variant="ghost" onClick={async () => { if (!guard || await guard.confirmDiscard(owner)) onBack(); }}>
          <ChevronLeft aria-hidden />
          {backLabel}
        </Button>
        <h2
          tabIndex={-1}
          className="mt-3 break-words px-4 text-15 font-semibold outline-none"
        >
          {title}
        </h2>
      </div>
      <div className="assistant-scrollbar min-h-0 flex-1 space-y-6 overflow-y-auto">
        <GroupedDetailContext.Provider
          value={{ container, visible: active, level }}
        >
          <GroupedDraftOwnerContext.Provider value={owner}>
            {children}
          </GroupedDraftOwnerContext.Provider>
        </GroupedDetailContext.Provider>
      </div>
      <GroupedFormBar />
    </section>
  ) : null;
  return (
    <>
      <span ref={attach} hidden />
      {container ? createPortal(content, container) : content}
    </>
  );
}
