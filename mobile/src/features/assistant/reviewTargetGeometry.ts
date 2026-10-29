export type ReviewMeasureTarget = {
  measureInWindow: (
    callback: (x: number, y: number, width: number, height: number) => void,
  ) => void;
};

/** Native measurements complete asynchronously; recheck the exact review scope. */
export function revealAssistantReviewTarget({
  viewport,
  target,
  current,
  offset,
  scroll,
  focus,
  pause,
}: {
  viewport: ReviewMeasureTarget;
  target: ReviewMeasureTarget;
  current: () => boolean;
  offset: () => number;
  scroll: (offset: number) => void;
  focus: () => void;
  pause: () => void;
}) {
  if (!current()) return;
  pause();
  viewport.measureInWindow((_x, viewportY, _width, viewportHeight) => {
    if (!current() || viewportHeight <= 0) return;
    target.measureInWindow((_tx, targetY, _tw, targetHeight) => {
      if (!current() || targetHeight <= 0) return;
      const next =
        offset() +
        targetY -
        viewportY -
        Math.max(16, (viewportHeight - targetHeight) / 2);
      if (!Number.isFinite(next)) return;
      scroll(Math.max(0, next));
      if (current()) focus();
    });
  });
}
