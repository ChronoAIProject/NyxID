import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import {
  animate,
  useReducedMotion,
  type AnimationPlaybackControls,
  type TargetAndTransition,
  type Transition,
} from "motion/react";

/*
 * Expanding a service or pool card runs as three phases instead of one
 * snapshot morph:
 *   1. close  – an open card's body folds away (ease-in, short);
 *   2. reflow – the card spans the row and its neighbours slide into place,
 *               while the page scrolls the card under the filters;
 *   3. reveal – the new body unfolds once the reflow has landed.
 * The body grows only after the reflow so it never stretches inside the
 * card's in-flight layout projection.
 */
export const MOVE = [0.2, 0.8, 0.2, 1] as const;
const EXIT = [0.4, 0, 1, 1] as const;
export const REFLOW: Transition = { duration: 0.26, ease: MOVE };
export const REVEAL_DELAY = 0.24;
const REVEAL_DURATION = 0.3;

/** Height-and-fade reveal shared by card bodies and expanded table rows. */
export function useRevealMotion(delay = 0) {
  const reduce = useReducedMotion();
  const instant: Transition = { duration: 0 };
  const animate: TargetAndTransition = {
    height: "auto",
    opacity: 1,
    transition: reduce
      ? instant
      : {
          height: { delay, duration: REVEAL_DURATION, ease: MOVE },
          opacity: { delay: delay + 0.06, duration: 0.2, ease: "easeOut" },
        },
  };
  const exit: TargetAndTransition = {
    height: 0,
    opacity: 0,
    transition: reduce
      ? instant
      : {
          height: { duration: 0.2, ease: EXIT },
          opacity: { duration: 0.14, ease: "easeIn" },
        },
  };
  return { initial: { height: 0, opacity: 0 }, animate, exit };
}

function scrollTarget(
  card: HTMLElement,
  scroller: HTMLElement,
  toolbar: HTMLElement | null | undefined,
) {
  const grid = card.parentElement;
  // offsetTop ignores the layout transform the card is animating with.
  const cardTop =
    grid && card.offsetParent === grid.offsetParent
      ? grid.getBoundingClientRect().top + card.offsetTop - grid.offsetTop
      : card.getBoundingClientRect().top;
  const position =
    scroller.scrollTop +
    cardTop -
    scroller.getBoundingClientRect().top -
    scroller.clientTop;
  if (!toolbar?.isConnected)
    return (
      position -
      (Number.parseFloat(getComputedStyle(card).scrollMarginTop) || 0)
    );
  const inset = Number.parseFloat(getComputedStyle(toolbar).top) || 0;
  const padding = Number.parseFloat(getComputedStyle(scroller).paddingTop) || 0;
  const filters = (toolbar.firstElementChild ?? toolbar).getBoundingClientRect()
    .height;
  return Math.max(0, position - padding - filters - inset - 32);
}

const USER_SCROLL = ["wheel", "touchstart", "keydown", "pointerdown"] as const;

/** Scrolls the page in step with the reflow and the body reveal. */
function revealCard(
  card: HTMLElement,
  toolbar: HTMLElement | null | undefined,
  reduce: boolean,
) {
  const scroller = card.closest("main");
  if (!scroller) {
    card.scrollIntoView({
      behavior: reduce ? "instant" : "smooth",
      block: "start",
      inline: "nearest",
    });
    return () => undefined;
  }
  const top = scrollTarget(card, scroller, toolbar);
  if (reduce) {
    // Wait for the body to mount so the target is reachable.
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => scroller.scrollTo({ top }));
    });
    return () => cancelAnimationFrame(frame);
  }
  // Runs through the reveal: near the bottom of the page the target only
  // becomes reachable as the body grows, and scrollTop clamps until then.
  const controls: AnimationPlaybackControls = animate(scroller.scrollTop, top, {
    duration: REVEAL_DELAY + REVEAL_DURATION,
    ease: MOVE,
    onUpdate: (value) => {
      scroller.scrollTop = value;
    },
  });
  const stop = () => controls.stop();
  for (const type of USER_SCROLL)
    scroller.addEventListener(type, stop, { passive: true, once: true });
  void controls.finished.then(() => {
    for (const type of USER_SCROLL) scroller.removeEventListener(type, stop);
  });
  return stop;
}

/**
 * Sequences single-card expansion: an open body closes before the grid
 * reflows, and a newly opened card is scrolled into view during the reflow.
 */
export function useCardSequence({
  expanded,
  commit,
  isRendered,
  toolbarRef,
}: {
  readonly expanded: string | null;
  readonly commit: (next: string | null) => void;
  readonly isRendered: (id: string) => boolean;
  readonly toolbarRef?: RefObject<HTMLElement | null>;
}) {
  const reduce = useReducedMotion() ?? false;
  const [closing, setClosing] = useState<{
    from: string;
    next: string | null;
  } | null>(null);
  // A stale record (the store changed underneath us) never hides a body.
  const active = closing?.from === expanded ? closing : null;
  const pendingReveal = useRef<{ id: string; card: HTMLElement } | null>(null);
  const stopScroll = useRef<() => void>(() => undefined);

  useLayoutEffect(() => {
    const pending = pendingReveal.current;
    if (!pending || pending.id !== expanded) return;
    pendingReveal.current = null;
    if (!pending.card.isConnected) return;
    stopScroll.current();
    stopScroll.current = revealCard(pending.card, toolbarRef?.current, reduce);
  }, [expanded, reduce, toolbarRef]);
  useEffect(() => () => stopScroll.current(), []);

  const request = useCallback(
    (next: string | null, card?: HTMLElement | null) => {
      pendingReveal.current = next && card ? { id: next, card } : null;
      if (expanded && isRendered(expanded))
        setClosing({ from: expanded, next });
      else commit(next);
    },
    [commit, expanded, isRendered],
  );
  const onClosed = useCallback(() => {
    if (!active) return;
    // Commit first: if the store renders before React state, the guard
    // above still keeps the closed body hidden.
    commit(active.next);
    setClosing(null);
  }, [active, commit]);

  return {
    request,
    onClosed,
    isOpen: (id: string) => expanded === id && active?.from !== id,
  };
}
