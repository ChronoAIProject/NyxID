import { CompactTitleContext, type ScrollState, type ChromeSize, scrollOwner } from "@/lib/compact-title";
import { useCallback, useContext, useId, useLayoutEffect, useRef, useState, type ComponentProps, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/** Chrome and scroll insets change together, preserving every content position. */
export function CompactTitleProvider({ children }: { readonly children: ReactNode }) {
  const [scrolls, setScrolls] = useState<ReadonlyMap<string, ScrollState>>(new Map());
  const [sizes, setSizes] = useState<ReadonlyMap<string, ChromeSize>>(new Map());
  const scroll = useCallback((id: string, state: ScrollState | null) => setScrolls((old) => { const next = new Map(old); if (state) next.set(id, state); else next.delete(id); return next; }), []);
  const chrome = useCallback((id: string, size: ChromeSize | null) => setSizes((old) => { const next = new Map(old); if (size) next.set(id, size); else next.delete(id); return next; }), []);
  const active = [...scrolls.values()].filter((state) => state.active).sort((a, b) => b.level - a.level)[0];
  const compact = active?.compact ?? false;
  const expanded = [...sizes.values()].reduce((sum, size) => sum + size.expanded, 0);
  const current = [...sizes.values()].reduce((sum, size) => sum + size.current, 0);
  return <CompactTitleContext value={{ compact, gap: compact ? Math.max(0, expanded - current) : 0, scroll, chrome }}>{children}</CompactTitleContext>;
}
export function CompactChrome({ children, className, ...props }: ComponentProps<"div">) {
  const context = useContext(CompactTitleContext);
  const register = context?.chrome;
  const compact = context?.compact ?? false;
  const id = useId();
  const element = useRef<HTMLDivElement>(null);
  const expanded = useRef(0);
  const previous = useRef<{ compact: boolean; top: number } | null>(null);
  useLayoutEffect(() => {
    const node = element.current;
    if (!node) return;
    const top = node.getBoundingClientRect().top;
    const old = previous.current;
    if (old && old.compact !== compact && old.top !== top && !matchMedia("(prefers-reduced-motion: reduce)").matches)
      node.animate?.([{ transform: `translateY(${old.top - top}px)` }, { transform: "translateY(0)" }], { duration: 240, easing: "ease-out" });
    previous.current = { compact, top };
  });
  useLayoutEffect(() => {
    if (!register || !element.current) return;
    const measure = () => {
      const height = element.current!.getBoundingClientRect().height;
      if (!compact) expanded.current = height;
      register(id, { expanded: expanded.current, current: height });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element.current);
    return () => { observer.disconnect(); register(id, null); };
  }, [register, compact, id]);
  return <div {...props} ref={element} data-compact-chrome="" data-compact={compact} className={cn("shrink-0 bg-background", className)}>{children}</div>;
}
export function CompactScroll({ children, active = true, level = 0, className, ...props }: ComponentProps<"div"> & { readonly active?: boolean; readonly level?: number }) {
  const context = useContext(CompactTitleContext);
  const register = context?.scroll;
  const id = useId();
  const root = useRef<HTMLDivElement>(null), sentinel = useRef<HTMLSpanElement>(null), topSentinel = useRef<HTMLSpanElement>(null);
  const retainedTop = useRef(0);
  useLayoutEffect(() => { if (active && root.current) root.current.scrollTop = retainedTop.current; }, [active, context?.gap]);
  useLayoutEffect(() => {
    if (!register || !root.current || !sentinel.current || !topSentinel.current || !active) return;
    let compact = root.current.scrollTop > 24;
    const observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.target === sentinel.current && !entry.isIntersecting) compact = true;
        if (entry.target === topSentinel.current && entry.isIntersecting) compact = false;
      }
      retainedTop.current = root.current!.scrollTop;
      register(id, { active: true, level, compact });
    }, { root: root.current, threshold: 0 });
    register(id, { active: true, level, compact });
    observer.observe(sentinel.current);
    observer.observe(topSentinel.current);
    return () => { observer.disconnect(); register(id, null); };
  }, [register, active, level, id]);
  return <div {...props} ref={root} data-compact-scroll="" className={cn("relative min-h-0 overflow-y-auto [overflow-anchor:none]", className)}>
    <div aria-hidden data-compact-inset="" className="!m-0" style={{ height: context?.gap ?? 0 }}><span ref={sentinel} data-collapse-sentinel="" className="pointer-events-none absolute left-0 top-6 h-px w-px" /><span ref={topSentinel} data-expand-sentinel="" className="pointer-events-none absolute left-0 top-2 h-px w-px" /></div>
    {children}
  </div>;
}

/** An observer-driven desktop mirror; fixed positioning reserves no layout space. */
export function StickyTitle({ title, children, actions, column = false }: { readonly title: string; readonly children: ReactNode; readonly actions?: ReactNode; readonly column?: boolean }) {
  const heading = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false);
  const [bounds, setBounds] = useState({ top: 0, left: 0, width: 0 });
  useLayoutEffect(() => {
    const element = heading.current;
    if (!element) return;
    const root = scrollOwner(element);
    const desktop = matchMedia("(min-width: 768px)");
    let intersecting = true;
    const update = () => {
      const rect = element.getBoundingClientRect();
      setBounds({ top: root?.getBoundingClientRect().top ?? 0, left: rect.left, width: rect.width });
      const collapsed = desktop.matches && !intersecting;
      setCompact(collapsed);
      element.querySelectorAll<HTMLElement>("[data-title-actions]").forEach((actions) => { actions.inert = collapsed; });
    };
    const observer = new IntersectionObserver(([entry]) => { intersecting = entry?.isIntersecting ?? true; update(); }, { root });
    observer.observe(element.querySelector("h1") ?? element);
    const resize = new ResizeObserver(update); resize.observe(element); if (root) resize.observe(root);
    desktop.addEventListener("change", update); update();
    return () => { observer.disconnect(); resize.disconnect(); desktop.removeEventListener("change", update); };
  }, []);
  return <>
    <div data-sticky-title="" data-visible={compact} aria-hidden={!compact || !actions ? true : undefined} className={cn("compact-title-bar pointer-events-none fixed z-20 hidden items-center justify-between gap-3 border-b border-border/50 bg-background py-2 md:flex", column && "max-w-2xl", compact ? "pointer-events-auto opacity-100" : "invisible opacity-0")} style={bounds}>
      <span aria-hidden className="min-w-0 truncate text-13 font-semibold">{title}</span>
      {compact && actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </div>
    <div ref={heading} data-large-title="" className={column ? "max-w-2xl" : undefined}>{children}</div>
  </>;
}
