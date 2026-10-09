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
  const margin = Number.parseFloat(getComputedStyle(card).scrollMarginTop) || 0;
  // Sticky offsets start at the scroller's padding edge. Use the same live
  // inset as the card header, including the compact filter surface's height.
  const padding = toolbar?.isConnected
    ? Number.parseFloat(getComputedStyle(scroller).paddingTop) || 0
    : 0;
  return Math.max(0, position - padding - margin);
}

const USER_SCROLL = ["wheel", "touchstart", "keydown", "pointerdown"] as const;

/** Gives a short final card enough page space to reach the sticky filters. */
function reserveScrollRoom(
  card: HTMLElement,
  toolbar: HTMLElement | null | undefined,
) {
  const scroller = card.closest("main");
  const grid = card.parentElement;
  if (!toolbar || !scroller || !grid) return () => undefined;
  const original = grid.style.paddingBottom;
  const padding = Number.parseFloat(getComputedStyle(grid).paddingBottom) || 0;
  let extra = 0;
  const update = () => {
    if (!card.isConnected) {
      observer.disconnect();
      grid.style.paddingBottom = original;
      return;
    }
    // scrollHeight is at least clientHeight, even when the content is shorter.
    // Measure the grid's natural end so a single filtered card works too.
    const end = Math.max(
      scroller.scrollHeight > scroller.clientHeight ? scroller.scrollHeight : 0,
      scroller.scrollTop +
        grid.getBoundingClientRect().bottom -
        scroller.getBoundingClientRect().top -
        scroller.clientTop +
        (Number.parseFloat(getComputedStyle(scroller).paddingBottom) || 0),
    );
    const needed = Math.ceil(
      Math.max(
        0,
        scrollTarget(card, scroller, toolbar) -
          (end - scroller.clientHeight - extra),
      ),
    );
    if (needed === extra) return;
    extra = needed;
    grid.style.paddingBottom = `${padding + extra}px`;
  };
  const observer = new ResizeObserver(update);
  observer.observe(grid);
  observer.observe(card);
  observer.observe(scroller);
  observer.observe(toolbar.firstElementChild ?? toolbar);
  update();
  return () => {
    observer.disconnect();
    grid.style.paddingBottom = original;
  };
}

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
  if (reduce) {
    // The first scroll compacts the filters. Allow their resize measurement
    // and the instantly revealed body to settle before the final alignment.
    let started: number | undefined;
    let frame: number;
    const cleanup = () => {
      for (const type of USER_SCROLL) scroller.removeEventListener(type, stop);
    };
    const stop = () => {
      cancelAnimationFrame(frame);
      cleanup();
    };
    const align = (now: number) => {
      started ??= now;
      scroller.scrollTo({ top: scrollTarget(card, scroller, toolbar) });
      if (now - started < (REVEAL_DELAY + REVEAL_DURATION) * 1000)
        frame = requestAnimationFrame(align);
      else cleanup();
    };
    frame = requestAnimationFrame(align);
    for (const type of USER_SCROLL)
      scroller.addEventListener(type, stop, { passive: true, once: true });
    return stop;
  }
  // Runs through the reveal: near the bottom of the page the target only
  // becomes reachable as the body grows, and scrollTop clamps until then.
  const from = scroller.scrollTop;
  const controls: AnimationPlaybackControls = animate(0, 1, {
    duration: REVEAL_DELAY + REVEAL_DURATION,
    ease: MOVE,
    onUpdate: (progress) => {
      const top = scrollTarget(card, scroller, toolbar);
      scroller.scrollTop = from + (top - from) * progress;
    },
  });
  const cleanup = () => {
    for (const type of USER_SCROLL) scroller.removeEventListener(type, stop);
  };
  const stop = () => {
    controls.stop();
    cleanup();
  };
  for (const type of USER_SCROLL)
    scroller.addEventListener(type, stop, { passive: true, once: true });
  void controls.finished.then(cleanup);
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
  const releaseScrollRoom = useRef<() => void>(() => undefined);

  useLayoutEffect(() => {
    stopScroll.current();
    releaseScrollRoom.current();
    releaseScrollRoom.current = () => undefined;
    const pending = pendingReveal.current;
    if (!pending || pending.id !== expanded) return;
    pendingReveal.current = null;
    if (!pending.card.isConnected) return;
    releaseScrollRoom.current = reserveScrollRoom(
      pending.card,
      toolbarRef?.current,
    );
    stopScroll.current = revealCard(pending.card, toolbarRef?.current, reduce);
  }, [expanded, reduce, toolbarRef]);
  useEffect(
    () => () => {
      stopScroll.current();
      releaseScrollRoom.current();
    },
    [],
  );

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
