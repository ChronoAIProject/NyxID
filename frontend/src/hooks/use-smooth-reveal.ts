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
  }, [text.length]);

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
        if (current >= target) return current;
        const backlog = target - current;
        if (backlog > SNAP_BACKLOG_CHARS) return target;
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
