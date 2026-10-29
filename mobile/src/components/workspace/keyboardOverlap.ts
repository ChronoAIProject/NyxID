export type KeyboardFrame = { screenY: number; height: number };

/** Only consume the part of this view that the visible keyboard actually covers. */
export function keyboardOverlap(
  frame: { y: number; height: number },
  keyboard?: KeyboardFrame,
): number {
  if (
    !keyboard ||
    !Number.isFinite(keyboard.screenY) ||
    !Number.isFinite(keyboard.height) ||
    keyboard.height <= 0 ||
    !Number.isFinite(frame.y) ||
    !Number.isFinite(frame.height) ||
    frame.height <= 0
  ) {
    return 0;
  }
  return Math.min(
    frame.height,
    Math.max(0, frame.y + frame.height - keyboard.screenY),
  );
}
