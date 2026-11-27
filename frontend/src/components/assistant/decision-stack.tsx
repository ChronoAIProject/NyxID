import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, ChevronLeft, ChevronRight, ChevronUp, Layers3 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useCardTranslation } from "./card-language";
import { DecisionCardPresentation, DecisionCardActionTarget, DecisionCardInteractionBlocked, DecisionCardActive } from "./decision-card-presentation";
import { nextActiveDecision } from "@/lib/assistant/pending-decisions";

export interface DecisionStackItem {
  readonly id: string;
  readonly content: ReactNode;
}

function StackCard({ item, active, guarded }: { readonly item: DecisionStackItem; readonly active: boolean; readonly guarded: boolean }) {
  const [footer, setFooter] = useState<HTMLDivElement | null>(null);
  return <div hidden={!active} inert={!active} data-decision-id={item.id} className={active ? "flex min-h-0 flex-col" : "hidden"}>
    <DecisionCardActive value={active}>
      <DecisionCardInteractionBlocked value={guarded}>
        <DecisionCardActionTarget value={footer}>
          <fieldset disabled={guarded} className="flex min-h-0 min-w-0 flex-col border-0 p-0" onClickCapture={(event) => {
            if (guarded) { event.preventDefault(); event.stopPropagation(); }
          }}>
            <div data-decision-card-body="" className="min-h-0 max-h-[min(30dvh,14rem)] overflow-y-auto overscroll-contain md:max-h-[min(36dvh,20rem)]">{item.content}</div>
            <div ref={setFooter} data-decision-card-actions="" className="shrink-0 border-t border-border/50 bg-card px-4 py-3 empty:hidden" />
          </fieldset>
        </DecisionCardActionTarget>
      </DecisionCardInteractionBlocked>
    </DecisionCardActive>
  </div>;
}

/** One decision surface; back layers are decorative, never hidden actions. */
export function DecisionStack({ items, selectedId, revealKey, onEmptyFocus }: {
  readonly items: readonly DecisionStackItem[];
  readonly selectedId?: string;
  readonly revealKey?: string;
  readonly onEmptyFocus: () => void;
}) {
  const { t: translateUi, language } = useCardTranslation();
  const contentId = useId();
  const titleId = useId();
  const linkKey = revealKey ?? selectedId;
  const heading = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLElement>(null);
  const previousPager = useRef<HTMLButtonElement>(null);
  const nextPager = useRef<HTMLButtonElement>(null);
  const focused = useRef<HTMLElement | null>(null);
  const ids = items.map((item) => item.id);
  const signature = ids.join("\0");
  const [selection, setSelection] = useState({ id: selectedId ?? items[0]?.id, link: linkKey, collapsed: false, ids, signature });
  if (selection.signature !== signature || selection.link !== linkKey) {
    const linked = selectedId && ids.includes(selectedId) &&
      (selection.link !== linkKey || !selection.ids.includes(selectedId));
    setSelection({
      ...selection, ids, signature, link: linkKey,
      id: linked ? selectedId : nextActiveDecision(selection.id, selection.ids, ids),
      collapsed: linked ? false : selection.collapsed,
    });
  }
  const [readyId, setReadyId] = useState(selection.id);
  const index = Math.max(0, ids.indexOf(selection.id ?? ""));
  const active = items[index]?.id;
  const collapsed = selection.collapsed;
  const guarded = active !== readyId;
  const previousActive = useRef(active);
  const revealedLink = useRef<string | undefined>(undefined);

  useEffect(() => {
    if (active === readyId) return;
    // A second click on the old Allow button must never decide the next card.
    const timer = setTimeout(() => setReadyId(active), 750);
    return () => clearTimeout(timer);
  }, [active, readyId]);
  useLayoutEffect(() => {
    const changed = previousActive.current !== active;
    const focusLost = focused.current && (!focused.current.isConnected || focused.current.closest("[hidden]"));
    const linked = active === selectedId && Boolean(linkKey) && revealedLink.current !== linkKey;
    const currentFocus = document.activeElement;
    const focusedElsewhere = currentFocus && currentFocus !== document.body && !root.current?.contains(currentFocus);
    const disabledPager = focused.current instanceof HTMLButtonElement && focused.current.disabled
      ? focused.current === nextPager.current ? previousPager.current : focused.current === previousPager.current ? nextPager.current : null
      : null;
    if (changed && focusLost) focused.current = null;
    if (linked || (changed && focusLost && !focusedElsewhere)) {
      if (active) heading.current?.focus();
      else onEmptyFocus();
    } else if (!focusedElsewhere && disabledPager && !disabledPager.disabled) {
      disabledPager.focus();
    }
    if (linked) revealedLink.current = linkKey;
    previousActive.current = active;
  }, [active, linkKey, selectedId, onEmptyFocus]);

  if (!items.length) return null;
  function select(next: number) {
    const id = items[next]!.id;
    setSelection({ id, link: linkKey, collapsed: false, ids, signature });
    setReadyId(id);
  }
  return (
    <section
      ref={root}
      lang={language}
      aria-labelledby={titleId}
      data-decision-stack=""
      className="flex min-h-0 shrink flex-col px-4 pb-2 sm:px-6"
      style={{ width: "calc(100% - var(--assistant-content-inset, 0px))" }}
      onFocusCapture={(event) => { focused.current = event.target as HTMLElement; }}
      onBlurCapture={(event) => {
        if (event.relatedTarget instanceof Node && !event.currentTarget.contains(event.relatedTarget)) focused.current = null;
      }}
    >
      <div className="mx-auto flex min-h-0 w-full max-w-[var(--chat-content-width,758px)] flex-col">
        <div className={items.length > 1 ? "relative flex min-h-0 flex-col pt-3" : "relative flex min-h-0 flex-col"}>
          {items.length > 2 ? <div aria-hidden="true" data-decision-stack-layer="" className="pointer-events-none absolute inset-x-4 top-0 h-12 rounded-xl border border-border bg-muted shadow-sm transition-transform duration-200 motion-reduce:transition-none" /> : null}
          {items.length > 1 ? <div aria-hidden="true" data-decision-stack-layer="" className="pointer-events-none absolute inset-x-2 top-1.5 h-12 rounded-xl border border-border bg-card shadow-sm transition-transform duration-200 motion-reduce:transition-none" /> : null}
          <div className="relative flex min-h-0 flex-col overflow-hidden rounded-xl border border-border/50 bg-card shadow-sm">
            <div ref={heading} tabIndex={-1} className="flex shrink-0 flex-wrap items-center gap-2 px-4 py-2 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-input-focus">
              <Layers3 aria-hidden="true" className="h-4 w-4 shrink-0 text-warning" />
              <p aria-hidden="true" className="min-w-0 flex-1 type-row-title">
                <span className="sm:hidden">{items.length === 1 ? translateUi("Needs your decision") : translateUi("{{count}} decisions", { count: items.length })}</span>
                <span className="hidden sm:inline">{items.length === 1 ? translateUi("Needs your decision") : translateUi("{{count}} requests need your decision", { count: items.length })}</span>
              </p>
              <div className="flex items-center gap-1">
                {items.length > 1 ? <>
                  <Button ref={previousPager} variant="ghost" size="icon" aria-label={translateUi("Previous decision")} aria-controls={contentId} disabled={index === 0} onClick={() => select(index - 1)}><ChevronLeft aria-hidden="true" className="h-4 w-4" /></Button>
                  <span className="whitespace-nowrap type-secondary text-muted-foreground">{translateUi("{{current}} out of {{total}}", { current: index + 1, total: items.length })}</span>
                  <Button ref={nextPager} variant="ghost" size="icon" aria-label={translateUi("Next decision")} aria-controls={contentId} disabled={index === items.length - 1} onClick={() => select(index + 1)}><ChevronRight aria-hidden="true" className="h-4 w-4" /></Button>
                </> : null}
                <Button variant="ghost" size="icon" aria-label={collapsed ? translateUi("Expand decisions") : translateUi("Collapse decisions")} aria-expanded={!collapsed} aria-controls={contentId} onClick={() => setSelection({ ...selection, id: active, collapsed: !collapsed })}>
                  {collapsed ? <ChevronUp aria-hidden="true" className="h-4 w-4" /> : <ChevronDown aria-hidden="true" className="h-4 w-4" />}
                </Button>
              </div>
            </div>
            <p id={titleId} role="status" className="sr-only">{items.length === 1 ? translateUi("Needs your decision") : translateUi("{{count}} requests need your decision", { count: items.length })}</p>
            <div id={contentId} hidden={collapsed} className={collapsed ? "hidden" : "flex min-h-0 flex-col border-t border-border/50"}>
              <DecisionCardPresentation value="pinned">
                {items.map((item) => <StackCard key={item.id} item={item} active={item.id === active} guarded={guarded} />)}
              </DecisionCardPresentation>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
