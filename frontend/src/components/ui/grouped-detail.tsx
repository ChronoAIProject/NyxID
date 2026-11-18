import { SectionInfo } from "./section-info";
import { drawerSectionInfo, DrawerSectionInfoContext } from "@/lib/drawer-section-info";
import { CompactChrome, CompactScroll } from "./compact-title";
import { DrawerSearchRevealContext, useDrawerSearching, useDrawerSearchPage } from "@/lib/drawer-search";
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
  compactHeader = false,
  showBack = true,
  forceMount = false,
  draftOwner,
  searchPage,
  title,
  info,
  onReveal,
  backLabel,
  onBack,
  children,
  level: requestedLevel,
}: {
  readonly open: boolean;
  readonly compactHeader?: boolean;
  readonly showBack?: boolean;
  readonly forceMount?: boolean;
  readonly draftOwner?: string;
  readonly searchPage?: string;
  readonly title: string;
  readonly info?: string;
  readonly onReveal?: () => void;
  readonly backLabel: string;
  readonly onBack: () => void;
  readonly children: ReactNode;
  readonly level?: number;
}) {
  const drawer = useContext(DrawerSectionInfoContext);
  const help = info ?? (drawer ? drawerSectionInfo(title) : undefined);
  const context = useContext(GroupedDetailContext);
  const parentOwner = useContext(GroupedDraftOwnerContext);
  const id = useId();
  const owner = `${parentOwner}/${draftOwner ?? id}`;
  const parentSearch = useContext(DrawerSearchRevealContext);
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
  const searching = useDrawerSearching();
  useDrawerSearchPage(searchPage ? { id: searchPage, open, level, leave: async () => {
    if (guard && !await guard.confirmDiscard(owner)) return false;
    onBack(); return true;
  } } : null);
  const active = open && context.visible && !searching;
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
    if (!page.current.inert && !document.activeElement?.matches('[role="searchbox"]'))
      page.current
        .querySelector<HTMLElement>("h2")
        ?.focus({ preventScroll: true });
    const activePage = page.current;
    return () => {
      release.forEach((restore) => restore());
      if (!activePage.contains(document.activeElement)) return;
      const trigger = searchPage ? [...(container?.querySelectorAll<HTMLElement>("[data-drawer-page-trigger]") ?? [])]
        .find((element) => element.dataset.drawerPageTrigger === searchPage && !element.closest("[hidden], [inert]")) : null;
      // Focusing an inactive automatic tab would select it and navigate again.
      const previousAvailable = previous?.isConnected && previous !== document.body &&
        !previous.closest("[hidden], [inert]") && !previous.matches('[role="tab"][aria-selected="false"]');
      const destination = trigger ?? (previousAvailable ? previous : null);
      destination?.focus({ preventScroll: true });
    };
  }, [active, container, level, searchPage]);
  const content = visited || forceMount ? (
    <section
      ref={page}
      hidden={!active}
      style={{ display: active ? undefined : "none", zIndex: 10 + level }}
      aria-label={title}
      data-grouped-detail=""
      data-grouped-section={searchPage}
      data-grouped-level={level}
      className={`${container === document.body ? "fixed" : "absolute"} inset-0 z-10 flex flex-col bg-background p-4`}
    >
      <CompactChrome enabled={compactHeader && active} className={`shrink-0 pb-4${compactHeader ? " pr-6 [[data-compact=true]]:pb-2" : ""}`}>
        {showBack && <Button variant="ghost" onClick={async () => { if (!guard || await guard.confirmDiscard(owner)) onBack(); }}>
          <ChevronLeft aria-hidden />
          {backLabel}
        </Button>}
        <h2
          aria-label={title}
          tabIndex={-1}
          className={`mt-3 flex items-center gap-1 break-words px-4 type-card-title outline-none${compactHeader ? " [[data-compact=true]_&]:mt-1 [[data-compact=true]_&]:text-15" : ""}`}
        >
          <span>{title}</span>{help && <SectionInfo title={title}>{help}</SectionInfo>}
        </h2>
      </CompactChrome>
      <CompactScroll active={active} level={level} className="assistant-scrollbar flex-1 space-y-6">
        <GroupedDetailContext.Provider
          value={{ container, visible: active, level }}
        >
          <GroupedDraftOwnerContext.Provider value={owner}>
            <DrawerSearchRevealContext value={searchPage ? { pages: [...(parentSearch?.pages ?? []), searchPage], reveal: () => { parentSearch?.reveal?.(); onReveal?.(); } } : null}>{children}</DrawerSearchRevealContext>
          </GroupedDraftOwnerContext.Provider>
        </GroupedDetailContext.Provider>
      </CompactScroll>
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
