import { useCallback, useEffect, useRef } from "react";
import { flushSync } from "react-dom";

export function useServiceCardTransition() {
  const active = useRef<ViewTransition | null>(null);
  useEffect(
    () => () => {
      active.current?.skipTransition();
      active.current = null;
      document.documentElement.classList.remove("service-card-transition");
    },
    [],
  );

  return useCallback(
    (
      change: () => void,
      expandedCard?: HTMLElement | null,
      stickyToolbar?: HTMLElement | null,
    ) => {
      active.current?.skipTransition();
      active.current = null;
      const reducedMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)",
      ).matches;
      const revealCard = () => {
        if (expandedCard?.isConnected) {
          const scroller = expandedCard.closest("main");
          if (scroller && stickyToolbar?.isConnected) {
            const inset =
              Number.parseFloat(getComputedStyle(stickyToolbar).top) || 0;
            const padding =
              Number.parseFloat(getComputedStyle(scroller).paddingTop) || 0;
            scroller.scrollTo({
              top: Math.max(
                0,
                scroller.scrollTop +
                  expandedCard.getBoundingClientRect().top -
                  scroller.getBoundingClientRect().top -
                  scroller.clientTop -
                  padding -
                  (
                    stickyToolbar.firstElementChild ?? stickyToolbar
                  ).getBoundingClientRect().height -
                  inset -
                  32,
              ),
              behavior: reducedMotion ? "instant" : "smooth",
            });
            return;
          }
          expandedCard.scrollIntoView({
            behavior: reducedMotion ? "instant" : "smooth",
            block: "start",
            inline: "nearest",
          });
        }
      };
      if (!document.startViewTransition || reducedMotion) {
        document.documentElement.classList.remove("service-card-transition");
        flushSync(change);
        revealCard();
        return;
      }
      document.documentElement.classList.add("service-card-transition");
      const transition = document.startViewTransition(() => flushSync(change));
      active.current = transition;
      // A superseded transition must not remove the next transition's styles.
      void transition.finished
        .catch(() => undefined)
        .then(() => {
          if (active.current === transition) {
            active.current = null;
            document.documentElement.classList.remove(
              "service-card-transition",
            );
            revealCard();
          }
        });
    },
    [],
  );
}
