import { animate } from "motion/react";
import { MOVE } from "@/hooks/use-card-sequence";

/** How far above the newest message the reader must be before a jump is offered. */
export const JUMP_TO_LATEST_DISTANCE = 200;

/** Input that means the reader took the scroll back. */
const USER_SCROLL = ["wheel", "touchstart", "keydown", "pointerdown"] as const;

export function prefersReducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Glides `scroller` to its end. The end is re-read every frame, so a reply
 * growing or the header collapsing on the way (which shifts `scrollTop` to
 * keep the reading position) cannot strand it midway. Stops when the reader
 * scrolls; `onArrive` runs only when it lands. Returns a stop function.
 */
export function glideToEnd(scroller: HTMLElement, onArrive: () => void): () => void {
  const from = scroller.scrollTop;
  const controls = animate(0, 1, {
    duration: 0.4,
    ease: MOVE,
    onUpdate: (progress) => {
      scroller.scrollTop = from + (scroller.scrollHeight - scroller.clientHeight - from) * progress;
    },
    onComplete: () => {
      cleanup();
      onArrive();
    },
  });
  const cleanup = () => {
    for (const type of USER_SCROLL) scroller.removeEventListener(type, stop);
  };
  const stop = () => {
    controls.stop();
    cleanup();
  };
  for (const type of USER_SCROLL) scroller.addEventListener(type, stop, { passive: true, once: true });
  return stop;
}
