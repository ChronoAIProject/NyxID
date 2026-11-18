import { useEffect, useState, type RefObject } from "react";

export interface SearchRevealTarget {
  readonly id: string;
  readonly target?: () => HTMLElement | null;
}

/** Wait for a retained or lazy page, then use the drawer's shared focus/highlight treatment. */
export function useSearchReveal(
  container: HTMLElement | null,
  searching = false,
  fallback?: RefObject<HTMLElement | null>,
) {
  const [pending, setPending] = useState<SearchRevealTarget>();
  const [highlighted, setHighlighted] = useState<HTMLElement>();
  useEffect(() => {
    if (!pending || !container || searching) return;
    let frame = 0,
      attempts = 0;
    const focus = () => {
      const target =
        pending.target?.() ??
        [
          ...container.querySelectorAll<HTMLElement>(
            "[data-drawer-search-target]",
          ),
        ].find((element) => element.dataset.drawerSearchTarget === pending.id);
      if (target?.isConnected && !target.closest("[hidden],[inert]")) {
        target.scrollIntoView?.({ block: "center", behavior: "instant" });
        if (!target.hasAttribute("tabindex")) target.tabIndex = -1;
        target.focus({ preventScroll: true });
        setHighlighted(target);
        setPending(undefined);
      } else if (++attempts < 120) frame = requestAnimationFrame(focus);
      else {
        fallback?.current?.focus();
        setPending(undefined);
      }
    };
    frame = requestAnimationFrame(focus);
    return () => cancelAnimationFrame(frame);
  }, [pending, container, searching, fallback]);
  useEffect(() => {
    if (!highlighted) return;
    highlighted.setAttribute("data-search-highlight", "");
    const timer = setTimeout(() => setHighlighted(undefined), 1600);
    return () => {
      clearTimeout(timer);
      highlighted.removeAttribute("data-search-highlight");
    };
  }, [highlighted]);
  return setPending;
}
