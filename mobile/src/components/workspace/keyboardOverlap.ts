export type KeyboardFrame = { screenY: number; height: number };

export function visibleKeyboardFrame(
  frame: KeyboardFrame | undefined,
  windowHeight: number,
): KeyboardFrame | undefined {
  if (
    !frame ||
    !Number.isFinite(windowHeight) ||
    windowHeight <= 0 ||
    !Number.isFinite(frame.screenY) ||
    !Number.isFinite(frame.height) ||
    frame.height <= 0 ||
    frame.height > windowHeight ||
    frame.screenY < 0 ||
    frame.screenY >= windowHeight
  )
    return undefined;
  return {
    screenY: frame.screenY,
    height: Math.min(frame.height, windowHeight - frame.screenY),
  };
}

/** Intersect the view and visible keyboard in the same window coordinates. */
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
  return Math.max(
    0,
    Math.min(frame.y + frame.height, keyboard.screenY + keyboard.height) -
      Math.max(frame.y, keyboard.screenY),
  );
}
