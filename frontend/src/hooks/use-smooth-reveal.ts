import { useEffect, useRef, useState } from "react";
import { safeRevealEnd } from "@/lib/assistant/reveal-boundary";

/**
 * The backlog is drained over roughly this long, so the text on screen trails
 * what has arrived by a small and — crucially — CONSTANT amount. A fixed lag
 * reads as flow; a lag that varies with chunk size reads as stutter.
 */
const DRAIN_MS = 90;

/**
 * Floor, so the last few characters of a chunk still visibly move. The
 * proportional term decays towards zero as the backlog empties; without a floor
 * the final handful of characters would take longer than all the rest.
 */
const MIN_CHARS_PER_SECOND = 700;

/**
 * A jump this large is a load, not a stream — a re-mount mid-answer, a
 * reconnect, or a history projection landing at once. Typing it out would be a
 * lie about when it arrived, so it snaps.
 */
const SNAP_BACKLOG_CHARS = 1500;

/** A frame this long is a stall; crediting it in full would jerk the text. */
const MAX_FRAME_SECONDS = 0.05;

/** Weight of the newest observed producer gap in the cadence estimate. */
const GAP_EMA_ALPHA = 0.3;

/** Do not deliberately spread a chunk beyond this added-lag ceiling. */
const MAX_SPREAD_MS = 400;

/** Low adaptive floor; fractional credit keeps this time-derived. */
const MIN_ADAPTIVE_CHARS_PER_SECOND = 60;

/** Longer gaps are producer stalls or tool calls, not cadence samples. */
const STALL_GAP_MS = 1500;

/** Revert to the legacy drain after this many estimated gaps without input. */
const IDLE_GAPS = 2;

function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Decouple the rate text APPEARS from the rate it ARRIVES.
 *
 * Upstream deltas land in bursts at whatever cadence the network and the
 * projection scheduler produce; painting them the instant they land makes the
 * answer advance in visible steps. This paces the reveal against the frame
 * clock instead, chasing the arrived length with a proportional controller, so
 * the text flows continuously however lumpy its supply is.
 *
 * It only ever WITHHOLDS characters that have already arrived, and only while
 * `active`. The moment a block settles — or motion is suppressed, or there is
 * no frame clock to pace against — the full text is returned, so nothing can
 * strand content behind an animation.
 */
export function useSmoothReveal(text: string, active: boolean): string {
  const [revealed, setRevealed] = useState(() => text.length);
  // The frame loop must see the newest arrival without being torn down and
  // rebuilt on every delta — restarting it would drop the timestamp baseline
  // each time and the reveal would never accumulate.
  const targetRef = useRef(text.length);
  const arrivalTextRef = useRef(text);
  const lastArrivalRef = useRef<number | null>(null);
  const gapEmaRef = useRef<number | null>(null);
  const adaptiveCreditRef = useRef(0);
  const [displayed, setDisplayed] = useState(() => ({
    source: text,
    cut: text.length,
  }));

  const paced =
    active &&
    typeof requestAnimationFrame === "function" &&
    !prefersReducedMotion();

  useEffect(() => {
    targetRef.current = text.length;
    const previousText = arrivalTextRef.current;
    arrivalTextRef.current = text;

    if (!paced) {
      lastArrivalRef.current = null;
      gapEmaRef.current = null;
      adaptiveCreditRef.current = 0;
      return;
    }
    if (text === previousText) return;
    if (!text.startsWith(previousText)) {
      lastArrivalRef.current = null;
      gapEmaRef.current = null;
      adaptiveCreditRef.current = 0;
      return;
    }

    const now = performance.now();
    const lastArrival = lastArrivalRef.current;
    if (lastArrival !== null) {
      const gap = now - lastArrival;
      if (gap > STALL_GAP_MS) {
        gapEmaRef.current = null;
      } else if (gap > 0) {
        const previousGap = gapEmaRef.current;
        gapEmaRef.current =
          previousGap === null
            ? gap
            : previousGap + GAP_EMA_ALPHA * (gap - previousGap);
      }
    }
    lastArrivalRef.current = now;
  }, [paced, text]);

  // React's "adjust state when a prop changes" pattern. Crossing into paced
  // mode has to start from what is already on screen: a block that settles and
  // then resumes — an approval continuation appends to the one the reader is
  // already looking at — must stream only what is new, never re-type itself.
  const [pacedPreviously, setPacedPreviously] = useState(paced);
  if (pacedPreviously !== paced) {
    setPacedPreviously(paced);
    setRevealed(text.length);
  }

  useEffect(() => {
    if (!paced) return;
    let previous: number | undefined;
    let frame = 0;
    const step = (now: number) => {
      frame = requestAnimationFrame(step);
      const elapsed = previous === undefined ? 0 : (now - previous) / 1000;
      previous = now;
      if (elapsed <= 0) return;
      const seconds = Math.min(elapsed, MAX_FRAME_SECONDS);
      setRevealed((current) => {
        const target = targetRef.current;
        // Identity return: React bails out of the re-render, so a caught-up
        // stream costs one comparison per frame rather than a paint.
        if (current >= target) {
          adaptiveCreditRef.current = 0;
          return current;
        }
        const backlog = target - current;
        if (backlog > SNAP_BACKLOG_CHARS) {
          adaptiveCreditRef.current = 0;
          return target;
        }

        const gapEma = gapEmaRef.current;
        const spread = Math.min(
          MAX_SPREAD_MS,
          Math.max(DRAIN_MS, gapEma ?? DRAIN_MS),
        );
        const lastArrival = lastArrivalRef.current;
        const adaptive =
          gapEma !== null &&
          lastArrival !== null &&
          now - lastArrival <= IDLE_GAPS * spread;
        if (adaptive) {
          const rate = Math.max(
            MIN_ADAPTIVE_CHARS_PER_SECOND,
            backlog / (spread / 1000),
          );
          const credit = adaptiveCreditRef.current + rate * seconds;
          const advance = Math.floor(credit);
          adaptiveCreditRef.current = credit - advance;
          return advance === 0 ? current : Math.min(target, current + advance);
        }

        adaptiveCreditRef.current = 0;
        const rate = Math.max(MIN_CHARS_PER_SECOND, backlog / (DRAIN_MS / 1000));
        const advance = Math.max(1, Math.round(rate * seconds));
        return Math.min(target, current + advance);
      });
    };
    frame = requestAnimationFrame(step);
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [paced]);

  if (!paced) {
    if (displayed.source !== text || displayed.cut !== text.length) {
      setDisplayed({ source: text, cut: text.length });
    }
    return text;
  }

  const safeCut = safeRevealEnd(text, Math.min(revealed, text.length));
  const extendsPreviousText = text.startsWith(displayed.source);
  const displayedCut = extendsPreviousText
    ? Math.max(displayed.cut, safeCut)
    : safeCut;
  const clampedCut = Math.min(displayedCut, text.length);
  if (displayed.source !== text || displayed.cut !== clampedCut) {
    setDisplayed({ source: text, cut: clampedCut });
  }
  return text.slice(0, clampedCut);
}
